//! Bounded child-process probes for `doctor` / `update` environment detection.
//!
//! Every probe here shells out to a binary we do not control (`brew`, `npm`,
//! `infocmp`, a located `octos`/`octoscode`). A wedged shim — a broken npm
//! prefix script, a network-mount Homebrew, a binary blocked on a launcher
//! prompt — otherwise hangs the whole diagnostic run (#232 #18), so all of
//! them go through [`probe_output`], which caps the wait and kills the child
//! on expiry instead of blocking forever.

use std::io::Read;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

/// Per-probe budget: generous for a healthy local binary, bounded so a wedged
/// one costs seconds instead of the whole run.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// `try_wait` poll cadence while a probe runs.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Run `command` capturing its output, returning `None` when it cannot be
/// spawned or fails to exit within `timeout` (the child is then killed and
/// reaped, so no zombie is left behind). stdin is null — like
/// [`Command::output`] — so a child blocked on a prompt gets instant EOF
/// instead of consuming the caller's terminal input. Output is piped rather
/// than inherited; a child that floods the pipe buffer blocks on write, never
/// exits, and is reaped by the same timeout path — such output is noise for
/// these probes anyway.
pub(crate) fn probe_output(command: &mut Command, timeout: Duration) -> Option<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return Some(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            // waitpid itself failed: we can't tell whether the child exited,
            // so kill+reap it before giving up rather than leak it unbounded.
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn the test binary running just `blocking_helper` — with the env var
    /// set it sleeps far beyond any probe budget, without it the process exits
    /// immediately (same recursion pattern as the store locale tests).
    fn helper_command(block: bool) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("test binary path"));
        command.args([
            "--exact",
            "cmd::probe::tests::blocking_helper",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]);
        if block {
            command.env("OCTOSCODE_PROBE_TEST_BLOCK", "1");
        }
        command
    }

    #[test]
    #[ignore = "spawned by the probe_output tests as a controllable child"]
    fn blocking_helper() {
        if std::env::var_os("OCTOSCODE_PROBE_TEST_BLOCK").is_some() {
            std::thread::sleep(Duration::from_secs(120));
        } else if std::env::var_os("OCTOSCODE_PROBE_TEST_FLOOD").is_some() {
            // Write far beyond the pipe buffer without ever exiting.
            loop {
                println!("{}", "x".repeat(4096));
            }
        } else {
            println!("PROBE_SENTINEL");
        }
    }

    #[test]
    fn fast_child_returns_its_output() {
        let start = Instant::now();
        let output = probe_output(&mut helper_command(false), Duration::from_secs(10))
            .expect("fast child must complete");

        assert!(
            output.status.success(),
            "helper test run failed: {output:?}"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("PROBE_SENTINEL"),
            "captured stdout must carry the child's print: {output:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "a child that exits at once must return well inside the budget"
        );
    }

    #[test]
    fn wedged_child_is_killed_at_the_timeout() {
        let start = Instant::now();
        let output = probe_output(&mut helper_command(true), Duration::from_millis(500));

        assert!(output.is_none(), "a wedged child must yield None");
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(500),
            "the timeout must actually elapse: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(30),
            "the 120s helper must be killed, not waited out: {elapsed:?}"
        );
    }

    #[test]
    fn pipe_flooding_child_blocks_on_write_and_is_killed() {
        // The child never exits on its own: once the pipe buffer fills it
        // blocks on write, so `try_wait` never reports ready — the timeout
        // path must still kill it.
        let mut command = helper_command(false);
        command.env("OCTOSCODE_PROBE_TEST_FLOOD", "1");
        let start = Instant::now();
        let output = probe_output(&mut command, Duration::from_millis(500));

        assert!(output.is_none(), "a pipe-flooding child must yield None");
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the flooding helper must be killed, not waited out"
        );
    }

    #[test]
    fn unspawnable_command_returns_none() {
        let output = probe_output(
            &mut Command::new("octoscode-probe-test-no-such-binary"),
            Duration::from_millis(50),
        );
        assert!(output.is_none(), "a spawn failure must yield None");
    }
}
