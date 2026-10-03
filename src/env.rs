//! Environment variables, honouring `--noenv`, which makes Perl ack delete
//! `ACKRC` and every `ACK_*` variable before doing anything else.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::bytes::{Bytes, from_os};

static IGNORE_ACK_VARIABLES: AtomicBool = AtomicBool::new(false);

pub fn ignore_ack_variables() {
    IGNORE_ACK_VARIABLES.store(true, Ordering::Relaxed);
}

pub fn var(name: &str) -> Option<Bytes> {
    if IGNORE_ACK_VARIABLES.load(Ordering::Relaxed) && (name == "ACKRC" || name.starts_with("ACK_"))
    {
        return None;
    }
    std::env::var_os(name).map(|v| from_os(&v))
}
