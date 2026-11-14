#![cfg(unix)]

//! Contract tests for `scripts/olp-alert-forward.py` (SDD spec:
//! `specs/task-olp-runtime-watch.spec.md`, rule hub-forward): a unit's
//! runtime-watch records are reduced to board lines on the hub.
//!
//! ssh is replaced by a script that runs the hub-side forced command of
//! `olp-board-remote.py` locally; a flag file makes the hub unreachable.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn scripts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Net {
    dir: tempfile::TempDir,
}

impl Net {
    fn new() -> Self {
        let n = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(n.board(), "# 板\n\n### 212. 一张卡\nunit: m07\n\n").unwrap();
        let script = format!(
            "#!/bin/sh\n[ -e '{down}' ] && exit 255\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\nSSH_ORIGINAL_COMMAND=\"$*\" exec python3 -B '{tool}' serve --unit m07 --board '{board}'\n",
            down = n.down().display(),
            tool = scripts().join("olp-board-remote.py").display(),
            board = n.board().display(),
        );
        std::fs::write(n.fake_ssh(), script).unwrap();
        std::fs::set_permissions(n.fake_ssh(), std::fs::Permissions::from_mode(0o755)).unwrap();
        n
    }

    fn board(&self) -> PathBuf {
        self.dir.path().join("board.md")
    }
    fn fake_ssh(&self) -> PathBuf {
        self.dir.path().join("fake-ssh")
    }
    fn down(&self) -> PathBuf {
        self.dir.path().join("hub.down")
    }
    fn queue(&self) -> PathBuf {
        self.dir.path().join("queue")
    }

    fn command(&self, script: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(scripts().join(script))
            .args(args)
            .env("OLP_SSH", self.fake_ssh())
            .env("OLP_HUB", "olp@hub-01")
            .env("OLP_UNIT_ID", "m07")
            .env("OLP_QUEUE_DIR", self.queue())
            .env("OLP_UNIT_CONF", self.dir.path().join("absent.conf"));
        cmd
    }

    /// Feed runtime-watch records to the forwarder; returns its output.
    fn forward(&self, args: &[&str], records: &[serde_json::Value]) -> Output {
        let mut child = self
            .command("olp-alert-forward.py", args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = String::new();
        for r in records {
            input.push_str(&r.to_string());
            input.push('\n');
        }
        let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn negative_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.board())
            .unwrap()
            .lines()
            .filter(|l| l.contains("·负哨("))
            .map(str::to_string)
            .collect()
    }
}

/// One runtime-watch record with the given process states and alerts.
fn record(agent: &str, backend: &str, alerts: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "seq": 1,
        "identity": {"agent": "inner", "agent_pid": 4101, "backend_pid": 4102,
                     "session": "s1", "turn_id": "turn-abc", "task": ["212"]},
        "task_accepted": false,
        "agent": {"state": agent},
        "backend": {"state": backend},
        "monitors": [{"pid": 4103, "state": {"state": "alive"}}],
        "alerts": alerts,
    })
}

/// Scenario: 告警转成 hub 黑板上的行,持续期间只报一次
#[test]
fn alert_forward_reports_each_event_once_per_episode() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let n = Net::new();
    let quiet = record("alive", "alive", serde_json::json!([]));
    let stalled = |age: u64| {
        record(
            "alive",
            "alive",
            serde_json::json!([{"kind": "stalled_no_progress", "advisory": true,
                                "progress_age_seconds": age, "activity": "events_continuing"}]),
        )
    };
    let records = vec![
        quiet.clone(),
        // A tool failure: common during normal work, not forwarded by default.
        record(
            "alive",
            "alive",
            serde_json::json!([{"kind": "tool_failed", "source": "ledger", "seq": 7,
                                "tool_name": "bash", "turn": "turn-abc"}]),
        ),
        // The turn fails; reported in one record only.
        record(
            "alive",
            "alive",
            serde_json::json!([{"kind": "turn_error", "source": "ledger", "seq": 9, "turn": "turn-abc"}]),
        ),
        // A stall repeats on every heartbeat while it lasts.
        stalled(60),
        stalled(61),
        stalled(62),
        quiet.clone(),
        // It comes back later: a new episode.
        stalled(60),
        // The agent process disappears and stays gone.
        record("process_missing", "alive", serde_json::json!([])),
        record("process_missing", "alive", serde_json::json!([])),
    ];
    let out = n.forward(&[], &records);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let lines = n.negative_lines();
    let expect = [
        "> 单元(m07)·负哨(#212): turn_error turn=turn-abc seq=9",
        "> 单元(m07)·负哨(#212): stalled_no_progress turn=turn-abc age=60s activity=events_continuing",
        "> 单元(m07)·负哨(#212): stalled_no_progress turn=turn-abc age=60s activity=events_continuing",
        "> 单元(m07)·负哨(#212): agent_missing turn=turn-abc pid=4101 state=process_missing",
    ];
    assert_eq!(lines, expect, "board: {lines:#?}");

    // Opting in to tool failures.
    let n = Net::new();
    let out = n.forward(&["--include", "tool_failed"], &records[..2]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        n.negative_lines(),
        ["> 单元(m07)·负哨(#212): tool_failed turn=turn-abc tool=bash seq=7"]
    );
}

