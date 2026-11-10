//! `finite-mover --config <file> <npub | roster | mailbox | sizes>`
//!
//! - `npub`: the mover key's npub, for the deployment's `migrations.mover`.
//! - `roster`: push the roster once (a timer runs it every 10 minutes).
//! - `mailbox`: read the mailbox and do what it says (every 10 seconds).
//!   One run at a time: a second, meanwhile, does nothing.
//! - `sizes`: measure every live agent through its host's reader, from
//!   metadata alone, into the cache the roster reads (once a day). Only
//!   with Core's database as the fleet.
//!
//! It prints one JSON line per result, never a secret or a person's data.

use std::fs::File;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use finite_mover::config::{Config, FleetConfig, npub};
use finite_mover::fleet::{FileFleet, Fleet};
use finite_mover::fleet_pg::PgFleet;
use finite_mover::migrations::MigrationsClient;
use finite_mover::mover::Mover;
use finite_mover::sizes::{SizesCache, measure};

const USAGE: &str = "finite-mover --config <file> <npub | roster | mailbox | sizes>";

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
    if operation == "sizes" {
        return sizes(&config);
    }
    let platform = MigrationsClient::new(config.finite_chat.clone(), key)
        .map_err(|error| error.to_string())?;
    let fleet: Box<dyn Fleet> = match &config.fleet {
        FleetConfig::File(file) => Box::new(FileFleet::new(file)),
        FleetConfig::Postgres {
            conninfo,
            sizes_file,
        } => Box::new(PgFleet::new(conninfo, sizes_file)),
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
            let _held = match lock(&config, "mailbox.lock") {
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

/// Measure the live agents into the sizes cache, one run at a time.
fn sizes(config: &Config) -> Result<(), String> {
    let FleetConfig::Postgres {
        conninfo,
        sizes_file,
    } = &config.fleet
    else {
        return Err("sizes reads Core's database: the fleet must be postgres".into());
    };
    let Some(_held) = lock(config, "sizes.lock") else {
        return Ok(());
    };
    let live = PgFleet::new(conninfo, sizes_file)
        .live()
        .map_err(|error| error.to_string())?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64);
    let last = SizesCache::read(sizes_file);
    let (fresh, failed) = measure(&live, &config.hosts(), &last, now_ms);
    fresh
        .write(sizes_file)
        .map_err(|error| format!("{}: {error}", sizes_file.display()))?;
    for (old, why) in &failed {
        println!("{}", serde_json::json!({ "old": old, "unmeasured": why }));
    }
    println!(
        "{}",
        serde_json::json!({ "measured": live.len() - failed.len(), "failed": failed.len() })
    );
    Ok(())
}

/// A run lock in the locks folder, or `None` while another run holds it.
fn lock(config: &Config, name: &str) -> Option<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config.locks.join(name))
        .ok()?;
    file.try_lock().ok().map(|()| file)
}
