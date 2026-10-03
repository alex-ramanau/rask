//! Output: stdout buffering and the pager, and ack's `warn`, `die`, `say`
//! and friends (`App::Ack::warn`, `App::Ack::die`, `App::Ack::say`, ...).
//!
//! Like Perl's STDOUT, output is block-buffered into a pipe or file and
//! line-buffered on a terminal. Warnings go straight to stderr.

pub mod color;

use std::cell::RefCell;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::{Child, Command, Stdio};

/// Prefix for messages. Perl ack uses `basename($0)`. The test suite expects
/// `ack`, so that's what rask says, whatever the binary is called.
pub const PROGRAM: &str = "ack";

struct Out {
    writer: BufWriter<Box<dyn Write>>,
    line_buffered: bool,
    pager: Option<Child>,
}

thread_local! {
    static OUT: RefCell<Option<Out>> = const { RefCell::new(None) };
}

fn with_out(f: impl FnOnce(&mut Out) -> io::Result<()>) {
    OUT.with(|cell| {
        let mut cell = cell.borrow_mut();
        let out = cell.get_or_insert_with(|| {
            let line_buffered = io::stdout().is_terminal();
            Out {
                writer: BufWriter::with_capacity(64 * 1024, Box::new(io::stdout())),
                line_buffered,
                pager: None,
            }
        });
        if let Err(e) = f(out)
            && e.kind() == io::ErrorKind::BrokenPipe
        {
            // Perl would die of SIGPIPE; exit the way a shell would see that.
            std::process::exit(141);
        }
    });
}

/// `App::Ack::print`
pub fn print(parts: &[&[u8]]) {
    with_out(|out| {
        for p in parts {
            out.writer.write_all(p)?;
        }
        if out.line_buffered && parts.last().is_some_and(|p| p.ends_with(b"\n")) {
            out.writer.flush()?;
        }
        Ok(())
    });
}

/// Writes raw bytes to stdout.
pub fn write(bytes: &[u8]) {
    print(&[bytes]);
}

/// `App::Ack::warn`: `ack: msg` on stderr.
pub fn warn(msg: &str) {
    warn_bytes(msg.as_bytes());
}

pub fn warn_bytes(msg: &[u8]) {
    let line = [PROGRAM.as_bytes(), b": ", msg, b"\n"].concat();
    let captured = CAPTURE.with(|c| match c.borrow_mut().as_mut() {
        Some(buf) => {
            buf.push(line.clone());
            true
        }
        None => false,
    });
    if !captured {
        emit_warnings(&[line]);
    }
}

thread_local! {
    /// Warnings recorded instead of printed, while a worker thread handles a file.
    static CAPTURE: RefCell<Option<Vec<Vec<u8>>>> = const { RefCell::new(None) };
}

/// Runs `f`, returning the warnings it would have printed instead of
/// printing them, so they can be printed later in the right order.
pub fn capture_warnings<R>(f: impl FnOnce() -> R) -> (R, Vec<Vec<u8>>) {
    let previous = CAPTURE.with(|c| c.replace(Some(Vec::new())));
    let result = f();
    let warnings = CAPTURE.with(|c| c.replace(previous)).unwrap_or_default();
    (result, warnings)
}

/// Prints complete warning lines (as captured) to stderr.
pub fn emit_warnings(lines: &[Vec<u8>]) {
    let mut err = io::stderr().lock();
    for line in lines {
        let _ = err.write_all(line);
    }
}

/// The text `App::Ack::die` passes to `CORE::die`, for use where Perl ack
/// dies inside an eval (Getopt::Long callbacks).
pub fn die_text(msg: &str) -> String {
    format!("{PROGRAM}: {msg}\n")
}

/// `App::Ack::die`: print `ack: msg` and exit. Perl's `die` exits with 255
/// when `$!` and `$?` are both zero, which is the case for all of ack's
/// fatal errors.
pub fn die(msg: &str) -> ! {
    warn(msg);
    exit(255)
}

/// Flushes output, waits for the pager, and exits.
pub fn exit(code: i32) -> ! {
    OUT.with(|cell| {
        if let Some(mut out) = cell.borrow_mut().take() {
            let _ = out.writer.flush();
            // Closing the pipe lets the pager finish.
            drop(out.writer);
            if let Some(mut child) = out.pager.take() {
                let _ = child.wait();
            }
        }
    });
    std::process::exit(code)
}

/// Characters that make Perl run a piped `open` command through the shell.
const SHELL_METACHARS: &[u8] = b"$&*(){}[]'\";\\|?<>~`\n";

/// `App::Ack::set_up_pager`: send all output through `command`. Does nothing
/// when stdout isn't a terminal.
pub fn set_up_pager(command: &[u8]) {
    if !io::stdout().is_terminal() {
        return;
    }
    let cmd = crate::bytes::lossy(command);
    let mut builder = if command.iter().any(|c| SHELL_METACHARS.contains(c)) {
        let mut b = Command::new("/bin/sh");
        b.arg("-c").arg(&cmd);
        b
    } else {
        let mut words = cmd.split_ascii_whitespace();
        let Some(program) = words.next() else { return };
        let mut b = Command::new(program);
        b.args(words);
        b
    };
    let mut child = match builder.stdin(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => die(&format!(
            "Unable to pipe to pager \"{cmd}\": {}",
            crate::bytes::errno_text(&e)
        )),
    };
    let stdin = child.stdin.take().expect("piped stdin");
    OUT.with(|cell| {
        *cell.borrow_mut() = Some(Out {
            writer: BufWriter::with_capacity(64 * 1024, Box::new(stdin)),
            line_buffered: false,
            pager: Some(child),
        });
    });
}