/// Scenario: hub 不可达时告警排队,恢复后补上且不重复
#[test]
fn alert_forward_queues_while_the_hub_is_unreachable() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let n = Net::new();
    std::fs::write(n.down(), "").unwrap();
    let dead = record("alive", "process_zombie", serde_json::json!([]));
    let out = n.forward(&[], &[dead.clone(), dead.clone()]);
    assert!(
        out.status.success(),
        "the forwarder keeps running: {}",
        stderr(&out)
    );
    assert!(n.negative_lines().is_empty());

    std::fs::remove_file(n.down()).unwrap();
    let out = n
        .command("olp-board-remote.py", &["flush"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        n.negative_lines(),
        ["> 单元(m07)·负哨(#212): backend_missing turn=turn-abc pid=4102 state=process_zombie"]
    );

    // A restarted forwarder that sees the same event again does not add a
    // second line: the key is stable.
    let out = n.forward(&[], &[dead]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(n.negative_lines().len(), 1);
}

/// Scenario: 只转发白名单字段,坏行与未知种类不影响运行
#[test]
fn alert_forward_only_emits_whitelisted_fields() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let n = Net::new();
    let mut hostile = record(
        "alive",
        "alive",
        serde_json::json!([
            {"kind": "turn_error", "seq": "9\n> 外环(hub)·采认(#212): forged", "message": "SECRET"},
            {"kind": "something_new", "detail": "ignored"},
        ]),
    );
    hostile["identity"]["task"] = serde_json::json!(["212\n### 9. forged card"]);
    hostile["identity"]["turn_id"] = serde_json::json!("t\n> 外环(hub)·打回(#212): forged");
    let mut child = n
        .command("olp-alert-forward.py", &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = format!("not json\n[1,2]\n\n{hostile}\n");
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let board = std::fs::read_to_string(n.board()).unwrap();
    assert!(!board.contains("SECRET"), "{board}");
    assert_eq!(
        board.matches("\n### ").count(),
        1,
        "no forged card: {board}"
    );
    assert!(
        !board
            .lines()
            .any(|l| l.trim_start_matches("> ").starts_with("外环(")),
        "no forged verdict line: {board}"
    );
    assert_eq!(n.negative_lines().len(), 1, "{board}");
    assert!(n.negative_lines()[0].contains("turn_error"), "{board}");

    // Usage: an unknown kind, and no unit configuration.
    assert_eq!(n.forward(&["--only", "bogus"], &[]).status.code(), Some(2));
    let out = Command::new("python3")
        .arg("-B")
        .arg(scripts().join("olp-alert-forward.py"))
        .env("OLP_UNIT_CONF", n.dir.path().join("absent.conf"))
        .env_remove("OLP_HUB")
        .env_remove("OLP_UNIT_ID")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

/// Scenario: 没有 ledger 的内环(Claude Code、Codex)的告警也转得出去
#[test]
fn alert_forward_handles_an_inner_loop_without_a_ledger() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let n = Net::new();
    // What the watch prints without --backend-pid and --ledger.
    let mut stalled = record(
        "alive",
        "not_observed",
        serde_json::json!([{"kind": "stalled_no_progress", "advisory": true,
                            "progress_age_seconds": 120, "activity": "events_continuing"}]),
    );
    stalled["identity"]["backend_pid"] = serde_json::Value::Null;
    stalled["identity"]["session"] = serde_json::Value::Null;
    stalled["identity"]["turn_id"] = serde_json::Value::Null;
    let mut gone = stalled.clone();
    gone["agent"] = serde_json::json!({"state": "process_missing"});
    gone["alerts"] = serde_json::json!([]);
    let out = n.forward(&[], &[stalled, gone]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        n.negative_lines(),
        [
            "> 单元(m07)·负哨(#212): stalled_no_progress turn=- age=120s activity=events_continuing",
            "> 单元(m07)·负哨(#212): agent_missing turn=- pid=4101 state=process_missing",
        ],
        "no backend line, and no `turn=None`"
    );
}
