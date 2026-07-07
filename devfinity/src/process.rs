use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub log_path: PathBuf,
}

impl ProcessSpec {
    pub fn new(
        name: impl Into<String>,
        program: impl Into<String>,
        cwd: impl Into<PathBuf>,
        log_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
            log_path: log_path.into(),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn env<I, K, V>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env = env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self
    }

    fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(crate::topology::shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone)]
pub struct ProcessStatus {
    pub name: String,
    pub pid: Option<u32>,
    pub running: bool,
    pub pid_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ProcessManager {
    control_dir: PathBuf,
    logs_dir: PathBuf,
}

impl ProcessManager {
    pub fn new(control_dir: impl Into<PathBuf>, logs_dir: impl Into<PathBuf>) -> Self {
        Self {
            control_dir: control_dir.into(),
            logs_dir: logs_dir.into(),
        }
    }

    pub fn spawn(&self, spec: ProcessSpec) -> Result<ProcessHandle> {
        fs::create_dir_all(&self.control_dir).with_context(|| {
            format!(
                "failed to create process control dir {}",
                self.control_dir.display()
            )
        })?;
        if let Some(parent) = spec.log_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut log = open_log(&spec.log_path)?;
        writeln!(log, "$ {}", spec.command_line())
            .with_context(|| format!("failed to write {}", spec.log_path.display()))?;
        log.flush()
            .with_context(|| format!("failed to flush {}", spec.log_path.display()))?;

        let stdout = Stdio::from(
            log.try_clone()
                .with_context(|| format!("failed to clone {}", spec.log_path.display()))?,
        );
        let stderr = Stdio::from(log);
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .stdout(stdout)
            .stderr(stderr);

        let child = command
            .spawn()
            .with_context(|| format!("failed to start {}", spec.name))?;
        let pid_file = self.pid_file(&spec.name);
        fs::write(&pid_file, child.id().to_string())
            .with_context(|| format!("failed to write {}", pid_file.display()))?;

        Ok(ProcessHandle {
            name: spec.name,
            child,
            log_path: spec.log_path,
            pid_file,
            shutdown_complete: false,
        })
    }

    pub fn run_command(&self, name: &str, command: &mut Command) -> Result<ExitStatus> {
        fs::create_dir_all(&self.logs_dir)
            .with_context(|| format!("failed to create {}", self.logs_dir.display()))?;
        let log_path = self.logs_dir.join(format!("{name}.log"));
        let mut log = open_log(&log_path)?;
        writeln!(log, "$ {:?}", command)
            .with_context(|| format!("failed to write {}", log_path.display()))?;
        log.flush()
            .with_context(|| format!("failed to flush {}", log_path.display()))?;
        let stdout = Stdio::from(
            log.try_clone()
                .with_context(|| format!("failed to clone {}", log_path.display()))?,
        );
        let stderr = Stdio::from(log);
        command.stdout(stdout).stderr(stderr);
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to run {name}; see {}", log_path.display()))?;
        println!("running {name} pid={}", child.id());
        child
            .wait()
            .with_context(|| format!("failed to wait for {name}; see {}", log_path.display()))
    }

    pub fn cleanup_stale_processes(&self) -> Result<()> {
        if !self.control_dir.exists() {
            println!("no devfinity process control directory found");
            return Ok(());
        }

        for entry in fs::read_dir(&self.control_dir)
            .with_context(|| format!("failed to read {}", self.control_dir.display()))?
        {
            let entry = entry.with_context(|| {
                format!("failed to read entry under {}", self.control_dir.display())
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("pid") {
                continue;
            }
            let Some(pid) = read_pid(&path) else {
                remove_file_best_effort(&path);
                continue;
            };
            if process_alive(pid) {
                println!("stopping devfinity process {pid} ({})", path.display());
                terminate_pid(pid);
                wait_for_pid_exit(pid, Duration::from_secs(10));
                if process_alive(pid) {
                    kill_pid(pid);
                    wait_for_pid_exit(pid, Duration::from_secs(2));
                }
            }
            remove_file_best_effort(&path);
        }

        println!("devfinity cleanup complete");
        Ok(())
    }

    pub fn statuses(&self, names: &[&str]) -> Vec<ProcessStatus> {
        names
            .iter()
            .map(|name| {
                let pid_file = self.pid_file(name);
                let pid = read_pid(&pid_file);
                let running = pid.is_some_and(process_alive);
                ProcessStatus {
                    name: (*name).to_string(),
                    pid,
                    running,
                    pid_file,
                }
            })
            .collect()
    }

    fn pid_file(&self, name: &str) -> PathBuf {
        self.control_dir
            .join(format!("{}.pid", sanitize_name(name)))
    }
}

#[derive(Debug)]
pub struct ProcessHandle {
    name: String,
    child: Child,
    log_path: PathBuf,
    pid_file: PathBuf,
    shutdown_complete: bool,
}

impl ProcessHandle {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn try_exit_status(&mut self) -> Result<Option<ExitStatus>> {
        self.child
            .try_wait()
            .with_context(|| format!("failed to poll {}", self.name))
    }

    pub fn shutdown(&mut self) -> Result<()> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.shutdown_complete = true;

        if self.try_exit_status()?.is_none() {
            terminate_pid(self.child.id());
            if wait_child_exit(&mut self.child, Duration::from_secs(10))?.is_none() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
        remove_file_best_effort(&self.pid_file);
        Ok(())
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.shutdown_complete {
            if let Err(error) = self.shutdown() {
                eprintln!("failed to stop {}: {error:#}", self.name);
            }
        }
    }
}

pub fn wait_child_exit(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to wait for child process")?
        {
            return Ok(Some(status));
        }
        if started.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn open_log(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn terminate_pid(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn remove_file_best_effort(path: &Path) {
    if path.exists() {
        if let Err(error) = fs::remove_file(path) {
            eprintln!("failed to remove {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_spec_quotes_command_line() {
        let spec =
            ProcessSpec::new("example", "echo", ".", "example.log").args(["hello world", "a'b"]);

        assert_eq!(spec.command_line(), "'echo' 'hello world' 'a'\"'\"'b'");
    }
}
