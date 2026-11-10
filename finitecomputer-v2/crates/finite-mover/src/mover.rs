//! The mover's work (FIN-463 M3): push the roster, read the mailbox, and
//! do what each command says, reporting each step. It acts only when told:
//! nothing here starts a stop, a stage or a retirement of its own.
//!
//! | Command | Steps | Reports |
//! |---|---|---|
//! | `stop_and_stage` | Core stops the runtime; the host reader streams it; the stage goes up in parts through finite.chat and is completed | `stopped`, `progress`, `staged`; or `failed` |
//! | `retire` | Core retires it (its verified backup), unless it did already; Core marks it moved | `retired`; or `failed` |
//! | `restart` | Core starts it again | `restarted`; or `failed` |
//! | another kind | nothing: left in the mailbox | none |
//!
//! Each step is idempotent: a command read again (its report lost, the
//! mover restarted) is done again, and `seq` keeps growing, as fragment
//! takes only a report whose `seq` is past the last it took.

use std::cell::Cell;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::contract::{Command, CommandKind, Report, Reported, RosterTaken, short};
use crate::core_cli::CoreCli;
use crate::fleet::{Binding, Fleet, FleetError};
use crate::hosts::HostCommands;
use crate::migrations::MigrationsClient;
use crate::sink::SinkError;
use crate::stage::{PART_BYTES, StageError, StageTarget, upload_stage_with_progress};

/// How often a stage in progress says so (fragment holds one silent for 30
/// minutes).
pub const PROGRESS_EVERY: Duration = Duration::from_secs(60);

/// A report's `seq`: milliseconds since the epoch, and always past the
/// last one this process gave. A mover that restarts mid-command keeps
/// growing it, with no state of its own.
#[derive(Debug, Default)]
pub struct SeqClock {
    last: Cell<u64>,
}

