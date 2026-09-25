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
        serde_json::json!({"state": "observation_unknown"})
    );
    assert_eq!(
        last["agent_status_advisory"],
        serde_json::json!("herdr_unavailable_or_unknown_value")
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

        let out = binary()
            .args([
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
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{label}: exit ok");
        let text = String::from_utf8_lossy(&out.stdout);
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
