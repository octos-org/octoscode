//! Running another program with an end in sight: one place for "run it,
//! give it this much time, take what it printed".
//!
//! Two things every caller needs and each used to get slightly differently:
//!
//! - a deadline for the whole run, not only for connecting (an ssh that
//!   connected and then stalls is killed at the deadline);
//! - not waiting on the pipes after the program has ended. A grandchild
//!   that inherited stdout or stderr keeps them open; what was printed up
//!   to a short grace after the exit is what the run printed.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the pipes may stay open after the program itself has ended.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// A whole request to the other machine: connect, send, be answered.
const SSH_SECONDS: u64 = 60;

pub struct Ran {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Ran {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

#[derive(Debug)]
pub enum RunError {
    Spawn(std::io::Error),
    Timeout,
}

/// The time one ssh request may take in all (`OLP_SSH_TIMEOUT` seconds,
/// default 60).
pub fn ssh_limit() -> Duration {
    let seconds = std::env::var("OLP_SSH_TIMEOUT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(SSH_SECONDS);
    Duration::from_secs(seconds)
}

struct Pipe {
    bytes: Arc<Mutex<Vec<u8>>>,
    ended: mpsc::Receiver<()>,
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Pipe {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, ended) = mpsc::channel();
    let sink = Arc::clone(&bytes);
    std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut held) = sink.lock() {
                            held.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
            }
        }
        let _ = tx.send(());
    });
    Pipe { bytes, ended }
}

impl Pipe {
    /// What arrived, waiting until `until` at most for the pipe to close.
    fn take(self, until: Instant) -> Vec<u8> {
        let left = until.saturating_duration_since(Instant::now());
        let _ = self.ended.recv_timeout(left);
        self.bytes
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }
}

/// Run `cmd` to its end, or kill it at the deadline. `stdin` is written
/// from its own thread, so a program that never reads does not block the
/// caller; `None` gives it no stdin at all.
pub fn run(mut cmd: Command, stdin: Option<&[u8]>, limit: Duration) -> Result<Ran, RunError> {
    let mut child = cmd
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    if let (Some(mut pipe), Some(input)) = (child.stdin.take(), stdin) {
        let input = input.to_vec();
        // A request refused before stdin is read closes the pipe first.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
        }
    };
    let grace = Instant::now() + PIPE_GRACE;
    Ok(Ran {
        code: status.code(),
        stdout: out.take(grace),
        stderr: err.take(grace),
    })
}

/// One word for a POSIX shell line, quoted only when it has to be (the
/// form Python's `shlex.quote` gives, which the scripts printed).
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_program_that_outlives_the_limit_is_killed() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let started = Instant::now();
        let ran = run(cmd, None, Duration::from_millis(200));
        assert!(matches!(ran, Err(RunError::Timeout)));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_the_pipes_does_not_hold_the_caller() {
        // The shell ends at once; the sleep it left behind keeps stdout
        // and stderr open for half a minute.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo said; sleep 30 & exit 0"]);
        let started = Instant::now();
        let ran = run(cmd, None, Duration::from_secs(20)).expect("the shell ran");
        assert_eq!(ran.code, Some(0));
        assert_eq!(ran.stdout_text().trim(), "said");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[test]
    fn stdin_reaches_the_program_and_an_unread_stdin_does_not_block() {
        let mut cat = Command::new("cat");
        cat.arg("-");
        let ran = run(cat, Some(b"hello"), Duration::from_secs(10)).expect("cat ran");
        assert_eq!(ran.stdout, b"hello");
        let mut deaf = Command::new("sh");
        deaf.args(["-c", "exit 3"]);
        let big = vec![b'x'; 1 << 20];
        let ran = run(deaf, Some(&big), Duration::from_secs(10)).expect("sh ran");
        assert_eq!(ran.code, Some(3));
    }

    #[test]
    fn quoting_leaves_plain_words_alone() {
        assert_eq!(quote("a/b-c.d"), "a/b-c.d");
        assert_eq!(quote("two words"), "'two words'");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
        assert_eq!(quote(""), "''");
    }
}
