//! Environment variables, honouring `--noenv`, which makes Perl ack delete
//! `ACKRC` and every `ACK_*` variable before doing anything else.
//!
//! Tests can override variables for the current thread with
//! [`with_overrides`]; changing the real environment would need `unsafe`
//! and would race between tests.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bytes::{Bytes, from_os};

static IGNORE_ACK_VARIABLES: AtomicBool = AtomicBool::new(false);

thread_local! {
    static OVERRIDES: RefCell<Option<HashMap<String, Option<Bytes>>>> = const { RefCell::new(None) };
}

pub fn ignore_ack_variables() {
    IGNORE_ACK_VARIABLES.store(true, Ordering::Relaxed);
}

pub fn var(name: &str) -> Option<Bytes> {
    if IGNORE_ACK_VARIABLES.load(Ordering::Relaxed) && (name == "ACKRC" || name.starts_with("ACK_"))
    {
        return None;
    }
    if let Some(value) = OVERRIDES.with(|o| o.borrow().as_ref().and_then(|m| m.get(name).cloned()))
    {
        return value;
    }
    std::env::var_os(name).map(|v| from_os(&v))
}

/// Runs `f` with variables set (`Some`) or unset (`None`) for this thread.
/// Variables not listed keep their real values.
pub fn with_overrides<R>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> R) -> R {
    let map = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.map(|v| v.as_bytes().to_vec())))
        .collect();
    let previous = OVERRIDES.with(|o| o.replace(Some(map)));
    let result = f();
    OVERRIDES.with(|o| *o.borrow_mut() = previous);
    result
}
