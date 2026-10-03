//! Printing matches, filenames and errors (Phases 1 and 4).

use std::process::ExitCode;

/// Prefix for error messages. Perl ack writes `ack: ...`, and the test suite
/// matches on it, so we keep that prefix whatever the binary is called.
const PROGRAM: &str = "ack";

/// Print a fatal error the way `App::Ack::die` does, and return exit code 2.
pub fn die(msg: &str) -> ExitCode {
    eprintln!("{PROGRAM}: {msg}");
    ExitCode::from(2)
}
