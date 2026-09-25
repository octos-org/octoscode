//! Real-subprocess tests for `octoscode olp-runtime-watch` (spec
//! task-olp-runtime-watch: rules identity-snapshot + process-probe).
//! Test-owned children are spawned, terminated, and reaped here; the guard
//! itself never kills anything.

use std::process::{Command, Stdio};

/// Spawn a disposable `sleep` child (test-owned) and return its pid.
fn spawn_child() -> u32 {
    Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sleep")
        .id()
}

/// Terminate + reap a test-owned child.
fn reap(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_octoscode"))
}

fn base_args<'a>(cmd: &'a mut Command, dir: &std::path::Path) -> &'a mut Command {
    let ledger = dir.join("ledger.jsonl");
    std::fs::write(&ledger, "{\"event\":{\"turn_id\":\"t1\"}}\n").unwrap();
    cmd.args([
        "olp-runtime-watch",
        "--once",
        "--agent",
        "inner_test",
        "--repo",
        dir.to_str().unwrap(),
        "--ledger",
        ledger.to_str().unwrap(),
        "--session",
        "s1",
        "--turn-id",
        "t1",
        "--task",
        "guard",
        "--interval-ms",
        "50",
    ])
}

fn parse_lines(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).expect("valid ndjson"))
        .collect()
}

#[test]
fn live_agent_alive_and_terminated_monitor_is_watcher_missing() {
    let mut agent = Command::new("sleep").arg("30").spawn().unwrap();
    let mut monitor = Command::new("sleep").arg("30").spawn().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = binary();
    base_args(&mut cmd, dir.path())
        .arg("--agent-pid")
        .arg(agent.id().to_string())
        .arg("--backend-pid")
        .arg(std::process::id().to_string())
        .arg("--monitor-pid")
        .arg(monitor.id().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Terminate only the monitor before the probe: watcher_missing expected.
    reap(&mut monitor);
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let records = parse_lines(&String::from_utf8_lossy(&out.stdout));
    let last = records.last().expect("one observation record");
    assert_eq!(last["task_accepted"], serde_json::json!(false));
    assert_eq!(last["agent"]["state"], serde_json::json!("alive"));
    assert_eq!(
        last["monitors"][0]["state"]["state"],
        serde_json::json!("watcher_missing")
    );
    reap(&mut agent);
}

#[test]
fn terminated_agent_is_process_missing_never_acceptance() {
    let mut agent = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = agent.id();
    reap(&mut agent);
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = binary();
    base_args(&mut cmd, dir.path())
        .arg("--agent-pid")
        .arg(pid.to_string())
        .arg("--backend-pid")
        .arg(std::process::id().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let last = parse_lines(&String::from_utf8_lossy(&out.stdout))
        .pop()
        .expect("record");
    assert_eq!(last["agent"]["state"], serde_json::json!("process_missing"));
    assert_eq!(last["task_accepted"], serde_json::json!(false));
}

#[test]
fn empty_task_is_rejected_with_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let out = binary()
        .args([
            "olp-runtime-watch",
            "--once",
            "--agent",
            "a",
            "--agent-pid",
            "1",
            "--backend-pid",
            "2",
            "--repo",
            dir.path().join("r").to_str().unwrap(),
            "--ledger",
            dir.path().join("l.jsonl").to_str().unwrap(),
            "--session",
            "s",
            "--turn-id",
            "t",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!out.stderr.is_empty());
}

#[test]
fn zero_pid_is_rejected_with_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let out = binary()
        .args([
            "olp-runtime-watch",
            "--once",
            "--agent",
            "a",
            "--agent-pid",
            "0",
            "--backend-pid",
            "1",
            "--repo",
            dir.path().join("r").to_str().unwrap(),
            "--ledger",
            dir.path().join("l.jsonl").to_str().unwrap(),
            "--session",
            "s",
            "--turn-id",
            "t",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!out.stderr.is_empty());
}
