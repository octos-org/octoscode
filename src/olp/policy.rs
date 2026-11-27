//! External policies: a decision this tool makes by a fixed rule can be
//! asked of a program outside it instead, so the rule can change without
//! rebuilding the tool.
//!
//! The caller stays in charge. It gathers the facts, hands them over as
//! one JSON document on stdin, reads one JSON object from stdout, checks
//! that answer against what a policy is allowed to say, and falls back to
//! its own rule when the policy fails, runs out of time or answers out of
//! bounds. A policy never touches the log, a lock or a gate.
//!
//! A policy is any command. With the `relax` cargo feature it can also be
//! a Rust package run straight from source by the Relax JIT
//! (<https://github.com/makepad/relax>), which is what makes editing one
//! a matter of seconds. Relax is an external program found at run time,
//! never a dependency of this crate.

use std::io::{Read, Write};
#[cfg(feature = "relax")]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a policy given as a command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
/// A package run by Relax compiles itself on its first run.
#[cfg(feature = "relax")]
const RELAX_TIMEOUT: Duration = Duration::from_secs(180);
/// More output than this is not a decision.
const MAX_OUTPUT: u64 = 1 << 20;

pub enum Source {
    /// A command line, split the way a shell would; no shell runs it.
    Command(String),
    /// A Rust package directory, run by Relax.
    #[cfg(feature = "relax")]
    Package(PathBuf),
}

/// `--policy-crate <目录>`: a policy written as a Rust package.
#[cfg(feature = "relax")]
pub fn package(dir: &str) -> Result<Source, String> {
    let dir = PathBuf::from(dir);
    package_name(&dir)?;
    Ok(Source::Package(dir))
}

#[cfg(not(feature = "relax"))]
pub fn package(_dir: &str) -> Result<Source, String> {
    Err(
        "这个构建没有带 Relax 支持:用 `cargo build --features relax` 构建,或把策略做成命令交给 --policy"
            .to_string(),
    )
}

/// The `name` of the `[package]` table of `<dir>/Cargo.toml`.
#[cfg(feature = "relax")]
fn package_name(dir: &std::path::Path) -> Result<String, String> {
    let manifest = dir.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("读不了 {}: {e}", manifest.display()))?;
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if in_package && key.trim() == "name" {
            let name = value.trim().trim_matches('"');
            let plain = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if plain {
                return Ok(name.to_string());
            }
        }
    }
    Err(format!("{} 里没有 [package] 的 name", manifest.display()))
}

fn invocation(source: &Source) -> Result<(Command, Duration), String> {
    match source {
        Source::Command(line) => {
            let words = shlex::split(line)
                .filter(|w| !w.is_empty())
                .ok_or("命令写法不对")?;
            let mut command = Command::new(&words[0]);
            command.args(&words[1..]);
            Ok((command, COMMAND_TIMEOUT))
        }
        #[cfg(feature = "relax")]
        Source::Package(dir) => {
            let name = package_name(dir)?;
            let relax = std::env::var("OLP_RELAX")
                .ok()
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "relax".to_string());
            // --unattended: a panic or a compile error ends the run instead
            // of waiting for somebody to attach.
            let mut command = Command::new(relax);
            command
                .args(["run", "--unattended", "--quiet", &name])
                .current_dir(dir);
            Ok((command, RELAX_TIMEOUT))
        }
    }
}

/// Runs the policy on `input` and returns the JSON object it printed (the
/// last line of stdout that opens one). `Err` says why there is none.
pub fn ask(source: &Source, input: Vec<u8>) -> Result<String, String> {
    let (mut command, timeout) = invocation(source)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("起不来: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A policy that never reads must not block us.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let stdout = child.stdout.take().ok_or("没有 stdout")?;
    // The answer comes back over a channel: a grandchild that keeps the
    // pipe open after the policy has ended must not hold us.
    let (said, heard) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.take(MAX_OUTPUT).read_to_string(&mut text);
        let _ = said.send(text);
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("超过 {} 秒没有结束", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    };
    if !status.success() {
        return Err(format!("退出码 {}", status.code().unwrap_or(-1)));
    }
    let text = heard
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "读不到输出(它结束后输出管道仍被占着)")?;
    text.lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .map(str::to_string)
        .ok_or_else(|| "输出里没有 JSON".to_string())
}
