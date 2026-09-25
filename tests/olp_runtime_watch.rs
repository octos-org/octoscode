//! Real-subprocess tests for `octoscode olp-runtime-watch` (spec
//! task-olp-runtime-watch: rules identity-snapshot + process-probe).
//! Test-owned children are spawned, terminated, and reaped here; the guard
//! itself never kills anything.

use std::process::{Command, Stdio};

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

/// Real-ps multi-heartbeat: same live pid across >=3 cycles must produce NO
/// identity alert (first sight records the baseline; later cycles only
/// compare — regression for the per-tick insert()-overwrite bug).
#[test]
fn real_ps_multi_heartbeat_no_identity_alert() {
    let mut agent = Command::new("sleep").arg("30").spawn().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut watcher = binary()
        .args([
            "olp-runtime-watch",
            "--agent",
            "inner_test",
            "--agent-pid",
            &agent.id().to_string(),
            "--backend-pid",
            &std::process::id().to_string(),
            "--repo",
            dir.path().join("r").to_str().unwrap(),
            "--ledger",
            dir.path().join("l.jsonl").to_str().unwrap(),
            "--session",
            "s",
            "--turn-id",
            "t",
            "--task",
            "guard",
            "--interval-ms",
            "80",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = String::new();
    {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(watcher.stdout.take().unwrap());
        for i in 1..=3u64 {
            let mut line = String::new();
            reader.read_line(&mut line).expect("heartbeat line");
            let rec: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(rec["seq"], serde_json::json!(i));
            assert_eq!(rec["task_accepted"], serde_json::json!(false));
            assert_eq!(rec["alerts"], serde_json::json!([]));
            assert!(rec["observation_time"].as_str().unwrap().ends_with('Z'));
        }
    }
    let _ = &mut out;
    watcher.kill().unwrap();
    let _ = watcher.wait();
    reap(&mut agent);
}

/// Deterministic identity change via a PATH ps stub that flips the agent
/// pid's lstart after the first heartbeat: cycle 2+ must carry the alert
/// INSIDE the snapshot (seq/observation_time/identity/task_accepted) and
/// repeat it (baseline is never overwritten). Backend pid stays constant.
#[test]
fn identity_change_alert_lives_in_snapshot_and_repeats() {
    let mut agent = Command::new("sleep").arg("30").spawn().unwrap();
    let stub_dir = tempfile::tempdir().unwrap();
    let flip = stub_dir.path().join("flip");
    let ps = stub_dir.path().join("ps");
    std::fs::write(
        &ps,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do :; done\npid=\"$a\"\nif [ \"$pid\" = '{agent}' ] && [ -e '{flip}' ]; then l='Fri Sep 18 19:39:15 2026'; else l='Thu Sep 17 19:39:15 2026'; fi\nprintf '%s %s S\\n' \"$pid\" \"$l\"\n",
            agent = agent.id(),
            flip = flip.display(),
        ),
    )
    .unwrap();
    make_executable(&ps);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut watcher = binary()
        .args([
            "olp-runtime-watch",
            "--agent",
            "inner_test",
            "--agent-pid",
            &agent.id().to_string(),
            "--backend-pid",
            &std::process::id().to_string(),
            "--repo",
            dir.path().join("r").to_str().unwrap(),
            "--ledger",
            dir.path().join("l.jsonl").to_str().unwrap(),
            "--session",
            "s",
            "--turn-id",
            "t",
            "--task",
            "guard",
            "--interval-ms",
            "80",
        ])
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", stub_dir.path().display()),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(watcher.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let first: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(first["seq"], serde_json::json!(1));
    assert_eq!(first["alerts"], serde_json::json!([]));
    std::fs::write(&flip, b"1").unwrap();
    for seq in 2..=3u64 {
        let mut line = String::new();
        reader.read_line(&mut line).expect("heartbeat line");
        let rec: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(rec["seq"], serde_json::json!(seq));
        // Alert is INSIDE the snapshot with identity/time, never a bare line.
        assert_eq!(
            rec["alerts"][0]["kind"],
            serde_json::json!("process_identity_changed")
        );
        assert_eq!(rec["alerts"][0]["pid"], serde_json::json!(agent.id()));
        assert_eq!(rec["identity"]["agent_pid"], serde_json::json!(agent.id()));
        assert!(rec["observation_time"].as_str().unwrap().ends_with('Z'));
        assert_eq!(rec["task_accepted"], serde_json::json!(false));
    }
    watcher.kill().unwrap();
    let _ = watcher.wait();
    reap(&mut agent);
}

/// A herdr on PATH that hangs must yield agent_status observation_unknown,
/// not hang the watcher, not kill anything, and not abort other probes.
#[test]
fn hung_herdr_stub_yields_unknown_without_breaking_process_probes() {
    let stub_dir = tempfile::tempdir().unwrap();
    let herdr = stub_dir.path().join("herdr");
    std::fs::write(&herdr, "#!/bin/sh\nexec sleep 30\n").unwrap();
    make_executable(&herdr);

    let mut agent = Command::new("sleep").arg("30").spawn().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut cmd = binary();
    cmd.args([
        "olp-runtime-watch",
        "--once",
        "--agent",
        "inner_test",
        "--agent-pid",
        &agent.id().to_string(),
        "--backend-pid",
        &std::process::id().to_string(),
        "--repo",
        dir.path().join("r").to_str().unwrap(),
        "--ledger",
        dir.path().join("l.jsonl").to_str().unwrap(),
        "--session",
        "s",
        "--turn-id",
        "t",
        "--task",
        "guard",
        "--interval-ms",
        "50",
    ])
    .env(
        "PATH",
        format!("{}:/usr/bin:/bin", stub_dir.path().display()),
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let started = std::time::Instant::now();
    let out = cmd.output().unwrap();
    let elapsed = started.elapsed();
    assert!(out.status.success());
    // Both the hung herdr probe (1s deadline) and the run finished quickly.
    assert!(
        elapsed.as_secs() < 10,
        "hung herdr blocked the run: {elapsed:?}"
    );
    let last = parse_lines(&String::from_utf8_lossy(&out.stdout))
        .pop()
        .expect("record");
    assert_eq!(
        last["agent_status"],
        serde_json::json!("observation_unknown")
    );
    assert_eq!(
        last["agent_status_advisory"],
        serde_json::json!("herdr_unavailable_observation_unknown")
    );
    // Process probes were NOT aborted by the herdr failure.
    assert_eq!(last["agent"]["state"], serde_json::json!("alive"));
    assert_eq!(last["task_accepted"], serde_json::json!(false));
    reap(&mut agent);
}

/// A herdr stub emitting the REAL envelope must surface agent_status only
/// (whitelist), with the exact agent name matched.
#[test]
fn herdr_stub_real_envelope_surfaces_agent_status_only() {
    let stub_dir = tempfile::tempdir().unwrap();
    let herdr = stub_dir.path().join("herdr");
    std::fs::write(
        &herdr,
        "#!/bin/sh\ncat <<'JSON'\n{\"id\":\"cli:agent:get\",\"result\":{\"agent\":{\"agent\":\"octoscode\",\"agent_status\":\"working\",\"cwd\":\"/cwd-leak-token-zqx\",\"name\":\"inner_test\",\"terminal_title\":\"x\"}},\"type\":\"agent_info\"}\nJSON\n",
    )
    .unwrap();
    make_executable(&herdr);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut cmd = binary();
    cmd.args([
        "olp-runtime-watch",
        "--once",
        "--agent",
        "inner_test",
        "--agent-pid",
        &std::process::id().to_string(),
        "--backend-pid",
        &std::process::id().to_string(),
        "--repo",
        dir.path().join("r").to_str().unwrap(),
        "--ledger",
        dir.path().join("l.jsonl").to_str().unwrap(),
        "--session",
        "s",
        "--turn-id",
        "t",
        "--task",
        "guard",
    ])
    .env(
        "PATH",
        format!("{}:/usr/bin:/bin", stub_dir.path().display()),
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let last = parse_lines(&text).pop().expect("record");
    assert_eq!(last["agent_status"], serde_json::json!("working"));
    // Whitelist: cwd / terminal_title from the envelope must NOT leak.
    assert!(
        text.find("cwd-leak-token-zqx").is_none(),
        "cwd must not leak into NDJSON"
    );
    assert!(text.find("terminal_title").is_none());
    assert_eq!(last["task_accepted"], serde_json::json!(false));
}

fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
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
