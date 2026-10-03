//! rask: a Rust reimplementation of ack 3.
//!
//! Perl ack 3.10.0 is the specification: its test suite is run against the
//! binary by `scripts/ack-suite`. The modules follow Perl ack's structure;
//! `app` is its `MAIN` block.

pub mod app;
pub mod bytes;
pub mod cli;
pub mod config;
pub mod env;
pub mod filetest;
pub mod filter;
pub mod help;
pub mod matcher;
pub mod output;
pub mod parallel;
pub mod search;
pub mod walk;

/// Version of Perl ack whose behaviour we reproduce.
pub const ACK_VERSION: &str = "3.10.0";
