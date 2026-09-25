//! Ordered parallel search (plan item 5.3).
//!
//! The walk, the per-file work and the printing run on different threads:
//!
//! - one thread walks the directories, in File::Next order, and numbers each
//!   entry (and each warning the walk produces, such as an unreadable
//!   directory);
//! - worker threads run the file filter and `Settings::prepare` (opening,
//!   the `-T` test, the prescan, counting), capturing their warnings;
//! - the caller gets the results back in walk order, with each file's
//!   warnings printed just before it is handed over.
//!
//! Everything that depends on what was printed before (blank lines between
//! files, `--` separators, `-1`, `-m`) stays with the caller, in the same
//! single-threaded code as the serial search, so the output is the same.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::filter::File;
use crate::output;
use crate::search::{Mode, Prepared, Settings};
use crate::walk::{Entry, Files};

/// Decides which walked files are searched: the file filter, with the
/// state it keeps between calls. Each worker gets its own clone.
pub trait Select: Clone + Send + 'static {
    fn select(&mut self, entry: &Entry) -> Option<File>;
}

/// How far the walk may get ahead of the printing, in entries. This bounds
/// the memory held by results waiting to be printed.
const WINDOW: usize = 256;

enum Job {
    Entry(Entry),
    /// Warnings from the walk itself, printed in walk order.
    Warnings(Vec<Vec<u8>>),
}

struct Done {
    warnings: Vec<Vec<u8>>,
    prepared: Option<Prepared>,
}

/// The number of worker threads: the CPU count, or `RASK_THREADS` (for
/// testing and benchmarks; `RASK_THREADS=1` gives the serial search).
pub fn threads() -> usize {
    if let Some(n) = std::env::var("RASK_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return n.max(1);
    }
    thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(16)
}

/// Starts the threads. `make_walker` runs on the walker thread, so the walk
/// doesn't need to be `Send`.
pub fn run<S: Select>(
    make_walker: impl FnOnce() -> Files<'static> + Send + 'static,
    selector: S,
    settings: Arc<Settings>,
    mode: Mode,
    threads: usize,
) -> Ordered {
    let (job_tx, job_rx) = sync_channel::<(u64, Job)>(WINDOW);
    let job_rx = Arc::new(Mutex::new(job_rx));
    let (done_tx, done_rx) = channel::<(u64, Done)>();
    // One token per entry in flight; the consumer gives each one back.
    let (token_tx, token_rx) = sync_channel::<()>(WINDOW);
    for _ in 0..WINDOW {
        token_tx.send(()).expect("token channel");
    }

    thread::spawn(move || walk(make_walker, job_tx, token_rx));
    for _ in 0..threads {
        let job_rx = job_rx.clone();
        let done_tx = done_tx.clone();
        let mut selector = selector.clone();
        let settings = settings.clone();
        thread::spawn(move || work(&job_rx, &done_tx, &mut selector, &settings, mode));
    }

    Ordered {
        results: done_rx,
        pending: HashMap::new(),
        next: 0,
        tokens: token_tx,
    }
}

fn walk(
    make_walker: impl FnOnce() -> Files<'static>,
    jobs: SyncSender<(u64, Job)>,
    tokens: Receiver<()>,
) {
    let mut walker = make_walker();
    let mut seq = 0u64;
    let mut send = |job: Job| -> bool {
        if tokens.recv().is_err() {
            return false;
        }
        let ok = jobs.send((seq, job)).is_ok();
        seq += 1;
        ok
    };
    loop {
        let (entry, warnings) = output::capture_warnings(|| walker.next_entry());
        if !warnings.is_empty() && !send(Job::Warnings(warnings)) {
            return;
        }
        match entry {
            Some(entry) => {
                if !send(Job::Entry(entry)) {
                    return;
                }
            }
            None => return,
        }
    }
}

fn work<S: Select>(
    jobs: &Mutex<Receiver<(u64, Job)>>,
    done: &Sender<(u64, Done)>,
    selector: &mut S,
    settings: &Settings,
    mode: Mode,
) {
    loop {
        let job = jobs.lock().expect("job queue").recv();
        let Ok((seq, job)) = job else { return };
        let result = match job {
            Job::Warnings(warnings) => Done {
                warnings,
                prepared: None,
            },
            Job::Entry(entry) => {
                let (prepared, warnings) = output::capture_warnings(|| {
                    let file = selector.select(&entry)?;
                    Some(settings.prepare(file.regular(entry.is_file), mode))
                });
                Done { warnings, prepared }
            }
        };
        if done.send((seq, result)).is_err() {
            return;
        }
    }
}

/// The prepared files, in walk order.
pub struct Ordered {
    results: Receiver<(u64, Done)>,
    pending: HashMap<u64, Done>,
    next: u64,
    tokens: SyncSender<()>,
}

impl Iterator for Ordered {
    type Item = Prepared;

    fn next(&mut self) -> Option<Prepared> {
        loop {
            if let Some(done) = self.pending.remove(&self.next) {
                self.next += 1;
                let _ = self.tokens.send(());
                output::emit_warnings(&done.warnings);
                if let Some(prepared) = done.prepared {
                    return Some(prepared);
                }
                continue;
            }
            match self.results.recv() {
                Ok((seq, done)) => {
                    self.pending.insert(seq, done);
                }
                // The walk and all the workers have finished.
                Err(_) => return None,
            }
        }
    }
}
