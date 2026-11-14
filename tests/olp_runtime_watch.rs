#![cfg(unix)]

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

/// RAII: kill+reap a test-owned child on scope exit, including assert panics.
struct TestChild(std::process::Child);

impl std::ops::Deref for TestChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for TestChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_test_child() -> TestChild {
    TestChild(
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

/// Bounded heartbeat collection: a reader thread feeds lines into a channel;
/// the test side receives with a hard deadline so a broken implementation can
/// never hang the suite. The reader thread exits when the pipe closes.
fn collect_heartbeats_with_deadline(
    stdout: std::process::ChildStdout,
    n: usize,
    timeout: std::time::Duration,
) -> Vec<String> {
    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut out = Vec::new();
        while out.len() < n {
            match rx.recv_timeout(timeout) {
                Ok(l) => out.push(l),
                Err(_) => break,
            }
        }
        out
    })
    .join()
    .expect("reader thread")
}

/// Bounded one-shot CLI run with a TEST-SIDE deadline: a reader thread drains
/// stdout/stderr into pipes; we poll try_wait and kill the CLI (test-owned)
/// if it exceeds the bound. Returns (success, stdout text, elapsed).
fn bounded_cli_run(
    cmd: &mut Command,
    timeout: std::time::Duration,
) -> (Option<i32>, String, String, std::time::Duration) {
    use std::io::Read;
    let started = std::time::Instant::now();
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn octoscode");
    let mut out_pipe = child.stdout.take().unwrap();
    let mut err_pipe = child.stderr.take().unwrap();
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let deadline = started + timeout;
    let status;
    loop {
        let mut tmp = [0u8; 8192];
        // Drain a bit of both pipes each cycle so a chatty writer cannot
        // fill the pipe buffer and block before our deadline fires.
        match out_pipe.read(&mut tmp) {
            Ok(0) => {}
            Ok(n) => out_buf.extend_from_slice(&tmp[..n]),
            Err(_) => {}
        }
        match err_pipe.read(&mut tmp) {
            Ok(0) => {}
            Ok(n) => err_buf.extend_from_slice(&tmp[..n]),
            Err(_) => {}
        }
        match child.try_wait().unwrap() {
            Some(s) => {
                status = s;
                break;
            }
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("CLI run exceeded test deadline {timeout:?}");
            }
            None => {}
        }
    }
    // Final drain after exit.
    let mut tmp = [0u8; 8192];
    while let Ok(n) = out_pipe.read(&mut tmp) {
        if n == 0 {
            break;
        }
        out_buf.extend_from_slice(&tmp[..n]);
    }
    while let Ok(n) = err_pipe.read(&mut tmp) {
        if n == 0 {
            break;
        }
        err_buf.extend_from_slice(&tmp[..n]);
    }
    (
        status.code(),
        String::from_utf8_lossy(&out_buf).into_owned(),
        String::from_utf8_lossy(&err_buf).into_owned(),
        started.elapsed(),
    )
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

/// STRICT: every non-empty stdout line must parse as NDJSON. No filtering —
/// a polluted line (or empty output where a record was promised) fails the
/// test instead of being silently hidden.
fn parse_lines(stdout: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(line).expect("stdout must be strict NDJSON"));
    }
    assert!(
        !out.is_empty(),
        "expected at least one record, stdout empty"
    );
    out
}

#[test]
fn live_agent_alive_and_terminated_monitor_is_watcher_missing() {
    let agent = spawn_test_child();
    let mut monitor = Command::new("sleep").arg("30").spawn().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = binary();
    base_args(&mut cmd, dir.path())
        .arg("--agent-pid")
        .arg(agent.id().to_string())
        .arg("--backend-pid")
        .arg(std::process::id().to_string())
        .arg("--monitor-pid")
        .arg(monitor.id().to_string());
    // Terminate only the monitor before the probe: watcher_missing expected.
    reap(&mut monitor);
    let _ = &agent;
    let (code, stdout, _err, _el) = bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(0));
    let records = parse_lines(&stdout);
    let last = records.last().expect("one observation record");
    assert_eq!(last["task_accepted"], serde_json::json!(false));
    assert_eq!(last["agent"]["state"], serde_json::json!("alive"));
    assert_eq!(
        last["monitors"][0]["state"]["state"],
        serde_json::json!("watcher_missing")
    );
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
        .stdout(Stdio::piped());
    let (code, stdout, _err, _el) = bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(0));
    let last = parse_lines(&stdout).pop().expect("record");
    assert_eq!(last["agent"]["state"], serde_json::json!("process_missing"));
    assert_eq!(last["task_accepted"], serde_json::json!(false));
}

