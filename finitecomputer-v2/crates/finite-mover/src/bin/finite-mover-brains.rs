//! `finite-mover-brains <finite-brain database>`: every brain an owner
//! holds in finite-brain's SQLite, as one line of JSON
//! (`finite_mover::brains::ReaderOutput`), for the mover (FIN-499).
//!
//! lat-2's module runs it only through a fixed wrapper, by sudo, as
//! finite-brain's own dynamic user: it refuses a database another user
//! owns, opens it read-only and changes nothing.

use std::path::PathBuf;
use std::process::ExitCode;

use finite_mover::brains::read_database;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("finite-mover-brains <finite-brain database>");
        return ExitCode::from(64);
    };
    match read_database(&PathBuf::from(path)) {
        Ok(output) => {
            println!(
                "{}",
                serde_json::to_string(&output).expect("owned brains serialize")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("finite-mover-brains: {error}");
            ExitCode::FAILURE
        }
    }
}
