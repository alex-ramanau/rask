//! rask: a Rust reimplementation of ack 3.
//!
//! Perl ack 3.10.0 is the specification: its test suite is run against this
//! binary by `scripts/ack-suite`.

mod cli;
mod config;
mod filter;
mod help;
mod matcher;
mod output;
mod search;
mod walk;

use std::process::ExitCode;

/// Version of Perl ack whose behaviour we reproduce.
pub const ACK_VERSION: &str = "3.10.0";

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    if args.iter().any(|a| a == "--version") {
        println!("{}", help::version_text());
        return ExitCode::SUCCESS;
    }

    output::die("searching is not implemented yet (rask Phase 0)")
}