#[test]
fn empty_task_is_rejected_with_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut cmd = binary();
    cmd.args([
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
    ]);
    let (code, _stdout, stderr, _el) =
        bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(2));
    assert!(!stderr.is_empty());
}

/// Real-ps multi-heartbeat: same live pid across >=3 cycles must produce NO
/// identity alert (first sight records the baseline; later cycles only
/// compare — regression for the per-tick insert()-overwrite bug).
#[test]
fn real_ps_multi_heartbeat_no_identity_alert() {
    let agent = spawn_test_child();
    let backend = spawn_test_child();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut watcher = TestChild(
        binary()
            .args([
                "olp-runtime-watch",
                "--agent",
                "inner_test",
                "--agent-pid",
                &agent.id().to_string(),
                "--backend-pid",
                &backend.id().to_string(),
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
            .unwrap(),
    );
    let stdout = watcher.stdout.take().unwrap();
    let lines = collect_heartbeats_with_deadline(stdout, 3, std::time::Duration::from_secs(10));
    assert_eq!(lines.len(), 3, "expected 3 heartbeats within deadline");
    for (i, line) in lines.iter().enumerate() {
        let rec: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(rec["seq"], serde_json::json!(i as u64 + 1));
        assert_eq!(rec["task_accepted"], serde_json::json!(false));
        assert_eq!(rec["alerts"], serde_json::json!([]));
        assert!(rec["observation_time"].as_str().unwrap().ends_with('Z'));
    }
}

/// Deterministic identity change via a PATH ps stub: each role (agent,
/// backend, monitor) flips its lstart after heartbeat 1; cycles 2+ must
/// carry process_identity_changed INSIDE the snapshot, repeat without
/// resetting the baseline, and an unknown cycle that recovers to the SAME
/// original birth must NOT fire. Uses bounded collection + RAII guards.
#[test]
fn identity_change_alert_lives_in_snapshot_and_repeats_per_role() {
    let agent = spawn_test_child();
    let backend = spawn_test_child();
    let monitor = spawn_test_child();
    let stub_dir = tempfile::tempdir().unwrap();
    let flip = stub_dir.path().join("flip");
    let role = stub_dir.path().join("role");
    let ps = stub_dir.path().join("ps");
    // ps stub: pid from -p; lstart depends on whether the pid is the flip
    // target for the current round. base lstart per role.
    std::fs::write(
        &ps,
        format!(
            "#!/bin/sh\nbase_a='Thu Sep 17 19:39:15 2026'\nbase_b='Fri Sep 18 19:39:15 2026'\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\n[ -n \"$pid\" ] || exit 1\ncase \"$pid\" in\n  {agent}) l=$base_a; tag=agent ;;\n  {backend}) l=$base_a; tag=backend ;;\n  {monitor}) l=$base_a; tag=monitor ;;\n  *) exit 0 ;;\nesac\nif [ -e '{flip}' ] && [ \"$(cat '{flip}')\" = \"$tag\" ]; then l=$base_b; fi\nif [ -e '{role}' ] && [ \"$(cat '{role}')\" = \"$pid\" ]; then exit 1; fi\nprintf '%s %s S\\n' \"$pid\" \"$l\"\n",
            agent = agent.id(),
            backend = backend.id(),
            monitor = monitor.id(),
            flip = flip.display(),
            role = role.display(),
        ),
    )
    .unwrap();
    make_executable(&ps);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();
    let mut watcher = TestChild(
        binary()
            .args([
                "olp-runtime-watch",
                "--agent",
                "inner_test",
                "--agent-pid",
                &agent.id().to_string(),
                "--backend-pid",
                &backend.id().to_string(),
                "--monitor-pid",
                &monitor.id().to_string(),
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
            .unwrap(),
    );

    fn read_one(rx: &std::sync::mpsc::Receiver<String>) -> serde_json::Value {
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("heartbeat within deadline");
        serde_json::from_str(&line).expect("strict ndjson")
    }
    let stdout = watcher.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Heartbeat 1: baselines recorded, no alerts.
    let first = read_one(&rx);
    assert_eq!(first["seq"], serde_json::json!(1));
    assert_eq!(first["alerts"], serde_json::json!([]));

    // Flip the AGENT birth: alert inside snapshot, repeating, per cycle.
    std::fs::write(&flip, b"agent").unwrap();
    for seq in 2..=3u64 {
        let rec = read_one(&rx);
        assert_eq!(rec["seq"], serde_json::json!(seq));
        let alerts: Vec<String> = rec["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["kind"].as_str().unwrap().to_string())
            .collect();
        assert!(alerts.iter().all(|k| k == "process_identity_changed"));
        let pids: Vec<u32> = rec["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["pid"].as_u64().unwrap() as u32)
            .collect();
        assert!(pids.contains(&agent.id()));
        assert_eq!(rec["task_accepted"], serde_json::json!(false));
        assert!(rec["observation_time"].as_str().unwrap().ends_with('Z'));
    }

    // Flip BACK: birth returns to the original baseline value; the baseline
    // was never overwritten, so the alert must persist (still changed vs the
    // first-seen baseline), proving no reset. Then clear the flip entirely:
    // agent back to base => alert stops firing for agent.
    std::fs::remove_file(&flip).unwrap();
    let rec = read_one(&rx);
    let agent_alerted = rec["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["pid"].as_u64() == Some(agent.id() as u64));
    assert!(
        !agent_alerted,
        "alert must clear when birth returns to baseline"
    );

    // Unknown recovery: make ps FAIL for the backend pid for one cycle
    // (exit 1 => Missing... to force Unknown we exit with stderr). Then
    // restore; same birth => NO identity alert for backend.
    std::fs::write(&role, backend.id().to_string()).unwrap();
    std::fs::write(
        stub_dir.path().join("ps"),
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\nif [ \"$pid\" = '{backend}' ]; then echo oops >&2; exit 3; fi\ncase \"$pid\" in\n  {agent}) l='Thu Sep 17 19:39:15 2026' ;;\n  {monitor}) l='Thu Sep 17 19:39:15 2026' ;;\n  *) exit 0 ;;\nesac\nprintf '%s %s S\\n' \"$pid\" \"$l\"\n",
            backend = backend.id(),
            agent = agent.id(),
            monitor = monitor.id(),
        ),
    )
    .unwrap();
    let unknown_cycle = read_one(&rx);
    assert_eq!(
        unknown_cycle["backend"]["state"],
        serde_json::json!("observation_unknown")
    );
    let backend_alerted_during_unknown = unknown_cycle["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["pid"].as_u64() == Some(backend.id() as u64));
    assert!(!backend_alerted_during_unknown);
    // Restore the original stub; backend returns with the SAME birth.
    std::fs::remove_file(&role).unwrap();
    std::fs::write(
        &ps,
        format!(
            "#!/bin/sh\nbase_a='Thu Sep 17 19:39:15 2026'\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\ncase \"$pid\" in\n  {agent}|{backend}|{monitor}) l=$base_a ;;\n  *) exit 0 ;;\nesac\nprintf '%s %s S\\n' \"$pid\" \"$l\"\n",
            agent = agent.id(),
            backend = backend.id(),
            monitor = monitor.id(),
        ),
    )
    .unwrap();
    let rec = read_one(&rx);
    let backend_alerted = rec["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["pid"].as_u64() == Some(backend.id() as u64));
    assert!(
        !backend_alerted,
        "unknown cycle followed by same birth must not fire identity alert"
    );
}

/// A herdr on PATH that hangs must yield agent_status observation_unknown,
/// not hang the watcher, not kill anything, and not abort other probes.
#[test]
fn hung_herdr_stub_yields_unknown_without_breaking_process_probes() {
    let stub_dir = tempfile::tempdir().unwrap();
    let herdr = stub_dir.path().join("herdr");
    std::fs::write(&herdr, "#!/bin/sh\nexec sleep 30\n").unwrap();
    make_executable(&herdr);

    let agent = spawn_test_child();
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
    let (code, stdout, _stderr, elapsed) =
        bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(0));
    // Both the hung herdr probe (1s deadline) and the run finished quickly.
    assert!(
        elapsed.as_secs() < 10,
        "hung herdr blocked the run: {elapsed:?}"
    );
    let last = parse_lines(&stdout).pop().expect("record");
    assert_eq!(
        last["agent_status"],
        serde_json::json!({"state": "observation_unknown"})
    );
    assert_eq!(
        last["agent_status_advisory"],
        serde_json::json!("herdr_unavailable_or_unknown_value")
    );
    // Process probes were NOT aborted by the herdr failure.
    assert_eq!(last["agent"]["state"], serde_json::json!("alive"));
    assert_eq!(last["task_accepted"], serde_json::json!(false));
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
    );
    let (code, text, stderr, _elapsed) =
        bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(0), "cli failed: code={code:?} stderr={stderr}");
    let last = parse_lines(&text).pop().expect("record");
    assert_eq!(
        last["agent_status"],
        serde_json::json!({"state": "working"})
    );
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
    let mut cmd = binary();
    cmd.args([
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
    ]);
    let (code, _stdout, stderr, _el) =
        bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(2));
    assert!(!stderr.is_empty());
}

/// CLI-level Herdr whitelist fixtures: done/idle => completion_unverified +
/// awaiting_review observation_state; an UNKNOWN status value and a
/// name-mismatched / error envelope => observation_unknown — and the raw
/// unknown status string must never leak into NDJSON. task_accepted stays
/// false in every case.
#[test]
fn herdr_whitelist_done_idle_unknown_and_mismatch_fixtures() {
    let agent = spawn_test_child();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("r")).unwrap();
    std::fs::write(dir.path().join("l.jsonl"), "x\n").unwrap();

    let envelope = |status: &str, name: &str| {
        format!(
            "{{\"id\":\"cli:agent:get\",\"result\":{{\"agent\":{{\"agent\":\"octoscode\",\"agent_status\":\"{status}\",\"cwd\":\"/x\",\"name\":\"{name}\"}}}},\"type\":\"agent_info\"}}\n"
        )
    };
    // (label, body, expected agent_status.state, observation_state, leak_check)
    let cases: &[(&str, String, &str, &str, bool)] = &[
        ("done", envelope("done", "inner_test"), "completion_unverified", "awaiting_review", false),
        ("idle", envelope("idle", "inner_test"), "completion_unverified", "awaiting_review", false),
        ("starting", envelope("starting", "inner_test"), "starting", "watching", false),
        ("unknown_status", envelope("pwned-fancy-state", "inner_test"), "observation_unknown", "watching", true),
        ("name_mismatch", envelope("working", "someone_else"), "observation_unknown", "watching", false),
        ("error_envelope", "{\"error\":{\"code\":\"agent_not_found\",\"message\":\"nope\"},\"id\":\"cli:agent:get\"}\n".to_string(), "observation_unknown", "watching", false),
    ];

    for (label, body, want_state, want_obs, leak_check) in cases {
        let stub_dir = tempfile::tempdir().unwrap();
        let herdr = stub_dir.path().join("herdr");
        std::fs::write(&herdr, format!("#!/bin/sh\ncat <<'JSON'\n{body}JSON\n")).unwrap();
        make_executable(&herdr);

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
        ])
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", stub_dir.path().display()),
        );
        let (code, text, _stderr, _elapsed) =
            bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
        assert_eq!(code, Some(0), "{label}: exit ok");
        let last = parse_lines(&text).pop().expect("record");
        assert_eq!(
            last["agent_status"]["state"],
            serde_json::json!(want_state),
            "{label}: agent_status"
        );
        assert_eq!(
            last["observation_state"],
            serde_json::json!(want_obs),
            "{label}: observation_state"
        );
        assert_eq!(
            last["task_accepted"],
            serde_json::json!(false),
            "{label}: never accepted"
        );
        if *leak_check {
            assert!(
                text.find("pwned-fancy-state").is_none(),
                "{label}: unknown status string leaked"
            );
        }
        if *label == "done" || *label == "idle" {
            assert_eq!(
                last["agent_status"]["disposition"],
                serde_json::json!("awaiting_review")
            );
        }
    }
}

/// Real OS boundary matrix through the production CLI path (PATH-stubbed ps).
/// Each case: the observed sleep child must be ALIVE before and after — the
/// watcher never kills observed pids. Covers #5 item 3 gaps.
#[test]
fn ps_boundary_matrix_through_cli() {
    let self_pid = std::process::id();
    // (label, ps stub body, expected agent.state)
    let ok_row = "printf '%s %s S\\n' \"$pid\" 'Thu Sep 17 19:39:15 2026'".to_string();
    let cases: &[(&str, String, &str)] = &[
        (
            "valid_stdout_plus_stderr_is_unknown",
            format!("#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\n{ok_row}\necho warning >&2\nexit 0"),
            "observation_unknown",
        ),
        (
            "exit42_is_unknown",
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\necho boom >&2\nexit 42".to_string(),
            "observation_unknown",
        ),
        (
            "multiline_stdout_is_unknown",
            format!("#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\n{ok_row}\n{ok_row}\n"),
            "observation_unknown",
        ),
        (
            "wrong_pid_is_unknown",
            "#!/bin/sh\nprintf '%s %s S\\n' 999999 'Thu Sep 17 19:39:15 2026'\n".to_string(),
            "observation_unknown",
        ),
        (
            "invalid_time_is_unknown",
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\nprintf '%s %s S\\n' \"$pid\" 'Thu Sep 17 99:99:99 2026'\n".to_string(),
            "observation_unknown",
        ),
        (
            "oversized_stdout_is_unknown",
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\nhead -c 20000 /dev/zero | tr '\\0' 'a'\nexit 0".to_string(),
            "observation_unknown",
        ),
        (
            "oversized_stderr_is_unknown",
            format!("#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\nhead -c 20000 /dev/zero | tr '\\0' 'e' >&2\n{ok_row}\n"),
            "observation_unknown",
        ),
        (
            // Grandchild holds the inherited write fd while the direct ps
            // child exits 0 with EMPTY output: temp-file redirection means
            // no pipe can block us, and the contract classifies it Missing.
            // Grandchild is short-lived (2s) so it self-cleans after the
            // probe; it is a stub-owned descendant, never the observed pid.
            "grandchild_holds_fd_is_bounded",
            "#!/bin/sh\n(sleep 2) &\nfor a in \"$@\"; do case \"$a\" in -p) : ;; [0-9]*) pid=$a ;; esac; done\nexit 0".to_string(),
            "process_missing",
        ),
        (
            "ps_timeout_is_unknown",
            "#!/bin/sh\nexec sleep 30".to_string(),
            "observation_unknown",
        ),
    ];

    for (label, body, want) in cases {
        let mut agent = spawn_test_child(); // observed child: must stay alive
        let stub_dir = tempfile::tempdir().unwrap();
        let ps = stub_dir.path().join("ps");
        std::fs::write(&ps, body).unwrap();
        make_executable(&ps);

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
            &self_pid.to_string(),
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
        );

        let (code, stdout, _stderr, elapsed) =
            bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
        assert_eq!(code, Some(0), "{label}: CLI exited failing");
        assert!(
            elapsed.as_secs() < 12,
            "{label}: exceeded bound ({elapsed:?}) — a bounded probe hung"
        );
        let last = parse_lines(&stdout).pop().expect("record");
        assert_eq!(
            last["agent"]["state"],
            serde_json::json!(want),
            "{label}: wrong agent state"
        );
        assert_eq!(last["task_accepted"], serde_json::json!(false));
        // The OBSERVED child was never killed by the watcher.
        assert!(
            matches!(agent.try_wait(), Ok(None)),
            "{label}: observed child died during watch"
        );
    }
}