impl SeqClock {
    pub fn next(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        let next = now.max(self.last.get() + 1);
        self.last.set(next);
        next
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MoverError {
    #[error(transparent)]
    Platform(#[from] SinkError),
    #[error(transparent)]
    Fleet(#[from] FleetError),
}

/// What one command came to, for the run's log: no secret, no person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Worked {
    pub command: String,
    /// `staged`, `retired`, `restarted`, `failed`, `stale` (not this old
    /// agent's command any more), or `left` (a kind the mover doesn't know).
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Worked {
    fn of(command: &Command, outcome: &str) -> Self {
        Self {
            command: command.id.clone(),
            outcome: outcome.into(),
            step: None,
            why: None,
        }
    }
}

/// A step's end, short of done.
enum Stop {
    /// Report `failed` with this step and why.
    Failed(&'static str, String),
    /// The command is no longer this old agent's (409, 403): stop quietly.
    Stale,
    /// finite.chat or the fleet couldn't be asked: the command stays in
    /// the mailbox and is read again.
    Platform(MoverError),
}

impl From<SinkError> for Stop {
    fn from(error: SinkError) -> Self {
        match error {
            SinkError::NotCommanded | SinkError::NotStaging => Stop::Stale,
            other => Stop::Platform(other.into()),
        }
    }
}

pub struct Mover<'a> {
    pub platform: &'a MigrationsClient,
    pub fleet: &'a dyn Fleet,
    pub core: &'a CoreCli,
    pub hosts: &'a HostCommands,
    /// The per-agent locks' directory (the uploader's).
    pub locks: &'a Path,
    pub part_bytes: usize,
    /// How often a stage in progress reports (`PROGRESS_EVERY`).
    pub progress_every: Duration,
    pub seq: SeqClock,
}

impl<'a> Mover<'a> {
    pub fn new(
        platform: &'a MigrationsClient,
        fleet: &'a dyn Fleet,
        core: &'a CoreCli,
        hosts: &'a HostCommands,
        locks: &'a Path,
    ) -> Self {
        Self {
            platform,
            fleet,
            core,
            hosts,
            locks,
            part_bytes: PART_BYTES,
            progress_every: PROGRESS_EVERY,
            seq: SeqClock::default(),
        }
    }

    /// Push the whole roster, as Core has it now.
    pub fn push_roster(&self) -> Result<RosterTaken, MoverError> {
        let roster = self.fleet.roster()?;
        Ok(self.platform.push_roster(&roster)?)
    }

    /// Read the mailbox and do each command in it, oldest first.
    pub fn work_mailbox(&self) -> Result<Vec<Worked>, MoverError> {
        let mailbox = self.platform.mailbox()?;
        let mut worked = Vec::new();
        for command in mailbox.commands {
            worked.push(self.work(&command)?);
        }
        Ok(worked)
    }

    fn work(&self, command: &Command) -> Result<Worked, MoverError> {
        let done = match command.kind {
            CommandKind::StopAndStage => self.stop_and_stage(command),
            CommandKind::Retire => self.retire(command),
            CommandKind::Restart => self.restart(command),
            // Open on the wire: done by no one, and held at its deadline.
            CommandKind::Other => return Ok(Worked::of(command, "left")),
        };
        match done {
            Ok(outcome) => Ok(Worked::of(command, outcome)),
            Err(Stop::Stale) => Ok(Worked::of(command, "stale")),
            Err(Stop::Platform(error)) => Err(error),
            Err(Stop::Failed(step, why)) => {
                let why = short(&why);
                let report = Report {
                    step: Some(step.to_string()),
                    why: Some(why.clone()),
                    ..Report::of(&command.id, 0, "failed")
                };
                match self.report(command, report) {
                    Ok(_) | Err(Stop::Stale) => {}
                    Err(Stop::Platform(error)) => return Err(error),
                    Err(Stop::Failed(..)) => unreachable!("a report fails as the platform's"),
                }
                Ok(Worked {
                    step: Some(step.to_string()),
                    why: Some(why),
                    ..Worked::of(command, "failed")
                })
            }
        }
    }

    fn report(&self, command: &Command, report: Report) -> Result<Reported, Stop> {
        let report = Report {
            seq: self.seq.next(),
            ..report
        };
        Ok(self.platform.report(&command.old, &report)?)
    }

    fn binding(&self, command: &Command) -> Result<Binding, Stop> {
        match self.fleet.binding(&command.old) {
            Ok(Some(binding)) => Ok(binding),
            Ok(None) => Err(Stop::Failed(
                "binding",
                "Core has no runtime for this old agent".into(),
            )),
            Err(error) => Err(Stop::Platform(error.into())),
        }
    }

    fn stop_and_stage(&self, command: &Command) -> Result<&'static str, Stop> {
        let (Some(phase), Some(stage)) = (command.phase, command.stage.as_deref()) else {
            return Err(Stop::Failed(
                "command",
                "a stop_and_stage needs its phase and stage".into(),
            ));
        };
        // The stage must be this command's own: <old>.<phase>.<attempt>.
        let target = StageTarget::new(phase, &command.old, command.attempt)
            .map_err(|error| Stop::Failed("command", error.to_string()))?;
        if target.stage_id() != stage {
            return Err(Stop::Failed(
                "command",
                format!("stage {stage} is not this command's"),
            ));
        }
        let binding = self.binding(command)?;
        self.core
            .stop(&binding)
            .map_err(|error| Stop::Failed("stop", error.to_string()))?;
        self.report(command, Report::of(&command.id, 0, "stopped"))?;
        let stream = self
            .hosts
            .stream(&binding)
            .map_err(|error| Stop::Failed("stream", error.to_string()))?;
        let mut last = Instant::now();
        let mut progress = |bytes: u64, files: u64| {
            if last.elapsed() < self.progress_every {
                return;
            }
            last = Instant::now();
            // Best effort: a lost progress report only nears the deadline.
            let _ = self.report(
                command,
                Report {
                    bytes: Some(bytes),
                    files: Some(files),
                    ..Report::of(&command.id, 0, "progress")
                },
            );
        };
        let (manifest, completed) = upload_stage_with_progress(
            self.platform,
            &target,
            stream,
            self.part_bytes,
            self.locks,
            &mut progress,
        )
        .map_err(|error| match error {
            StageError::Sink(sink) => Stop::from(sink),
            other => Stop::Failed("stage", other.to_string()),
        })?;
        self.report(
            command,
            Report {
                manifest_sha256: Some(completed.manifest_sha256),
                bytes: Some(manifest.bytes),
                files: Some(manifest.files),
                ..Report::of(&command.id, 0, "staged")
            },
        )?;
        Ok("staged")
    }

    fn retire(&self, command: &Command) -> Result<&'static str, Stop> {
        let Some(mark) = command.mark else {
            return Err(Stop::Failed(
                "command",
                "a retire needs its mark (as)".into(),
            ));
        };
        let binding = self.binding(command)?;
        let receipt = match &binding.retired_receipt {
            Some(receipt) => receipt.clone(),
            None => self
                .core
                .retire(&binding)
                .map_err(|error| Stop::Failed("retire", error.to_string()))?,
        };
        self.core
            .mark_moved(&binding, mark)
            .map_err(|error| Stop::Failed("mark", error.to_string()))?;
        self.report(
            command,
            Report {
                receipt: Some(receipt),
                ..Report::of(&command.id, 0, "retired")
            },
        )?;
        Ok("retired")
    }

    fn restart(&self, command: &Command) -> Result<&'static str, Stop> {
        let binding = self.binding(command)?;
        self.core
            .restart(&binding)
            .map_err(|error| Stop::Failed("restart", error.to_string()))?;
        self.report(command, Report::of(&command.id, 0, "restarted"))?;
        Ok("restarted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_only_grows_even_across_movers() {
        let first = SeqClock::default();
        let seqs: Vec<u64> = (0..1000).map(|_| first.next()).collect();
        assert!(seqs.windows(2).all(|pair| pair[1] > pair[0]));
        // A mover started again begins at the clock, past every seq of the
        // last one that didn't run ahead of the clock. One that did (more
        // reports than milliseconds) has its next report answered
        // taken: false, and its command, still in the mailbox, done again.
        let last = *seqs.last().unwrap();
        while SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            <= last
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(SeqClock::default().next() > last);
    }
}
