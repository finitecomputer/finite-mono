//! `finite-mover --config <file> <npub | roster | mailbox>`
//!
//! - `npub`: the mover key's npub, for the deployment's `migrations.mover`.
//! - `roster`: push the roster once (a timer runs it every 10 minutes).
//! - `mailbox`: read the mailbox and do what it says (every 10 seconds).
//!   One run at a time: a second, meanwhile, does nothing.
//!
//! It prints one JSON line per result, never a secret or a person's data.

use std::fs::File;
use std::path::PathBuf;
use std::process::ExitCode;

use finite_mover::config::{Config, FleetConfig, npub};
use finite_mover::fleet::{FileFleet, Fleet};
use finite_mover::migrations::MigrationsClient;
use finite_mover::mover::Mover;

const USAGE: &str = "finite-mover --config <file> <npub | roster | mailbox>";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("finite-mover: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--config") {
        return Err(USAGE.into());
    }
    let path = PathBuf::from(args.next().ok_or(USAGE)?);
    let operation = args.next().ok_or(USAGE)?;
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    let config = Config::read(&path).map_err(|error| error.to_string())?;
    let key = config.key().map_err(|error| error.to_string())?;
    if operation == "npub" {
        println!("{}", npub(&key).ok_or("the mover's key is not a key")?);
        return Ok(());
    }
    let platform = MigrationsClient::new(config.finite_chat.clone(), key)
        .map_err(|error| error.to_string())?;
    let fleet: Box<dyn Fleet> = match &config.fleet {
        FleetConfig::File(file) => Box::new(FileFleet::new(file)),
    };
    let core = config.core();
    let hosts = config.hosts();
    let mover = Mover::new(&platform, fleet.as_ref(), &core, &hosts, &config.locks);
    match operation.as_str() {
        "roster" => {
            let taken = mover.push_roster().map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string(&taken).expect("an answer serializes")
            );
        }
        "mailbox" => {
            let _held = match lock(&config) {
                Some(held) => held,
                None => return Ok(()),
            };
            let worked = mover.work_mailbox().map_err(|error| error.to_string())?;
            for one in worked {
                println!(
                    "{}",
                    serde_json::to_string(&one).expect("a result serializes")
                );
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

/// The mailbox's run lock, or `None` while another run holds it.
fn lock(config: &Config) -> Option<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config.locks.join("mailbox.lock"))
        .ok()?;
    file.try_lock().ok().map(|()| file)
}