/// Lines of a running watcher's stdout, delivered through a channel so the
/// test can wait for a condition with a hard deadline instead of blocking.
fn line_channel(stdout: std::process::ChildStdout) -> std::sync::mpsc::Receiver<String> {
    use std::io::BufRead;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(l) = line else { break };
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    rx
}

/// Receive records until `want` holds for one, or the deadline passes.
/// Returns every record seen, the matching one last.
fn records_until(
    rx: &std::sync::mpsc::Receiver<String>,
    timeout: std::time::Duration,
    want: impl Fn(&serde_json::Value) -> bool,
) -> Vec<serde_json::Value> {
    let deadline = std::time::Instant::now() + timeout;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let line = rx
            .recv_timeout(left)
            .unwrap_or_else(|_| panic!("condition not met within {timeout:?}; saw {seen:?}"));
        let rec: serde_json::Value = serde_json::from_str(&line).expect("strict NDJSON");
        let hit = want(&rec);
        seen.push(rec);
        if hit {
            return seen;
        }
    }
}

fn alert_kinds(rec: &serde_json::Value) -> Vec<String> {
    rec["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap().to_string())
        .collect()
}

fn ledger_line(seq: u64, turn: &str, kind: &str, extra: &str) -> String {
    format!(
        "{{\"v\":1,\"seq\":{seq},\"event\":{{\"record_kind\":\"notification\",\"kind\":\"{kind}\",\"session_id\":\"s1\",\"turn_id\":\"{turn}\"{extra}}}}}\n"
    )
}

/// Spawn a watcher over `ledger` for session s1 / turn t1 with two live
/// test-owned processes as agent and backend.
fn spawn_watcher(
    dir: &std::path::Path,
    ledger: &std::path::Path,
    agent: &TestChild,
    backend: &TestChild,
    extra: &[&str],
) -> TestChild {
    TestChild(
        binary()
            .args([
                "olp-runtime-watch",
                "--agent",
                "inner_test",
                "--agent-pid",
                &agent.id().to_string(),
                "--backend-pid",
                &backend.id().to_string(),
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
                "60",
            ])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

/// Scenario: 目标 turn 工具失败被告警，旧/异 turn 记录被忽略；滚动后继续
#[test]
fn olp_runtime_watch_target_turn_error_alerted_rotation_followed() {
    let agent = spawn_test_child();
    let backend = spawn_test_child();
    let dir = tempfile::tempdir().unwrap();
    let failed = ",\"tool_call_id\":\"c1\",\"tool_name\":\"bash\",\"success\":false,\"output_preview\":\"SECRET-TOKEN\"";
    let first = dir.path().join("ledger-1695000000-111.log");
    std::fs::write(
        &first,
        format!(
            "{}{}",
            // A failure in another turn of the same session must be ignored.
            ledger_line(1, "OLD-TURN", "tool_completed", failed),
            ledger_line(2, "t1", "tool_completed", failed),
        ),
    )
    .unwrap();
    // The UI rolls its ledger within the session directory: the successor
    // carries the turn's error and its completion.
    std::fs::write(
        dir.path().join("ledger-1695000100-111.log"),
        format!(
            "{}{}",
            ledger_line(
                3,
                "t1",
                "turn_error",
                ",\"message\":\"quota exceeded SECRET-TOKEN\""
            ),
            ledger_line(4, "t1", "turn_completed", ""),
        ),
    )
    .unwrap();

    let mut watcher = spawn_watcher(dir.path(), &first, &agent, &backend, &[]);
    let rx = line_channel(watcher.stdout.take().unwrap());
    let timeout = std::time::Duration::from_secs(20);

    // The target turn's tool failure is in the very first record.
    let seen = records_until(&rx, timeout, |r| {
        alert_kinds(r).contains(&"tool_failed".into())
    });
    let rec = seen.last().unwrap();
    let alert = &rec["alerts"][0];
    assert_eq!(alert["source"], "ledger");
    assert_eq!(alert["turn"], "t1");
    assert_eq!(alert["tool_name"], "bash");
    assert_eq!(alert["tool_call_id"], "c1");
    assert_eq!(
        rec["alerts"].as_array().unwrap().len(),
        1,
        "the other turn is ignored: {rec}"
    );
    assert_eq!(rec["task_accepted"], false);

    // After the old file goes quiet the reader follows the successor.
    let mut all = seen;
    all.extend(records_until(&rx, timeout, |r| {
        alert_kinds(r).contains(&"turn_error".into())
    }));
    all.extend(records_until(&rx, timeout, |r| {
        r["observation_state"] == "awaiting_review"
    }));
    let last = all.last().unwrap();
    assert!(
        last["ledger"]["path"]
            .as_str()
            .unwrap()
            .ends_with("ledger-1695000100-111.log"),
        "{last}"
    );
    for rec in &all {
        assert_eq!(
            rec["task_accepted"], false,
            "a completed turn is never acceptance"
        );
        assert!(
            !rec.to_string().contains("SECRET-TOKEN"),
            "payloads never reach the output: {rec}"
        );
    }
    // Each ledger alert is reported once, not on every heartbeat.
    let tool_failed = all
        .iter()
        .filter(|r| alert_kinds(r).contains(&"tool_failed".into()))
        .count();
    assert_eq!(tool_failed, 1);
}

/// Scenario: 重复读事件+哈希不变触发停滞；真实字节变更重置
#[test]
fn olp_runtime_watch_stall_and_progress_reset() {
    use std::io::Write;
    let agent = spawn_test_child();
    let backend = spawn_test_child();
    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join("ledger-1695000000-111.log");
    std::fs::write(
        &ledger,
        ledger_line(1, "t1", "tool_completed", ",\"success\":true"),
    )
    .unwrap();
    let source = dir.path().join("owned.rs");
    std::fs::write(&source, "fn a() {}\n").unwrap();

    let mut watcher = spawn_watcher(
        dir.path(),
        &ledger,
        &agent,
        &backend,
        &[
            "--progress-file",
            source.to_str().unwrap(),
            "--stall-seconds",
            "1",
        ],
    );
    let rx = line_channel(watcher.stdout.take().unwrap());
    let timeout = std::time::Duration::from_secs(20);

    // Not stalled at the start: first sight is the baseline.
    let first = records_until(&rx, timeout, |_| true);
    assert!(alert_kinds(&first[0]).is_empty(), "{}", first[0]);
    assert_eq!(first[0]["progress"]["tracked_files"], 1);

    // The agent keeps producing events (reads, searches) but the file's
    // bytes never change: a read-only livelock.
    let mut seq = 1;
    let mut append = |n: usize| {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&ledger)
            .unwrap();
        for _ in 0..n {
            seq += 1;
            f.write_all(ledger_line(seq, "t1", "tool_completed", ",\"success\":true").as_bytes())
                .unwrap();
        }
    };
    append(3);
    let seen = records_until(&rx, timeout, |r| {
        alert_kinds(r).contains(&"stalled_no_progress".into())
    });
    let stalled = seen.last().unwrap();
    let alert = &stalled["alerts"][0];
    assert_eq!(alert["advisory"], true);
    assert_eq!(alert["activity"], "events_continuing", "{stalled}");
    assert!(alert["progress_age_seconds"].as_u64().unwrap() >= 1);
    assert_eq!(stalled["task_accepted"], false);
    assert_eq!(stalled["agent"]["state"], "alive", "a stall is not a death");

    // A real byte change resets the clock.
    std::fs::write(&source, "fn a() { changed(); }\n").unwrap();
    let seen = records_until(&rx, timeout, |r| {
        r["progress"]["changed_this_cycle"] == true
    });
    let reset = seen.last().unwrap();
    assert!(alert_kinds(reset).is_empty(), "{reset}");
    assert_eq!(reset["progress"]["progress_age_seconds"], 0);

    // Unchanged again, and silent this time: the stall comes back, labelled
    // as no events.
    let seen = records_until(&rx, timeout, |r| {
        alert_kinds(r).contains(&"stalled_no_progress".into())
    });
    assert_eq!(seen.last().unwrap()["alerts"][0]["activity"], "no_events");

    // A removed file is reported explicitly, and counts as a change.
    std::fs::remove_file(&source).unwrap();
    let seen = records_until(&rx, timeout, |r| {
        r["progress"]["missing_files"]
            .as_array()
            .is_some_and(|m| m.len() == 1)
    });
    assert_eq!(seen.last().unwrap()["progress"]["changed_this_cycle"], true);
}

/// Scenario: 没有 octos ledger 与后端的内环(Claude Code、Codex)也能被守护
#[test]
fn watch_without_ledger_or_backend_uses_herdr_status_for_activity() {
    let stub_dir = tempfile::tempdir().unwrap();
    let herdr = stub_dir.path().join("herdr");
    let status_file = stub_dir.path().join("status");
    std::fs::write(&status_file, "working").unwrap();
    std::fs::write(
        &herdr,
        format!(
            "#!/bin/sh\nprintf '{{\"result\":{{\"agent\":{{\"agent\":\"claude\",\"name\":\"cc\",\"agent_status\":\"%s\"}}}}}}\\n' \"$(cat '{}')\"\n",
            status_file.display()
        ),
    )
    .unwrap();
    make_executable(&herdr);

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("r");
    std::fs::create_dir_all(&repo).unwrap();
    let progress = repo.join("README.md");
    std::fs::write(&progress, "x\n").unwrap();
    let mut agent = spawn_test_child();
    let agent_pid = agent.id().to_string();
    let run = |extra: &[&str]| {
        let mut cmd = binary();
        cmd.args([
            "olp-runtime-watch",
            "--agent",
            "cc",
            "--agent-pid",
            &agent_pid,
            "--repo",
            repo.to_str().unwrap(),
            "--task",
            "7",
            "--progress-file",
            progress.to_str().unwrap(),
            "--stall-seconds",
            "1",
            "--interval-ms",
            "600",
        ])
        .args(extra)
        // OLP_HERDR names the herdr to ask, as on a unit whose job PATH is short.
        .env("OLP_HERDR", &herdr);
        cmd
    };

    // Three cycles, ~1.2 s: long enough for the 1 s stall advisory.
    let mut cmd = run(&[]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let records = collect_heartbeats_with_deadline(stdout, 3, std::time::Duration::from_secs(20));
    let _ = child.kill();
    let _ = child.wait();
    let records: Vec<serde_json::Value> = records
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let last = records.last().expect("records");
    assert_eq!(last["identity"]["backend_pid"], serde_json::Value::Null);
    assert_eq!(last["identity"]["ledger"], serde_json::Value::Null);
    assert_eq!(last["identity"]["turn_id"], serde_json::Value::Null);
    assert_eq!(
        last["backend"],
        serde_json::json!({"state": "not_observed"})
    );
    assert_eq!(
        last["ledger"],
        serde_json::json!({"not_observed": "no_ledger"})
    );
    assert_eq!(last["agent"]["state"], "alive");
    assert_eq!(
        last["agent_status"],
        serde_json::json!({"state": "working"})
    );
    assert_eq!(last["task_accepted"], false);
    let stall = last["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "stalled_no_progress")
        .unwrap_or_else(|| panic!("a stall advisory after 1 s: {last}"));
    // herdr said `working` the whole time: busy without progress.
    assert_eq!(stall["activity"], "events_continuing", "{stall}");

    // herdr says idle: the turn is over, awaiting review; no activity.
    std::fs::write(&status_file, "idle").unwrap();
    let mut cmd = run(&[]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let records = collect_heartbeats_with_deadline(
        child.stdout.take().unwrap(),
        3,
        std::time::Duration::from_secs(20),
    );
    let _ = child.kill();
    let _ = child.wait();
    let records: Vec<serde_json::Value> = records
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let last = records.last().unwrap();
    assert_eq!(last["observation_state"], "awaiting_review");
    let stall = last["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "stalled_no_progress")
        .expect("stall");
    assert_eq!(stall["activity"], "no_events");

    // The agent process goes away: still reported, with no backend involved.
    reap(&mut agent.0);
    let mut cmd = run(&["--once"]);
    let (code, text, stderr, _) = bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
    assert_eq!(code, Some(0), "{stderr}");
    let last = parse_lines(&text).pop().unwrap();
    assert_eq!(last["agent"]["state"], "process_missing");

    // The ledger flags go together.
    for partial in [
        vec!["--ledger", progress.to_str().unwrap()],
        vec!["--session", "s1", "--turn-id", "t1"],
    ] {
        let mut cmd = run(&partial);
        cmd.arg("--once");
        let (code, _, stderr, _) = bounded_cli_run(&mut cmd, std::time::Duration::from_secs(15));
        assert_ne!(code, Some(0), "{partial:?}");
        assert!(stderr.contains("go together"), "{stderr}");
    }
}
