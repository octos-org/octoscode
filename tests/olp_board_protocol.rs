#![cfg(unix)]

//! Real CLI-path tests for the opt-in `olp-board/v1` extension.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script(name: &str) -> PathBuf {
    repo().join("scripts").join(name)
}

struct Sandbox {
    root: PathBuf,
    board: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "olp-board-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let board = root.join("BOARD.md");
        fs::write(&board, b"").unwrap();
        fs::write(root.join("BOARD.md.lock"), b"").unwrap();
        Self { root, board }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn event(&self, args: &[&str]) -> Output {
        let mut command = Command::new("python3");
        command.arg("-B").arg(script("olp-board-event.py"));
        command.args(args).arg("--board").arg(&self.board);
        command.output().unwrap()
    }

    fn state_output(&self) -> Output {
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["state", "--board"])
            .arg(&self.board)
            .output()
            .unwrap()
    }

    fn state(&self) -> Value {
        success_json(self.state_output())
    }

    fn item(&self, number: &str, title: &str, actor: &str, to: &str) -> Value {
        let body = self.file(&format!("item-{number}.txt"), b"Executable work item.\n");
        success_json(
            Command::new("python3")
                .arg("-B")
                .arg(script("olp-board-event.py"))
                .args(["item", "--board"])
                .arg(&self.board)
                .args([
                    "--actor", actor, "--number", number, "--title", title, "--to", to,
                ])
                .arg("--body-file")
                .arg(body)
                .output()
                .unwrap(),
        )
    }

    fn receive(&self, actor: &str, item: &str) -> Output {
        self.event(&["receive", "--actor", actor, "--item", item])
    }

    fn ack(&self, actor: &str, item: &str, outcome: &str) -> Output {
        let body = self.file("ack.txt", b"Completed through the real CLI");
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["ack", "--board"])
            .arg(&self.board)
            .args([
                "--actor",
                actor,
                "--item",
                item,
                "--outcome",
                outcome,
                "--r2",
                "verified",
            ])
            .arg("--commit")
            .arg("0123456789abcdef0123456789abcdef01234567")
            .arg("--body-file")
            .arg(body)
            .output()
            .unwrap()
    }

    fn review(&self, actor: &str, ack: &str, decision: &str) -> Output {
        let body = self.file("review.txt", b"Reviewed against the declared item");
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["review", "--board"])
            .arg(&self.board)
            .args(["--actor", actor, "--ack", ack, "--decision", decision])
            .arg("--body-file")
            .arg(body)
            .output()
            .unwrap()
    }

    fn shell_append(&self, bytes: &[u8]) -> Output {
        let mut child = Command::new("bash")
            .arg(script("olp-board-append.sh"))
            .arg(&self.board)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn success_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn failed(output: Output, needle: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(needle),
        "expected={needle} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_for_file(path: &Path) {
    for _ in 0..1000 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {}", path.display());
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn source_ref(offset: usize, bytes: &[u8]) -> Value {
    json!({"offset": offset, "length": bytes.len(), "sha256": sha256_hex(bytes)})
}

/// PATH with no-op `octoscode` and `octos` stubs in front, so the dependency
/// check in `olp-init.sh` never depends on whether the machine running the
/// tests has them installed (CI does not install them).
fn path_with_octoscode_stubs(root: &Path) -> std::ffi::OsString {
    let bin = root.join("octoscode-stubs");
    fs::create_dir_all(&bin).unwrap();
    for name in ["octoscode", "octos"] {
        let path = bin.join(name);
        fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        fs::set_permissions(&path, permissions).unwrap();
    }
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited))).unwrap()
}

fn tree_contains_file_named(root: &PathBuf, name: &str) -> bool {
    if !root.exists() {
        return false;
    }
    fs::read_dir(root).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name().and_then(|value| value.to_str()) == Some(name)
            || (path.is_dir() && tree_contains_file_named(&path, name))
    })
}

fn replace_last_event_recovery(board: &PathBuf, recovery: &Value) {
    let text = fs::read_to_string(board).unwrap();
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    let index = lines
        .iter()
        .rposition(|line| line.starts_with("> OLP-EVENT "))
        .unwrap();
    let mut event: Value = serde_json::from_str(
        lines[index]
            .strip_prefix("> OLP-EVENT ")
            .unwrap()
            .trim_end_matches('\n'),
    )
    .unwrap();
    event["recovery"] = recovery.clone();
    lines[index] = format!("> OLP-EVENT {}\n", serde_json::to_string(&event).unwrap());
    fs::write(board, lines.concat()).unwrap();
}

/// Test Path Statement: Real-path regression; production entrypoint is each
/// event CLI and both inbox roles; only the temporary filesystem is isolated.
#[test]
fn olp_board_lifecycle_projects_actionable_states() {
    let sb = Sandbox::new("lifecycle");
    assert_eq!(sb.state()["mode"], "legacy");
    let item = sb.item("A-1", "Publish result", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    assert!(
        fs::read(&sb.board)
            .unwrap()
            .windows(b"> OLP-EVENT ".len())
            .any(|window| window == b"> OLP-EVENT ")
    );
    let state = sb.state();
    assert_eq!(state["mode"], "structured");
    assert_eq!(state["unreceived"].as_array().unwrap().len(), 1);

    let inbox = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .args(["--board"])
            .arg(&sb.board)
            .args(["--for", "runtime"])
            .output()
            .unwrap(),
    );
    assert_eq!(inbox["matched"], true);

    success_json(sb.receive("runtime", item_id));
    assert_eq!(sb.state()["received_pending"].as_array().unwrap().len(), 1);
    let pending = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .args(["--board"])
            .arg(&sb.board)
            .args(["--for", "runtime", "--actor", "runtime"])
            .output()
            .unwrap(),
    );
    assert!(pending["unreceived"].as_array().unwrap().is_empty());
    assert_eq!(pending["received_pending"].as_array().unwrap().len(), 1);
    assert_eq!(pending["messages"].as_array().unwrap().len(), 1);
    let ack = success_json(sb.ack("runtime", item_id, "done"));
    let ack_id = ack["event"].as_str().unwrap();
    assert_eq!(sb.state()["unreviewed_ack"].as_array().unwrap().len(), 1);

    let outer = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .args(["--board"])
            .arg(&sb.board)
            .args(["--for", "outer", "--actor", "outer"])
            .output()
            .unwrap(),
    );
    assert_eq!(outer["matched"], true);
    success_json(sb.review("outer", ack_id, "accept"));
    let state = sb.state();
    assert!(state["unreceived"].as_array().unwrap().is_empty());
    assert!(state["received_pending"].as_array().unwrap().is_empty());
    assert!(state["unreviewed_ack"].as_array().unwrap().is_empty());
    assert!(state["escalated"].as_array().unwrap().is_empty());

    let receipt = sb.file("receipt.json", serde_json::to_vec(&ack).unwrap().as_slice());
    let verified = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["verify", "--receipt-file"])
            .arg(receipt)
            .output()
            .unwrap(),
    );
    assert_eq!(verified["verified"], true);
}

/// Test Path Statement: Real CLI restart recovery through runtime actor
/// filtering; received work remains visible but never authorizes dispatch.
#[test]
fn olp_board_runtime_inbox_filters_and_recovers_received_pending() {
    let sb = Sandbox::new("runtime-pending");
    let item = sb.item("R3", "Resume safely", "outer", "worker");
    let item_id = item["event"].as_str().unwrap();
    let query = |actor: &str| {
        success_json(
            Command::new("python3")
                .arg("-B")
                .arg(script("olp-board-inbox.py"))
                .args(["--board"])
                .arg(&sb.board)
                .args(["--for", "runtime", "--actor", actor])
                .output()
                .unwrap(),
        )
    };
    assert_eq!(query("runtime")["matched"], false);
    assert_eq!(query("worker")["unreceived"].as_array().unwrap().len(), 1);
    success_json(sb.receive("worker", item_id));
    for _ in 0..2 {
        let restarted = query("worker");
        assert_eq!(restarted["unreceived"].as_array().unwrap().len(), 0);
        assert_eq!(restarted["received_pending"].as_array().unwrap().len(), 1);
        assert_eq!(restarted["messages"][0]["id"], item_id);
        assert_eq!(restarted["execution_authorized"], false);
    }
}

/// Test Path Statement: Real-path regression through the state transition
/// guard; no decision is mocked and rejected commands must leave the head stable.
#[test]
fn olp_board_rejects_invalid_or_repeated_transitions() {
    let sb = Sandbox::new("transitions");
    let item = sb.item("B", "Guard transitions", "author", "worker");
    let item_id = item["event"].as_str().unwrap();
    let head = sb.state()["head"].clone();
    failed(sb.receive("intruder", item_id), "Only the item recipient");
    assert_eq!(sb.state()["head"], head);
    success_json(sb.receive("worker", item_id));
    failed(sb.receive("worker", item_id), "already received");
    failed(
        sb.ack("intruder", item_id, "done"),
        "Only the item recipient",
    );
    let ack = success_json(sb.ack("worker", item_id, "done"));
    failed(sb.ack("worker", item_id, "done"), "terminal ACK");
    failed(
        sb.review("intruder", ack["event"].as_str().unwrap(), "accept"),
        "Only the item author",
    );

    let other = Sandbox::new("wontdo");
    let item = other.item("C", "Dispute", "author", "worker");
    let item_id = item["event"].as_str().unwrap();
    success_json(other.receive("worker", item_id));
    let ack = success_json(other.ack("worker", item_id, "wontdo"));
    failed(
        other.review("author", ack["event"].as_str().unwrap(), "return"),
        "wontdo cannot be returned",
    );
}

/// Test Path Statement: Adapter boundary through strict JSON file loading and
/// the generic record CLI; malformed inputs never reach the append effect.
#[test]
fn olp_board_record_is_strict_and_cannot_bypass_rendering() {
    let sb = Sandbox::new("record");
    let item = sb.item("D", "Record strictness", "author", "worker");
    let item_id = item["event"].as_str().unwrap();
    let head = sb.state()["head"].as_str().unwrap().to_string();
    let receive_body = sb.file(
        "receive.txt",
        format!("RECEIVE(item={item_id}, actor=worker)\n").as_bytes(),
    );
    let event_file = sb.file(
        "receive.json",
        serde_json::to_vec(&json!({
            "prev": head, "type": "receive", "actor": "worker", "item": item_id
        }))
        .unwrap()
        .as_slice(),
    );
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["record", "--board"])
            .arg(&sb.board)
            .arg("--event-file")
            .arg(event_file)
            .arg("--body-file")
            .arg(receive_body)
            .output()
            .unwrap(),
    );

    let duplicate = sb.file(
        "duplicate.json",
        br#"{"prev":null,"type":"item","actor":"a","number":"1","number":"2","title":"x","to":"runtime"}"#,
    );
    let body = sb.file("body.txt", b"### 1. x\nbody\n");
    failed(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["record", "--board"])
            .arg(&sb.board)
            .arg("--event-file")
            .arg(duplicate)
            .arg("--body-file")
            .arg(&body)
            .output()
            .unwrap(),
        "Duplicate JSON key",
    );
    let nonfinite = sb.file(
        "nan.json",
        br#"{"prev":NaN,"type":"item","actor":"a","number":"1","title":"x","to":"runtime"}"#,
    );
    failed(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["record", "--board"])
            .arg(&sb.board)
            .arg("--event-file")
            .arg(nonfinite)
            .arg("--body-file")
            .arg(body)
            .output()
            .unwrap(),
        "Non-finite JSON number",
    );
}

/// Test Path Statement: Adapter boundary against immutable byte evidence;
/// corruption is applied only to a disposable copy of the real board.
#[test]
fn olp_board_replay_quarantines_tampered_source_chain_and_timestamp() {
    for (tag, needle) in [
        ("source", "source bytes changed"),
        ("prev", "Broken prev chain"),
        ("timestamp", "timestamp boundary mismatch"),
        ("bool", "Invalid source offset"),
    ] {
        let sb = Sandbox::new(tag);
        sb.item("E", "Tamper evidence", "author", "runtime");
        let mut bytes = fs::read(&sb.board).unwrap();
        match tag {
            "source" => bytes[5] ^= 1,
            "prev" => {
                let needle = b"\"prev\":null";
                let at = bytes
                    .windows(needle.len())
                    .position(|w| w == needle)
                    .unwrap();
                bytes[at + 7..at + 11].copy_from_slice(b"true");
            }
            _ => {
                if tag == "timestamp" {
                    let at = bytes.windows(3).rposition(|w| w == b"ts=").unwrap();
                    bytes[at + 3] = b'1';
                } else {
                    let needle = b"\"offset\":0";
                    let at = bytes
                        .windows(needle.len())
                        .position(|w| w == needle)
                        .unwrap();
                    bytes.splice(at..at + needle.len(), b"\"offset\":true".iter().copied());
                }
            }
        }
        fs::write(&sb.board, bytes).unwrap();
        assert_quarantined(&sb, needle);
    }
}

/// Replay must keep working and report the damaged event line as blocking
/// byte evidence instead of silently accepting it or failing outright.
fn assert_quarantined(sb: &Sandbox, needle: &str) {
    let state = sb.state();
    assert_eq!(state["mode"], "mixed", "{state}");
    assert_eq!(state["dispatch_blocked"], true);
    assert!(state["events"].as_array().unwrap().is_empty(), "{state}");
    let drift = state["drift"].as_array().unwrap();
    let entry = drift
        .iter()
        .find(|entry| entry["kind"] == "malformed_event")
        .unwrap_or_else(|| panic!("no malformed_event in {state}"));
    assert!(
        entry["reason"].as_str().unwrap().contains(needle),
        "expected={needle} drift={entry}"
    );
    let bytes = fs::read(&sb.board).unwrap();
    let offset = entry["offset"].as_u64().unwrap() as usize;
    let length = entry["length"].as_u64().unwrap() as usize;
    assert!(bytes[offset..].starts_with(b"> OLP-EVENT "));
    assert_eq!(entry["sha256"], sha256_hex(&bytes[offset..offset + length]));
}

/// Test Path Statement: Real-path replay of reserved ledger records at EOF;
/// corrupt terminal boundaries stay blocking DRIFT even if later bytes are
/// appended, and never make replay itself fail.
#[test]
fn olp_board_replay_quarantines_incomplete_reserved_records() {
    for tag in ["missing", "partial", "bad-terminal", "later-append"] {
        let sb = Sandbox::new(tag);
        if tag == "bad-terminal" {
            fs::write(&sb.board, b"> OLP-EVENT {bad json}\n").unwrap();
            assert_quarantined(&sb, "Invalid JSON");
            continue;
        }
        sb.item("R1", "Boundary", "outer", "runtime");
        let bytes = fs::read(&sb.board).unwrap();
        let ts = bytes
            .windows(3)
            .rposition(|window| window == b"ts=")
            .unwrap();
        let mut damaged = bytes[..ts].to_vec();
        if tag == "partial" {
            damaged.extend_from_slice(b"ts=2026-09");
        } else if tag == "later-append" {
            damaged.extend_from_slice(b"later text\nts=2026-09-27T00:00:00Z\n");
        }
        fs::write(&sb.board, damaged).unwrap();
        assert_quarantined(&sb, "timestamp boundary mismatch");
    }
}

/// Test Path Statement: Real-path regression through legacy append plus
/// structured replay; old history is exempt while new unpaired ACK is DRIFT.
#[test]
fn olp_board_mixed_mode_reports_only_post_opt_in_drift() {
    let sb = Sandbox::new("drift");
    assert!(
        sb.shell_append(b"### old. history\nACK(blocked): legacy record\n")
            .status
            .success()
    );
    sb.item("F", "Enable structure", "outer", "runtime");
    let paired_body = sb.file(
        "paired-examples.txt",
        b"ACK(blocked): paired body example\n### body. paired heading example\n",
    );
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["item", "--board"])
            .arg(&sb.board)
            .args([
                "--actor",
                "outer",
                "--number",
                "F2",
                "--title",
                "Paired examples",
            ])
            .arg("--body-file")
            .arg(paired_body)
            .output()
            .unwrap(),
    );
    assert!(sb
        .shell_append(
            b"```text\n### example. fenced item\nACK(blocked): fenced example\n```\n> ### quoted. item\n> > OLP-EVENT {quoted example}\n",
        )
        .status
        .success());
    let state = sb.state();
    assert_eq!(state["mode"], "structured");
    assert!(state["drift"].as_array().unwrap().is_empty());
    assert!(
        sb.shell_append(b"### 2. Unpaired item\nDo work.\nACK(blocked): unpaired new record\n> ACK(blocked): quoted hand-written record\n### 3. CRLF item\r\n")
            .status
            .success()
    );
    let state = sb.state();
    assert_eq!(state["mode"], "mixed");
    let kinds: Vec<&str> = state["drift"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "unpaired_item",
            "unpaired_ack",
            "suspected_ack",
            "unpaired_item"
        ]
    );
    assert_eq!(state["dispatch_blocked"], true);
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state/write, inbox, sentinel, legacy append, and harvest CLIs.
/// - Production path: the shared fence projection blocks every consumer, then a matching close restores writes.
/// - External edges faked: temporary boards, harvest state, and short sentinel polling only.
/// - What this proves: backtick and tilde fences require a matching kind and sufficient length, expose opener bytes, and never silently advance harvest.
/// - What this intentionally does not exercise: recovery of normative text inside a closed example fence, which remains forbidden by the existing recovery test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_unclosed_fence_blocks_consumers_and_matching_close_recovers
#[test]
fn olp_board_unclosed_fence_blocks_consumers_and_matching_close_recovers() {
    for (tag, opener, wrong_kind, too_short, matching_close) in [
        (
            "backtick",
            b"````text\n".as_slice(),
            b"~~~~\n".as_slice(),
            b"```\n".as_slice(),
            b"````\n".as_slice(),
        ),
        (
            "tilde",
            b"~~~~text\n".as_slice(),
            b"````\n".as_slice(),
            b"~~~\n".as_slice(),
            b"~~~~\n".as_slice(),
        ),
    ] {
        let sb = Sandbox::new(&format!("unclosed-{tag}"));
        sb.item("seed", "Enable structure", "outer", "runtime");
        let opener_offset = fs::metadata(&sb.board).unwrap().len() as usize;
        let mut hidden = opener.to_vec();
        hidden.extend_from_slice(b"ACK(blocked): hidden by the unclosed fence\n");
        hidden.extend_from_slice(wrong_kind);
        hidden.extend_from_slice(too_short);
        assert!(sb.shell_append(&hidden).status.success());

        let state = sb.state();
        assert_eq!(state["mode"], "mixed");
        assert_eq!(state["dispatch_blocked"], true);
        let drift = state["drift"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["reason"] == "unclosed fence after structured opt-in")
            .unwrap();
        assert_eq!(drift["offset"], opener_offset);
        assert_eq!(drift["length"], opener.len());
        assert_eq!(drift["sha256"], sha256_hex(opener));
        assert_eq!(state["fenced_ranges"][0]["closed"], false);

        let inbox = success_json(
            Command::new("python3")
                .arg("-B")
                .arg(script("olp-board-inbox.py"))
                .args(["--board"])
                .arg(&sb.board)
                .args(["--for", "runtime", "--actor", "runtime"])
                .output()
                .unwrap(),
        );
        assert_eq!(inbox["dispatch_blocked"], true);

        let sentinel = Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-sentinel.py"))
            .args(["--board"])
            .arg(&sb.board)
            .args(["--token", "unused", "--for", "runtime"])
            .output()
            .unwrap();
        assert!(sentinel.status.success());
        assert!(String::from_utf8_lossy(&sentinel.stdout).starts_with("DRIFT: "));

        if tag == "backtick" {
            let repo_root = sb.root.join("repo");
            fs::create_dir(&repo_root).unwrap();
            let state_root = sb.root.join("harvest-state");
            let evolution = sb.root.join("EVOLUTION.md");
            let harvest = Command::new("bash")
                .arg(script("olp-evo-harvest.sh"))
                .arg(&repo_root)
                .env("OLP_EVO_REVIEW_BOARD", &sb.board)
                .env("OLP_EVO_MCP_BOARD", sb.root.join("missing-mcp.md"))
                .env("OLP_EVO_STATE", &state_root)
                .env("OLP_EVO_BOARD", &evolution)
                .env("OLP_BOARD_EVENT_TOOL", script("olp-board-event.py"))
                .output()
                .unwrap();
            assert_eq!(harvest.status.code(), Some(1));
            assert!(
                String::from_utf8_lossy(&harvest.stderr)
                    .contains("unclosed fence after structured opt-in")
            );
            assert!(!tree_contains_file_named(&state_root, "state.json"));
            assert!(!evolution.exists());
        }

        assert!(sb.shell_append(matching_close).status.success());
        let restored = sb.state();
        assert_eq!(restored["mode"], "structured");
        assert_eq!(restored["dispatch_blocked"], false);
        assert!(restored["drift"].as_array().unwrap().is_empty());
        assert_eq!(restored["fenced_ranges"][0]["closed"], true);
        sb.item("after", "Writes resume", "outer", "runtime");
        if tag == "backtick" {
            let harvest = Command::new("bash")
                .arg(script("olp-evo-harvest.sh"))
                .arg(sb.root.join("repo"))
                .arg("--dry-run")
                .env("OLP_EVO_REVIEW_BOARD", &sb.board)
                .env("OLP_EVO_MCP_BOARD", sb.root.join("missing-mcp.md"))
                .env("OLP_EVO_STATE", sb.root.join("harvest-state"))
                .env("OLP_EVO_BOARD", sb.root.join("EVOLUTION.md"))
                .env("OLP_BOARD_EVENT_TOOL", script("olp-board-event.py"))
                .output()
                .unwrap();
            assert!(
                harvest.status.success(),
                "stderr={}",
                String::from_utf8_lossy(&harvest.stderr)
            );
        }
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: runtime inbox CLI.
/// - Production path: event insertion order flows unchanged into the runtime projection.
/// - External edges faked: temporary board files only.
/// - What this proves: arbitrary display numbers never reorder dispatch.
/// - What this intentionally does not exercise: receive reconciliation, covered by the actor-specific inbox test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_inbox_preserves_ledger_order_for_arbitrary_numbers
#[test]
fn olp_board_inbox_preserves_ledger_order_for_arbitrary_numbers() {
    let sb = Sandbox::new("inbox-ledger-order");
    for number in ["4N", "2", "A"] {
        sb.item(number, "Ledger ordered", "outer", "runtime");
    }
    let inbox = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .args(["--board"])
            .arg(&sb.board)
            .args(["--for", "runtime", "--actor", "runtime"])
            .output()
            .unwrap(),
    );
    let numbers: Vec<&str> = inbox["unreceived"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["number"].as_str().unwrap())
        .collect();
    assert_eq!(numbers, vec!["4N", "2", "A"]);
}

/// Test Path Statement: Real event, state and outer inbox CLIs with only
/// temporary files isolated. Concurrent completion order must determine ACK
/// and escalation queues, including their combined --since-head projection.
#[test]
fn olp_board_outer_queues_follow_ack_and_review_event_order() {
    let sb = Sandbox::new("outer-event-order");
    let items: Vec<Value> = ["A", "B", "C", "D"]
        .into_iter()
        .map(|number| sb.item(number, "Concurrent work", "outer", "runtime"))
        .collect();
    for item in &items {
        success_json(sb.receive("runtime", item["event"].as_str().unwrap()));
    }
    let acks: Vec<Value> = [1, 0, 2]
        .into_iter()
        .map(|index| {
            success_json(sb.ack(
                "runtime",
                items[index]["event"].as_str().unwrap(),
                "blocked",
            ))
        })
        .collect();
    let ids = |entries: &Value| -> Vec<String> {
        entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let event_id = |receipt: &Value| receipt["event"].as_str().unwrap().to_owned();
    assert_eq!(
        ids(&sb.state()["unreviewed_ack"]),
        acks.iter().map(event_id).collect::<Vec<_>>()
    );

    let reviews: Vec<Value> = [2, 0]
        .into_iter()
        .map(|index| {
            success_json(sb.review("outer", acks[index]["event"].as_str().unwrap(), "escalate"))
        })
        .collect();
    let last_ack = success_json(sb.ack("runtime", items[3]["event"].as_str().unwrap(), "blocked"));
    let state = sb.state();
    assert_eq!(
        ids(&state["escalated"]),
        reviews.iter().map(event_id).collect::<Vec<_>>()
    );
    assert_eq!(
        ids(&state["unreviewed_ack"]),
        vec![event_id(&acks[1]), event_id(&last_ack)]
    );
    for since in [None, Some(event_id(&reviews[0]))] {
        let mut command = Command::new("python3");
        command
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .arg("--board")
            .arg(&sb.board)
            .args(["--for", "outer", "--actor", "outer"]);
        if let Some(head) = &since {
            command.args(["--since-head", head]);
        }
        let inbox = success_json(command.output().unwrap());
        let expected = if since.is_some() {
            vec![event_id(&reviews[1]), event_id(&last_ack)]
        } else {
            vec![
                event_id(&acks[1]),
                event_id(&reviews[0]),
                event_id(&reviews[1]),
                event_id(&last_ack),
            ]
        };
        assert_eq!(ids(&inbox["messages"]), expected);
    }
}

/// Test Path Statement: Real item and generic record/ACK CLI paths recover
/// exact post-opt-in text while preserving it as auditable byte evidence.
#[test]
fn olp_board_recovery_clears_item_and_ack_drift_with_audit_evidence() {
    let sb = Sandbox::new("recovery-success");
    let item = sb.item("R8-A", "ACK target", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    success_json(sb.receive("runtime", item_id));

    let ack_line = b"ACK(done): completed by an older writer\n";
    let ack_offset = fs::metadata(&sb.board).unwrap().len() as usize;
    assert!(sb.shell_append(ack_line).status.success());
    let ack_recovery = sb.file(
        "ack-recovery.json",
        &serde_json::to_vec(&source_ref(ack_offset, ack_line)).unwrap(),
    );
    let explanation = sb.file("recovery-ack.txt", b"Backfilled exact old ACK");
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["ack", "--board"])
            .arg(&sb.board)
            .args([
                "--actor",
                "runtime",
                "--item",
                item_id,
                "--outcome",
                "done",
                "--r2",
                "unverified",
                "--body-file",
            ])
            .arg(&explanation)
            .arg("--recovery-file")
            .arg(&ack_recovery)
            .output()
            .unwrap(),
    );

    let item_line = b"### R8-I. Imported item\n";
    let item_offset = fs::metadata(&sb.board).unwrap().len() as usize;
    assert!(sb.shell_append(item_line).status.success());
    let item_recovery = source_ref(item_offset, item_line);
    let body = sb.file(
        "record-item.txt",
        b"### R8-I. Imported item\nRecovered item body.\n",
    );
    let event_file = sb.file(
        "record-item.json",
        &serde_json::to_vec(&json!({
            "prev": sb.state()["head"], "type": "item", "actor": "outer",
            "number": "R8-I", "title": "Imported item", "to": "runtime",
            "recovery": item_recovery,
        }))
        .unwrap(),
    );
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["record", "--board"])
            .arg(&sb.board)
            .arg("--event-file")
            .arg(event_file)
            .arg("--body-file")
            .arg(body)
            .output()
            .unwrap(),
    );
    let state = sb.state();
    assert!(state["drift"].as_array().unwrap().is_empty());
    assert_eq!(state["recovery_evidence"].as_array().unwrap().len(), 2);
    let bytes = fs::read(&sb.board).unwrap();
    assert_eq!(&bytes[ack_offset..ack_offset + ack_line.len()], ack_line);
    assert_eq!(
        &bytes[item_offset..item_offset + item_line.len()],
        item_line
    );
}

/// Test Path Statement: Real recovery validation rejects reuse and semantic
/// mismatch before append, leaving both ledger head and bytes unchanged.
#[test]
fn olp_board_recovery_rejects_duplicate_and_wrong_semantics_without_writes() {
    let sb = Sandbox::new("recovery-reject");
    let first = sb.item("R8-1", "First", "outer", "runtime");
    let first_id = first["event"].as_str().unwrap();
    success_json(sb.receive("runtime", first_id));
    let line = b"ACK(done): original completion\n";
    let offset = fs::metadata(&sb.board).unwrap().len() as usize;
    assert!(sb.shell_append(line).status.success());
    let recovery = sb.file(
        "used.json",
        &serde_json::to_vec(&source_ref(offset, line)).unwrap(),
    );
    let explanation = sb.file("ack-recovery.txt", b"Recover completion");
    let ack_with = |item: &str, outcome: &str, recovery_file: &PathBuf| {
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["ack", "--board"])
            .arg(&sb.board)
            .args([
                "--actor",
                "runtime",
                "--item",
                item,
                "--outcome",
                outcome,
                "--r2",
                "unverified",
                "--body-file",
            ])
            .arg(&explanation)
            .arg("--recovery-file")
            .arg(recovery_file)
            .output()
            .unwrap()
    };
    success_json(ack_with(first_id, "done", &recovery));

    let second = sb.item("R8-2", "Second", "outer", "runtime");
    let second_id = second["event"].as_str().unwrap();
    success_json(sb.receive("runtime", second_id));
    let before = fs::read(&sb.board).unwrap();
    failed(ack_with(second_id, "done", &recovery), "already consumed");
    assert_eq!(fs::read(&sb.board).unwrap(), before);

    let blocked_line = b"ACK(blocked): older writer blocked\n";
    let blocked_offset = fs::metadata(&sb.board).unwrap().len() as usize;
    assert!(sb.shell_append(blocked_line).status.success());
    let wrong = sb.file(
        "wrong-outcome.json",
        &serde_json::to_vec(&source_ref(blocked_offset, blocked_line)).unwrap(),
    );
    let before = fs::read(&sb.board).unwrap();
    failed(
        ack_with(second_id, "done", &wrong),
        "outcome does not match",
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);
}

/// Test Path Statement: Recovery evidence is restricted to exact, unpaired,
/// post-opt-in lines; invalid range/hash/history references have zero effects.
#[test]
fn olp_board_recovery_rejects_pre_opt_in_bad_hash_and_paired_sources() {
    let sb = Sandbox::new("recovery-evidence");
    let old_line = b"### OLD. Before opt in\n";
    assert!(sb.shell_append(old_line).status.success());
    let seed = sb.item("SEED", "Enable structure", "outer", "runtime");
    let paired_receive = success_json(sb.receive("runtime", seed["event"].as_str().unwrap()));

    let try_item = |name: &str, number: &str, title: &str, evidence: Value| {
        let recovery = sb.file(
            &format!("{name}-recovery.json"),
            &serde_json::to_vec(&evidence).unwrap(),
        );
        let body = sb.file(&format!("{name}-body.txt"), b"Recovered body.\n");
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["item", "--board"])
            .arg(&sb.board)
            .args(["--actor", "outer", "--number", number, "--title", title])
            .arg("--body-file")
            .arg(body)
            .arg("--recovery-file")
            .arg(recovery)
            .output()
            .unwrap()
    };

    let before = fs::read(&sb.board).unwrap();
    failed(
        try_item("old", "OLD", "Before opt in", source_ref(0, old_line)),
        "after structured opt-in",
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);

    let new_line = b"### NEW. Exact evidence\n";
    let new_offset = fs::metadata(&sb.board).unwrap().len() as usize;
    assert!(sb.shell_append(new_line).status.success());
    let mut bad_hash = source_ref(new_offset, new_line);
    bad_hash["sha256"] = Value::String("0".repeat(64));
    let before = fs::read(&sb.board).unwrap();
    failed(
        try_item("hash", "NEW", "Exact evidence", bad_hash),
        "bytes changed",
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);

    let bad_range = source_ref(new_offset + 1, new_line);
    let before = fs::read(&sb.board).unwrap();
    failed(
        try_item("range", "NEW", "Exact evidence", bad_range),
        "before the recovering event",
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);

    let paired = paired_receive["source"].clone();
    let before = fs::read(&sb.board).unwrap();
    failed(
        try_item("paired", "SEED", "Enable structure", paired),
        "already paired",
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: item/ack/state event CLIs.
/// - Production path: record validation and replay use the shared normative-line projection.
/// - External edges faked: temporary board files only.
/// - What this proves: fenced item/ACK examples and quoted item headings cannot be recovered, and a replayed event whose recovery points at them is quarantined.
/// - What this intentionally does not exercise: quoted ACK lines, which are suspected hand-written ACKs covered by the ACK-variant test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_recovery_rejects_non_normative_examples
#[test]
fn olp_board_recovery_rejects_non_normative_examples() {
    for (kind, context) in [("item", "fenced"), ("item", "quoted"), ("ack", "fenced")] {
        {
            let sb = Sandbox::new(&format!("recovery-{kind}-{context}"));
            let seed = sb.item("SEED", "Enable structure", "outer", "runtime");
            let seed_id = seed["event"].as_str().unwrap();
            if kind == "ack" {
                success_json(sb.receive("runtime", seed_id));
            }

            let normative: &[u8] = if kind == "item" {
                b"### 2. Example only\n"
            } else {
                b"ACK(blocked): example only\n"
            };
            let board_offset = fs::metadata(&sb.board).unwrap().len() as usize;
            let (payload, target_offset, target_line) = if context == "fenced" {
                let mut payload = b"```text\n".to_vec();
                let target_offset = board_offset + payload.len();
                payload.extend_from_slice(normative);
                payload.extend_from_slice(b"```\n");
                (payload, target_offset, normative.to_vec())
            } else {
                let mut line = b"> ".to_vec();
                line.extend_from_slice(normative);
                (line.clone(), board_offset, line)
            };
            assert!(sb.shell_append(&payload).status.success());
            let rejected_ref = source_ref(target_offset, &target_line);
            let rejected_file = sb.file(
                "rejected-recovery.json",
                &serde_json::to_vec(&rejected_ref).unwrap(),
            );
            let before = fs::read(&sb.board).unwrap();
            let rejected = if kind == "item" {
                let body = sb.file("rejected-item.txt", b"Recovered body.\n");
                Command::new("python3")
                    .arg("-B")
                    .arg(script("olp-board-event.py"))
                    .args(["item", "--board"])
                    .arg(&sb.board)
                    .args([
                        "--actor",
                        "outer",
                        "--number",
                        "2",
                        "--title",
                        "Example only",
                        "--body-file",
                    ])
                    .arg(body)
                    .arg("--recovery-file")
                    .arg(&rejected_file)
                    .output()
                    .unwrap()
            } else {
                let body = sb.file("rejected-ack.txt", b"Rejected example");
                Command::new("python3")
                    .arg("-B")
                    .arg(script("olp-board-event.py"))
                    .args(["ack", "--board"])
                    .arg(&sb.board)
                    .args([
                        "--actor",
                        "runtime",
                        "--item",
                        seed_id,
                        "--outcome",
                        "blocked",
                        "--r2",
                        "unverified",
                        "--body-file",
                    ])
                    .arg(body)
                    .arg("--recovery-file")
                    .arg(&rejected_file)
                    .output()
                    .unwrap()
            };
            failed(rejected, "unresolved normative line");
            assert_eq!(fs::read(&sb.board).unwrap(), before);

            let valid_offset = fs::metadata(&sb.board).unwrap().len() as usize;
            assert!(sb.shell_append(normative).status.success());
            let valid_file = sb.file(
                "valid-recovery.json",
                &serde_json::to_vec(&source_ref(valid_offset, normative)).unwrap(),
            );
            if kind == "item" {
                let body = sb.file("valid-item.txt", b"Recovered body.\n");
                success_json(
                    Command::new("python3")
                        .arg("-B")
                        .arg(script("olp-board-event.py"))
                        .args(["item", "--board"])
                        .arg(&sb.board)
                        .args([
                            "--actor",
                            "outer",
                            "--number",
                            "2",
                            "--title",
                            "Example only",
                            "--body-file",
                        ])
                        .arg(body)
                        .arg("--recovery-file")
                        .arg(&valid_file)
                        .output()
                        .unwrap(),
                );
            } else {
                let body = sb.file("valid-ack.txt", b"Backfill valid line");
                success_json(
                    Command::new("python3")
                        .arg("-B")
                        .arg(script("olp-board-event.py"))
                        .args(["ack", "--board"])
                        .arg(&sb.board)
                        .args([
                            "--actor",
                            "runtime",
                            "--item",
                            seed_id,
                            "--outcome",
                            "blocked",
                            "--r2",
                            "unverified",
                            "--body-file",
                        ])
                        .arg(body)
                        .arg("--recovery-file")
                        .arg(&valid_file)
                        .output()
                        .unwrap(),
                );
            }
            replace_last_event_recovery(&sb.board, &rejected_ref);
            let state = sb.state();
            assert_eq!(state["dispatch_blocked"], true);
            assert!(state["drift"].as_array().unwrap().iter().any(|entry| {
                entry["kind"] == "malformed_event"
                    && entry["reason"]
                        .as_str()
                        .unwrap()
                        .contains("unresolved normative line")
            }));
        }
    }
}

/// Test Path Statement: Adapter boundary with competing production CLI
/// processes against one real flock inode; the replayed chain proves ordering.
#[test]
fn olp_board_concurrent_writers_share_one_lock_and_chain() {
    let sb = Sandbox::new("concurrency");
    let mut children = Vec::new();
    for index in 0..8 {
        let body = sb.file(&format!("body-{index}.txt"), b"Concurrent item.\n");
        children.push(
            Command::new("python3")
                .arg("-B")
                .arg(script("olp-board-event.py"))
                .args(["item", "--board"])
                .arg(&sb.board)
                .args([
                    "--actor",
                    "outer",
                    "--number",
                    &index.to_string(),
                    "--title",
                    "Concurrent",
                ])
                .arg("--body-file")
                .arg(body)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let state = sb.state();
    assert_eq!(state["events"].as_array().unwrap().len(), 8);
    assert_eq!(state["unreceived"].as_array().unwrap().len(), 8);
}

/// Test Path Statement: Adapter boundary through both append implementations;
/// the old shell and new event writer overlap while sharing the canonical lock.
#[test]
fn olp_board_old_shell_and_new_writer_do_not_interleave() {
    let sb = Sandbox::new("shared-lock");
    let legacy = vec![b'L'; 512 * 1024];
    let mut legacy_body = legacy.clone();
    legacy_body.push(b'\n');
    let input = sb.file("legacy.txt", &legacy_body);
    let file = File::open(input).unwrap();
    let mut old = Command::new("bash")
        .arg(script("olp-board-append.sh"))
        .arg(&sb.board)
        .stdin(Stdio::from(file))
        .spawn()
        .unwrap();
    let body = sb.file("new.txt", b"New structured item.\n");
    let mut new = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-event.py"))
        .args(["item", "--board"])
        .arg(&sb.board)
        .args([
            "--actor",
            "outer",
            "--number",
            "G",
            "--title",
            "Shared lock",
        ])
        .arg("--body-file")
        .arg(body)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(old.wait().unwrap().success());
    assert!(new.wait().unwrap().success());
    let bytes = fs::read(&sb.board).unwrap();
    assert_eq!(
        bytes
            .windows(legacy_body.len())
            .filter(|w| *w == legacy_body)
            .count(),
        1
    );
    assert_eq!(sb.state()["events"].as_array().unwrap().len(), 1);
}

/// Test Path Statement: Real-path regression through wait/timeout, ready-file,
/// and sentinel prefixes; only wall-clock polling and temp files are isolated.
#[test]
fn olp_board_inbox_and_sentinel_expose_distinct_outcomes() {
    let sb = Sandbox::new("sentinel");
    let timeout = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-inbox.py"))
        .args(["--board"])
        .arg(&sb.board)
        .args([
            "--for",
            "runtime",
            "--wait",
            "--interval",
            "0.01",
            "--timeout",
            "0.03",
        ])
        .output()
        .unwrap();
    assert_eq!(timeout.status.code(), Some(3));

    let item = sb.item("H", "Wake runtime", "outer", "runtime");
    let ready = sb.root.join("ready.json");
    let sentinel = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&sb.board)
        .args(["--token", "NEVER", "--for", "runtime", "--ready-file"])
        .arg(&ready)
        .output()
        .unwrap();
    assert!(sentinel.status.success());
    assert!(String::from_utf8_lossy(&sentinel.stdout).starts_with("LEDGER-SIGNAL: "));
    assert!(ready.exists());

    let item_id = item["event"].as_str().unwrap();
    success_json(sb.receive("runtime", item_id));
    let ack = success_json(sb.ack("runtime", item_id, "done"));
    success_json(sb.review("outer", ack["event"].as_str().unwrap(), "accept"));
    let watcher_ready = sb.root.join("watcher-ready.json");
    let watcher = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&sb.board)
        .args([
            "--token",
            "WAKE-TOKEN",
            "--for",
            "runtime",
            "--interval",
            "0.02",
            "--timeout",
            "10",
            "--ready-file",
        ])
        .arg(&watcher_ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The ready file is written after the byte baseline is captured, so the
    // token below always lands after the baseline regardless of host load.
    wait_for_file(&watcher_ready);
    assert!(
        sb.shell_append(b"WAKE-TOKEN post-baseline\n")
            .status
            .success()
    );
    let output = watcher.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("BOARD-SIGNAL: "));
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-sentinel.py polling loop.
/// - Production path: both the ordinary deadline and incomplete trailing-line deadline emit their terminal result.
/// - External edges faked: temporary boards, ready files, and short polling intervals only.
/// - What this proves: every sentinel timeout is machine-readable and exits 3.
/// - What this intentionally does not exercise: inbox wait timeout, covered by the companion inbox test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_sentinel_timeouts_are_machine_readable
#[test]
fn olp_board_sentinel_timeouts_are_machine_readable() {
    let ordinary = Sandbox::new("sentinel-timeout");
    let output = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&ordinary.board)
        .args([
            "--token",
            "unused",
            "--for",
            "runtime",
            "--interval",
            "0.01",
            "--timeout",
            "0.03",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("TIMEOUT: "));

    let partial = Sandbox::new("sentinel-partial-timeout");
    let ready = partial.root.join("ready.json");
    let watcher = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&partial.board)
        .args([
            "--token",
            "unused",
            "--for",
            "runtime",
            "--interval",
            "0.01",
            "--timeout",
            "1.5",
            "--ready-file",
        ])
        .arg(&ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&ready);
    OpenOptions::new()
        .append(true)
        .open(&partial.board)
        .unwrap()
        .write_all(b"incomplete tail")
        .unwrap();
    let output = watcher.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("TIMEOUT: "));
    assert!(stdout.contains("incomplete trailing line"));
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-sentinel.py waiting on a live board.
/// - Production path: sentinel startup query enters its polling loop, then production item and ACK writers change the role-specific inbox projection.
/// - External edges faked: temporary boards, ready files, and short polling intervals only.
/// - What this proves: post-start item and ACK arrivals both use the frozen LEDGER-SIGNAL prefix.
/// - What this intentionally does not exercise: post-baseline text wakeups, covered by the companion sentinel test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_sentinel_reports_dynamic_ledger_signals
#[test]
fn olp_board_sentinel_reports_dynamic_ledger_signals() {
    let sb = Sandbox::new("sentinel-dynamic-ledger");
    let runtime_ready = sb.root.join("runtime-ready.json");
    let runtime_sentinel = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&sb.board)
        .args([
            "--token",
            "NEVER",
            "--for",
            "runtime",
            "--actor",
            "runtime",
            "--interval",
            "0.01",
            "--timeout",
            "2",
            "--ready-file",
        ])
        .arg(&runtime_ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if runtime_ready.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(runtime_ready.exists());

    let item = sb.item("DYNAMIC", "Wake after startup", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    let runtime_output = runtime_sentinel.wait_with_output().unwrap();
    assert!(runtime_output.status.success());
    let runtime_stdout = String::from_utf8_lossy(&runtime_output.stdout);
    assert!(runtime_stdout.starts_with("LEDGER-SIGNAL: "));
    assert!(runtime_stdout.contains(item_id));

    success_json(sb.receive("runtime", item_id));
    let outer_ready = sb.root.join("outer-ready.json");
    let outer_sentinel = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&sb.board)
        .args([
            "--token",
            "NEVER",
            "--for",
            "outer",
            "--actor",
            "outer",
            "--interval",
            "0.01",
            "--timeout",
            "2",
            "--ready-file",
        ])
        .arg(&outer_ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if outer_ready.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(outer_ready.exists());

    let ack = success_json(sb.ack("runtime", item_id, "done"));
    let ack_id = ack["event"].as_str().unwrap();
    let outer_output = outer_sentinel.wait_with_output().unwrap();
    assert!(outer_output.status.success());
    let outer_stdout = String::from_utf8_lossy(&outer_output.stdout);
    assert!(outer_stdout.starts_with("LEDGER-SIGNAL: "));
    assert!(outer_stdout.contains(ack_id));
}

/// Test Path Statement: Real waiting process with an empty head; ready-file
/// creation is a one-time startup effect across timeout, wakeup and collision.
#[test]
fn olp_board_empty_wait_owns_ready_file_once() {
    let timeout_board = Sandbox::new("ready-timeout");
    let ready = timeout_board.root.join("ready.json");
    let output = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-inbox.py"))
        .args(["--board"])
        .arg(&timeout_board.board)
        .args([
            "--for",
            "runtime",
            "--wait",
            "--interval",
            "0.01",
            "--timeout",
            "0.05",
            "--ready-file",
        ])
        .arg(&ready)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(ready.exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("File exists"));

    let wake = Sandbox::new("ready-wake");
    let wake_ready = wake.root.join("ready.json");
    let child = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-inbox.py"))
        .args(["--board"])
        .arg(&wake.board)
        .args([
            "--for",
            "runtime",
            "--wait",
            "--interval",
            "0.01",
            "--timeout",
            "2",
            "--ready-file",
        ])
        .arg(&wake_ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if wake_ready.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(wake_ready.exists());
    wake.item("R4", "Wake empty wait", "outer", "runtime");
    let output = child.wait_with_output().unwrap();
    assert_eq!(success_json(output)["matched"], true);

    let collision = Sandbox::new("ready-collision");
    let occupied = collision.file("ready.json", b"owner\n");
    let output = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-inbox.py"))
        .args(["--board"])
        .arg(&collision.board)
        .args(["--for", "runtime", "--ready-file"])
        .arg(&occupied)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(occupied).unwrap(), b"owner\n");
}

/// Test Path Statement: Real-path regression through sentinel health paths;
/// a rogue text append and inode replacement are real temporary-file effects.
#[test]
fn olp_board_sentinel_reports_drift_and_replacement_errors() {
    let drift = Sandbox::new("sentinel-drift");
    drift.item("I", "Enable structure", "outer", "runtime");
    assert!(
        drift
            .shell_append(b"ACK(blocked): unpaired\n")
            .status
            .success()
    );
    let output = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&drift.board)
        .args(["--token", "unused", "--for", "runtime"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("DRIFT: "));

    let replaced = Sandbox::new("sentinel-replaced");
    let ready = replaced.root.join("ready.json");
    let watcher = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&replaced.board)
        .args([
            "--token",
            "unused",
            "--for",
            "runtime",
            "--interval",
            "0.02",
            "--timeout",
            "2",
            "--ready-file",
        ])
        .arg(&ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if ready.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists());
    fs::rename(&replaced.board, replaced.root.join("old-board.md")).unwrap();
    fs::write(&replaced.board, b"").unwrap();
    let output = watcher.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("ERROR: "));
}

/// Test Path Statement: Real-path regression from structured event replay into
/// the production harvest script; no parser is duplicated in the test.
#[test]
fn olp_board_harvest_attributes_out_of_order_acks_to_event_items() {
    let sb = Sandbox::new("harvest");
    assert!(
        sb.shell_append(b"### 7. Legacy task\nACK(blocked): legacy condition\n")
            .status
            .success()
    );
    let first = sb.item("1", "First", "outer", "runtime");
    let second = sb.item("2", "Second", "outer", "runtime");
    let first_id = first["event"].as_str().unwrap();
    let second_id = second["event"].as_str().unwrap();
    success_json(sb.receive("runtime", first_id));
    success_json(sb.receive("runtime", second_id));
    success_json(sb.ack("runtime", second_id, "blocked"));
    success_json(sb.ack("runtime", first_id, "wontdo"));
    assert!(sb
        .shell_append(
            "> 外环(outer)·R2 记档(#1): verification issue\n```text\nACK(blocked): example only\n> 外环(outer)·R2 记档(#2): example only\n```\n".as_bytes(),
        )
        .status
        .success());

    let repo_root = sb.root.join("repo");
    fs::create_dir(&repo_root).unwrap();
    let missing = sb.root.join("missing-mcp.md");
    let output = Command::new("bash")
        .arg(script("olp-evo-harvest.sh"))
        .arg(&repo_root)
        .arg("--dry-run")
        .env("OLP_EVO_REVIEW_BOARD", &sb.board)
        .env("OLP_EVO_MCP_BOARD", missing)
        .env("OLP_EVO_STATE", sb.root.join("state"))
        .env("OLP_EVO_BOARD", sb.root.join("EVOLUTION.md"))
        .env("OLP_BOARD_EVENT_TOOL", script("olp-board-event.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("#2#blocked#"), "{text}");
    assert!(text.contains("#1#wontdo#"), "{text}");
    assert!(text.contains("#7#blocked#"), "{text}");
    assert!(text.contains("r2_record"), "{text}");
    assert_eq!(text.matches("### EVO-").count(), 4, "{text}");
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run.
/// - Production path: structured replay projection filters the unchanged legacy scanner.
/// - External edges faked: temporary boards and absent optional MCP source only.
/// - What this proves: recovered blocked/wontdo ACK text and its formal ACK yield one card.
/// - What this intentionally does not exercise: persistent evolution-board commits.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_deduplicates_recovered_acks
#[test]
fn olp_board_harvest_deduplicates_recovered_acks() {
    for outcome in ["blocked", "wontdo"] {
        let sb = Sandbox::new(&format!("harvest-recovered-{outcome}"));
        assert!(
            sb.shell_append(b"### 7. Legacy task\nACK(blocked): unrelated legacy condition\n")
                .status
                .success()
        );
        let item = sb.item("1", "Recovered ACK", "outer", "runtime");
        let item_id = item["event"].as_str().unwrap();
        success_json(sb.receive("runtime", item_id));
        let old_line = format!("ACK({outcome}): dependency unavailable\n");
        let old_offset = fs::metadata(&sb.board).unwrap().len() as usize;
        assert!(sb.shell_append(old_line.as_bytes()).status.success());
        let recovery = sb.file(
            "ack-recovery.json",
            &serde_json::to_vec(&source_ref(old_offset, old_line.as_bytes())).unwrap(),
        );
        let explanation = sb.file("ack.txt", b"Recovered through the structured event");
        success_json(
            Command::new("python3")
                .arg("-B")
                .arg(script("olp-board-event.py"))
                .args(["ack", "--board"])
                .arg(&sb.board)
                .args([
                    "--actor",
                    "runtime",
                    "--item",
                    item_id,
                    "--outcome",
                    outcome,
                    "--r2",
                    "unverified",
                    "--body-file",
                ])
                .arg(explanation)
                .arg("--recovery-file")
                .arg(recovery)
                .output()
                .unwrap(),
        );
        assert!(sb
            .shell_append(
                "> 外环(outer)·R2 记档(#1): retained signed trigger\n```text\nACK(blocked): fenced example\n```\n"
                    .as_bytes(),
            )
            .status
            .success());

        let repo_root = sb.root.join("repo");
        fs::create_dir(&repo_root).unwrap();
        let output = Command::new("bash")
            .arg(script("olp-evo-harvest.sh"))
            .arg(&repo_root)
            .arg("--dry-run")
            .env("OLP_EVO_REVIEW_BOARD", &sb.board)
            .env("OLP_EVO_MCP_BOARD", sb.root.join("missing-mcp.md"))
            .env("OLP_EVO_STATE", sb.root.join("state"))
            .env("OLP_EVO_BOARD", sb.root.join("EVOLUTION.md"))
            .env("OLP_BOARD_EVENT_TOOL", script("olp-board-event.py"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(text.matches("### EVO-").count(), 3, "{outcome}: {text}");
        assert!(text.contains("#7#blocked#"), "{outcome}: {text}");
        assert!(text.contains("r2_record"), "{outcome}: {text}");
        assert!(
            text.contains(&format!("#1#{outcome}#")),
            "{outcome}: {text}"
        );
        assert!(
            !text.contains("dependency unavailable"),
            "{outcome}: {text}"
        );
    }
}

/// Test Path Statement: Adapter boundary through the verified append CLI;
/// a saved receipt remains valid after later appends.
#[test]
fn olp_board_append_receipt_survives_later_appends() {
    let sb = Sandbox::new("receipt");
    let first = sb.file("first.txt", b"first body\n");
    let receipt = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-append.py"))
            .args(["append", "--board"])
            .arg(&sb.board)
            .arg("--body-file")
            .arg(first)
            .output()
            .unwrap(),
    );
    assert!(sb.shell_append(b"later body\n").status.success());
    let body = sb.file("verify-body.txt", b"first body\n");
    let verified = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-append.py"))
            .args(["verify", "--board"])
            .arg(&sb.board)
            .arg("--body-file")
            .arg(body)
            .arg("--offset")
            .arg(receipt["offset"].as_u64().unwrap().to_string())
            .arg("--ts")
            .arg(receipt["ts"].as_str().unwrap())
            .output()
            .unwrap(),
    );
    assert_eq!(verified["verified"], true);
}

/// Test Path Statement: Adapter boundary against the production append
/// function with faults injected only in the test process at OS effect edges.
#[test]
fn olp_board_test_process_faults_report_partial_write_truthfully() {
    let probe = br#"
import importlib.util, json, os, sys
from types import SimpleNamespace

path, board, body, mode = sys.argv[1:]
spec = importlib.util.spec_from_file_location("append", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
progress = {"may_have_appended": False}
real_write, real_fsync, real_read = module.os.write, module.os.fsync, module.read_region
calls = 0
def changed_write(fd, data):
    global calls
    calls += 1
    if calls == 1:
        return real_write(fd, data[:max(1, len(data) // 2)])
    if mode == "partial":
        raise OSError("injected partial write")
    return real_write(fd, data)
if mode in ("short", "partial"):
    module.os.write = changed_write
elif mode == "fsync":
    module.os.fsync = lambda fd: (_ for _ in ()).throw(OSError("injected fsync"))
elif mode == "readback":
    module.read_region = lambda fd, offset, length: b""
args = SimpleNamespace(command="append", board=board, body_file=body,
                       lock_timeout=1.0, max_bytes=None)
try:
    receipt = module.run(args, progress)
    print(json.dumps({"ok": True, "progress": progress, "receipt": receipt}))
except Exception as error:
    print(json.dumps({"ok": False, "progress": progress, "error": str(error)}))
"#;
    for mode in ["short", "partial", "fsync", "readback"] {
        let sb = Sandbox::new(mode);
        let probe_path = sb.file("probe.py", probe);
        let body = sb.file("payload.txt", b"fault boundary payload\n");
        let result = success_json(
            Command::new("python3")
                .arg("-B")
                .arg(probe_path)
                .arg(script("olp-board-append.py"))
                .arg(&sb.board)
                .arg(body)
                .arg(mode)
                .output()
                .unwrap(),
        );
        assert_eq!(result["progress"]["may_have_appended"], true);
        if mode == "short" {
            assert_eq!(result["ok"], true);
        } else {
            assert_eq!(result["ok"], false);
        }
    }
}

/// Test Path Statement: Real event CLI main with only OS/output boundaries
/// replaced in the test process; every failure emits one machine JSON object.
#[test]
fn olp_board_event_cli_reports_write_boundary_failures_as_json() {
    let probe = br#"
import builtins, importlib.util, os, sys

event_path, append_path, board, body, mode = sys.argv[1:]
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
append = load("append_probe", append_path)
event = load("event_probe", event_path)
event.board_module = lambda: append
real_write = append.os.write
calls = 0
def partial_write(fd, data):
    global calls
    calls += 1
    if calls == 1:
        return real_write(fd, data[:max(1, len(data) // 2)])
    raise OSError("injected write failure")
if mode == "write":
    append.os.write = partial_write
elif mode == "fsync":
    append.os.fsync = lambda fd: (_ for _ in ()).throw(OSError("injected fsync failure"))
elif mode == "readback":
    append.read_region = lambda fd, offset, length: b""
elif mode == "receipt":
    real_print = builtins.print
    def output_failure(*args, **kwargs):
        if kwargs.get("file", sys.stdout) is sys.stdout:
            raise OSError("injected receipt output failure")
        return real_print(*args, **kwargs)
    builtins.print = output_failure
raise SystemExit(event.main(["item", "--board", board, "--actor", "outer",
                            "--number", "R7", "--title", "Failure boundary",
                            "--body-file", body]))
"#;
    for mode in ["write", "fsync", "readback", "receipt"] {
        let sb = Sandbox::new(&format!("event-{mode}"));
        let probe_path = sb.file("event-probe.py", probe);
        let body = sb.file("item.txt", b"Boundary work.\n");
        let output = Command::new("python3")
            .arg("-B")
            .arg(probe_path)
            .arg(script("olp-board-event.py"))
            .arg(script("olp-board-append.py"))
            .arg(&sb.board)
            .arg(body)
            .arg(mode)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "mode={mode}");
        assert!(output.stdout.is_empty(), "mode={mode}");
        let lines: Vec<_> = output
            .stderr
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "mode={mode} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error: Value = serde_json::from_slice(lines[0]).unwrap();
        assert_eq!(error["verified"], false);
        assert_eq!(error["may_have_appended"], true);
        assert_eq!(error["execution_authorized"], false);
        assert!(!fs::read(&sb.board).unwrap().is_empty());
    }

    let before = Sandbox::new("event-prewrite");
    let output = before.event(&[
        "item",
        "--actor",
        "outer",
        "--number",
        "R7",
        "--title",
        "Prewrite",
        "--body-file",
        "/definitely/missing/body",
    ]);
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["may_have_appended"], false);
    assert!(fs::read(&before.board).unwrap().is_empty());
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-init.sh with OLP_BOARD_MODE=structured.
/// - Production path: installer creates the structured template and lock, installs all four tools, and those installed copies execute a ledger lifecycle.
/// - External edges faked: temporary HOME/repository and existing local executables only.
/// - What this proves: opt-in deployment is usable and reruns do not overwrite project or installed files.
/// - What this intentionally does not exercise: network fallback for the onboarding card.
/// - Focused command: cargo test --test olp_board_protocol olp_board_init_installs_structured_mode_idempotently
#[test]
fn olp_board_init_installs_structured_mode_idempotently() {
    let sb = Sandbox::new("init-structured");
    let project = sb.root.join("project");
    let home = sb.root.join("home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );

    let stub_path = path_with_octoscode_stubs(&sb.root);
    let run_init = || {
        Command::new("bash")
            .arg(script("olp-init.sh"))
            .current_dir(&project)
            .env("HOME", &home)
            .env("PATH", &stub_path)
            .env("OLP_BOARD_MODE", "structured")
            .env("OLP_INIT_LANG", "en")
            .output()
            .unwrap()
    };
    let first = run_init();
    assert!(
        first.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let outer = home.join(".octos/outer");
    for name in [
        "olp-board-append.py",
        "olp-board-event.py",
        "olp-board-inbox.py",
        "olp-board-sentinel.py",
    ] {
        assert!(outer.join(name).is_file(), "missing installed {name}");
    }
    let board = project.join(".octos/OUTER_LOOP_REVIEW.md");
    let lock = project.join(".octos/OUTER_LOOP_REVIEW.md.lock");
    let loop_file = project.join(".octos/loop.md");
    assert!(lock.is_file());
    let board_text = fs::read_to_string(&board).unwrap();
    assert!(!board_text.contains("ACK:"));
    assert!(board_text.contains("\n<!-- olp-board/v1 -->\n"));
    assert!(
        Command::new("git")
            .args(["check-ignore", "-q", ".octos/OUTER_LOOP_REVIEW.md.lock"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success(),
        "structured init must ignore the per-checkout lock"
    );
    let loop_text = fs::read_to_string(&loop_file).unwrap();
    assert!(loop_text.contains("olp-board-inbox.py"));
    assert!(loop_text.contains(" receive "));
    assert!(loop_text.contains("olp-board-event.py ack"));
    assert!(loop_text.contains("earliest `unreceived` item in ledger event order"));
    assert!(!loop_text.contains("lowest-numbered `unreceived` item"));

    let note = sb.file("deployment-note.txt", b"Structured deployment note.\n");
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-append.py"))
            .args(["append", "--board"])
            .arg(&board)
            .arg("--body-file")
            .arg(&note)
            .output()
            .unwrap(),
    );
    let item_body = sb.file("installed-item.txt", b"Run the installed tools.\n");
    let item = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-event.py"))
            .args(["item", "--board"])
            .arg(&board)
            .args([
                "--actor",
                "outer",
                "--number",
                "1",
                "--title",
                "Installed lifecycle",
                "--to",
                "runtime",
                "--body-file",
            ])
            .arg(&item_body)
            .output()
            .unwrap(),
    );
    assert_eq!(item["verified"], true);
    let inbox = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-inbox.py"))
            .args(["--board"])
            .arg(&board)
            .args(["--for", "runtime", "--actor", "runtime"])
            .output()
            .unwrap(),
    );
    assert_eq!(inbox["matched"], true);
    let sentinel = Command::new("python3")
        .arg("-B")
        .arg(outer.join("olp-board-sentinel.py"))
        .args(["--board"])
        .arg(&board)
        .args([
            "--token",
            "never-used",
            "--for",
            "runtime",
            "--actor",
            "runtime",
            "--interval",
            "0.01",
            "--timeout",
            "0.1",
        ])
        .output()
        .unwrap();
    assert!(sentinel.status.success());
    assert!(String::from_utf8_lossy(&sentinel.stdout).starts_with("LEDGER-SIGNAL:"));
    let item_id = item["event"].as_str().unwrap();
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-event.py"))
            .args(["receive", "--board"])
            .arg(&board)
            .args(["--actor", "runtime", "--item", item_id])
            .output()
            .unwrap(),
    );
    let ack_body = sb.file("installed-ack.txt", b"Installed lifecycle completed");
    let ack = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-event.py"))
            .args(["ack", "--board"])
            .arg(&board)
            .args([
                "--actor",
                "runtime",
                "--item",
                item_id,
                "--outcome",
                "done",
                "--r2",
                "verified",
                "--commit",
                "0123456789abcdef0123456789abcdef01234567",
                "--body-file",
            ])
            .arg(&ack_body)
            .output()
            .unwrap(),
    );
    let review_body = sb.file("installed-review.txt", b"Installed lifecycle reviewed");
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-event.py"))
            .args(["review", "--board"])
            .arg(&board)
            .args([
                "--actor",
                "outer",
                "--ack",
                ack["event"].as_str().unwrap(),
                "--decision",
                "accept",
                "--body-file",
            ])
            .arg(&review_body)
            .output()
            .unwrap(),
    );
    let final_state = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(outer.join("olp-board-event.py"))
            .args(["state", "--board"])
            .arg(&board)
            .output()
            .unwrap(),
    );
    assert!(final_state["unreceived"].as_array().unwrap().is_empty());
    assert!(
        final_state["received_pending"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(final_state["unreviewed_ack"].as_array().unwrap().is_empty());

    let installed_event = outer.join("olp-board-event.py");
    let mut customized_tool = fs::read(&installed_event).unwrap();
    customized_tool.extend_from_slice(b"# local migration marker\n");
    fs::write(&installed_event, &customized_tool).unwrap();
    fs::write(&lock, b"existing lock marker\n").unwrap();
    let board_before = fs::read(&board).unwrap();
    let loop_before = fs::read(&loop_file).unwrap();
    let second = run_init();
    assert!(second.status.success());
    assert_eq!(fs::read(&installed_event).unwrap(), customized_tool);
    assert_eq!(fs::read(&lock).unwrap(), b"existing lock marker\n");
    assert_eq!(fs::read(&board).unwrap(), board_before);
    assert_eq!(fs::read(&loop_file).unwrap(), loop_before);
    assert!(String::from_utf8_lossy(&second.stdout).contains("migration"));
}

/// Test Path Statement:
/// - Tier: Adapter boundary.
/// - Production entrypoint: olp-init.sh in its default mode and with a PATH that contains no Python.
/// - Production path: the unchanged legacy template and shell-tool installation branches execute in temporary projects.
/// - External edges faked: temporary HOME/repositories and a curated executable PATH.
/// - What this proves: default adoption stays legacy and missing Python disables only structured capability.
/// - What this intentionally does not exercise: structured lifecycle commands, covered by the companion test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_init_preserves_legacy_fallback_without_python
#[test]
fn olp_board_init_preserves_legacy_fallback_without_python() {
    let sb = Sandbox::new("init-legacy");
    let project = sb.root.join("legacy-project");
    let home = sb.root.join("legacy-home");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    let legacy = Command::new("bash")
        .arg(script("olp-init.sh"))
        .current_dir(&project)
        .env("HOME", &home)
        .env("PATH", path_with_octoscode_stubs(&sb.root))
        .env_remove("OLP_BOARD_MODE")
        .output()
        .unwrap();
    assert!(legacy.status.success());
    let loop_text = fs::read_to_string(project.join(".octos/loop.md")).unwrap();
    assert!(loop_text.contains("ACK(done|wontdo|blocked)"));
    assert!(!loop_text.contains("olp-board-inbox.py"));

    let no_python_project = sb.root.join("no-python-project");
    let no_python_home = sb.root.join("no-python-home");
    let bin = sb.root.join("no-python-bin");
    fs::create_dir_all(&no_python_project).unwrap();
    fs::create_dir_all(&no_python_home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&no_python_project)
            .status()
            .unwrap()
            .success()
    );
    let git_lookup = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(git_lookup.status.success());
    let git_path = String::from_utf8(git_lookup.stdout).unwrap();
    std::os::unix::fs::symlink(git_path.trim(), bin.join("git")).unwrap();
    for (name, target) in [
        ("mkdir", "/bin/mkdir"),
        ("cat", "/bin/cat"),
        ("tail", "/usr/bin/tail"),
        ("od", "/usr/bin/od"),
        ("tr", "/usr/bin/tr"),
        ("dirname", "/usr/bin/dirname"),
        ("cp", "/bin/cp"),
        ("chmod", "/bin/chmod"),
        ("rm", "/bin/rm"),
    ] {
        std::os::unix::fs::symlink(target, bin.join(name)).unwrap();
    }
    for name in ["octoscode", "octos"] {
        let path = bin.join(name);
        fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        fs::set_permissions(&path, permissions).unwrap();
    }
    let no_python = Command::new("/bin/bash")
        .arg(script("olp-init.sh"))
        .current_dir(&no_python_project)
        .env("HOME", &no_python_home)
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert!(
        no_python.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&no_python.stdout),
        String::from_utf8_lossy(&no_python.stderr)
    );
    let stdout = String::from_utf8_lossy(&no_python.stdout);
    assert!(stdout.contains("structured board tools unavailable"));
    assert!(no_python_home.join(".octos/outer/watch-board.sh").is_file());
    assert!(
        no_python_home
            .join(".octos/outer/board-append.sh")
            .is_file()
    );
    assert!(
        !no_python_home
            .join(".octos/outer/olp-board-event.py")
            .exists()
    );
}

#[test]
fn olp_board_public_tools_contain_no_production_fault_switches() {
    for name in [
        "olp-board-append.py",
        "olp-board-event.py",
        "olp-board-inbox.py",
        "olp-board-sentinel.py",
    ] {
        let text = fs::read_to_string(script(name)).unwrap();
        assert!(!text.contains("OLP_BOARD_TEST"));
        assert!(!text.contains("fault-inject"));
    }
}

impl Sandbox {
    fn item_output(&self, number: &str) -> Output {
        let body = self.file(&format!("item-{number}.txt"), b"Executable work item.\n");
        self.event(&[
            "item",
            "--actor",
            "outer",
            "--number",
            number,
            "--title",
            "Follow-up",
            "--body-file",
            body.to_str().unwrap(),
        ])
    }

    fn explained(&self, verb: &str, args: &[&str]) -> Output {
        let body = self.file(
            &format!("{verb}-explanation.txt"),
            b"Checked by the operator",
        );
        let mut full = vec![verb];
        full.extend_from_slice(args);
        full.extend_from_slice(&["--body-file", body.to_str().unwrap()]);
        self.event(&full)
    }

    fn withdraw(&self, actor: &str, item: &str) -> Output {
        self.explained("withdraw", &["--actor", actor, "--item", item])
    }

    fn resolve(&self, actor: &str, review: &str, next: Option<&str>) -> Output {
        let mut args = vec!["--actor", actor, "--review", review];
        if let Some(next) = next {
            args.extend_from_slice(&["--next", next]);
        }
        self.explained("resolve", &args)
    }

    fn void(&self, target: &Value) -> Output {
        let evidence = json!({
            "offset": target["offset"],
            "length": target["length"],
            "sha256": target["sha256"],
        });
        let file = self.file("void-target.json", &serde_json::to_vec(&evidence).unwrap());
        self.explained(
            "void",
            &["--actor", "outer", "--target-file", file.to_str().unwrap()],
        )
    }

    fn ack_recovering(&self, item: &str, outcome: &str, evidence: &Value) -> Output {
        let file = self.file("ack-evidence.json", &serde_json::to_vec(evidence).unwrap());
        self.explained(
            "ack",
            &[
                "--actor",
                "runtime",
                "--item",
                item,
                "--outcome",
                outcome,
                "--r2",
                "unverified",
                "--recovery-file",
                file.to_str().unwrap(),
            ],
        )
    }

    fn raw_append(&self, bytes: &[u8]) {
        OpenOptions::new()
            .append(true)
            .open(&self.board)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }

    fn len(&self) -> usize {
        fs::metadata(&self.board).unwrap().len() as usize
    }

    fn harvest(&self, event_tool: &PathBuf, path_prefix: Option<&PathBuf>) -> Output {
        self.harvest_with(Path::new("bash"), event_tool, path_prefix)
    }

    /// Harvest dry-run with the real event tool under one specific bash.
    fn harvest_under(&self, shell: &Path) -> Output {
        self.harvest_with(shell, &script("olp-board-event.py"), None)
    }

    fn harvest_with(
        &self,
        shell: &Path,
        event_tool: &PathBuf,
        path_prefix: Option<&PathBuf>,
    ) -> Output {
        let repo_root = self.root.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        let mut command = Command::new(shell);
        command
            .arg(script("olp-evo-harvest.sh"))
            .arg(&repo_root)
            .arg("--dry-run")
            .env("OLP_EVO_REVIEW_BOARD", &self.board)
            .env("OLP_EVO_MCP_BOARD", self.root.join("missing-mcp.md"))
            .env("OLP_EVO_STATE", self.root.join("harvest-state"))
            .env("OLP_EVO_BOARD", self.root.join("EVOLUTION.md"))
            .env("OLP_BOARD_EVENT_TOOL", event_tool);
        if let Some(prefix) = path_prefix {
            let path = std::env::var("PATH").unwrap_or_default();
            command.env("PATH", format!("{}:{path}", prefix.display()));
        }
        command.output().unwrap()
    }
}

/// Every distinct bash on this host the harvest may run under: the one on PATH
/// and the system `/bin/bash` (on macOS that is 3.2, whose `read` cuts a line
/// at a NUL byte where bash 4+ drops the byte).
fn harvest_shells() -> Vec<PathBuf> {
    let mut shells = Vec::new();
    let mut versions = Vec::new();
    for candidate in ["bash", "/bin/bash"] {
        let Ok(output) = Command::new(candidate)
            .args(["-c", "printf %s \"$BASH_VERSION\""])
            .output()
        else {
            continue;
        };
        let version = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() && !versions.contains(&version) {
            versions.push(version);
            shells.push(PathBuf::from(candidate));
        }
    }
    assert!(!shells.is_empty(), "no bash found");
    shells
}

/// Whether this bash's `read` drops a NUL byte (bash 4+) rather than cutting
/// the line there (bash 3.2).
fn read_drops_nul(shell: &Path) -> bool {
    let mut child = Command::new(shell)
        .args(["-c", "IFS= read -r line; printf %s \"$line\""])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"A\0B\n").unwrap();
    child.wait_with_output().unwrap().stdout == b"AB"
}

/// The `identity:` line of one harvest card.
fn card_identity(card: &str) -> String {
    card.lines()
        .find_map(|line| line.strip_prefix("identity: "))
        .unwrap_or_else(|| panic!("no identity in {card}"))
        .to_owned()
}

/// The cards a successful harvest dry-run printed, one string per card.
fn evo_cards(output: Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split("### EVO-")
        .skip(1)
        .map(str::to_owned)
        .collect()
}

fn drift_kinds(state: &Value) -> Vec<String> {
    state["drift"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// Hold the board lock from another process until the returned child is killed.
fn hold_lock(sb: &Sandbox, mode: &str) -> std::process::Child {
    let ready = sb.root.join(format!("lock-{mode}-ready"));
    let child = Command::new("python3")
        .arg("-B")
        .arg("-c")
        .arg(
            "import fcntl, sys, time\n\
             handle = open(sys.argv[1], 'rb')\n\
             fcntl.flock(handle, fcntl.LOCK_SH if sys.argv[2] == 'shared' else fcntl.LOCK_EX)\n\
             open(sys.argv[3], 'w').close()\n\
             time.sleep(60)\n",
        )
        .arg(sb.root.join("BOARD.md.lock"))
        .arg(mode)
        .arg(&ready)
        .spawn()
        .unwrap();
    wait_for_file(&ready);
    child
}

fn mask_timestamps(text: &str) -> String {
    let bytes = text.as_bytes();
    let shape = b"dddd-dd-ddTdd:dd:ddZ";
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let window = bytes.get(index..index + shape.len());
        let is_stamp = window.is_some_and(|window| {
            window
                .iter()
                .zip(shape)
                .all(|(byte, expected)| match expected {
                    b'd' => byte.is_ascii_digit(),
                    other => byte == other,
                })
        });
        if is_stamp {
            out.extend_from_slice(b"<ts>");
            index += shape.len();
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state/item/void/verify CLIs.
/// - Production path: replay turns damaged event lines into blocking DRIFT, and the append-only void event quarantines them.
/// - External edges faked: a raw file append stands in for a writer that bypasses every guard, and a truncated write stands in for an interrupted append.
/// - What this proves: one bad line no longer disables the board; earlier receipts still verify, writes continue from the last valid head, and every stuck state has an exact, audited way out.
/// - What this intentionally does not exercise: the legacy shell guard, covered by its own test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_void_quarantines_injected_and_partial_event_lines
#[test]
fn olp_board_void_quarantines_injected_and_partial_event_lines() {
    let sb = Sandbox::new("void-injected");
    let first = sb.item("1", "First", "outer", "runtime");
    let receipt = sb.file("first-receipt.json", &serde_json::to_vec(&first).unwrap());
    let text = fs::read_to_string(&sb.board).unwrap();
    let event_line = text
        .lines()
        .find(|line| line.starts_with("> OLP-EVENT "))
        .unwrap()
        .to_owned();
    let note_offset = sb.len();
    sb.raw_append(format!("Quoting the event:\n{event_line}\n").as_bytes());
    let state = sb.state();
    assert_eq!(drift_kinds(&state), vec!["malformed_event"]);
    assert_eq!(state["dispatch_blocked"], true);
    let injected = state["drift"][0].clone();
    success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(["verify", "--receipt-file"])
            .arg(&receipt)
            .output()
            .unwrap(),
    );
    let second = success_json(sb.item_output("2"));
    assert_eq!(sb.state()["head"], second["event"]);
    let note = b"Quoting the event:\n";
    failed(
        sb.void(&source_ref(note_offset, note)),
        "not an unresolved normative line",
    );
    success_json(sb.void(&injected));
    let state = sb.state();
    assert!(state["drift"].as_array().unwrap().is_empty(), "{state}");
    assert_eq!(state["quarantine_evidence"].as_array().unwrap().len(), 1);
    failed(sb.void(&injected), "already consumed");

    let sb = Sandbox::new("void-partial-event");
    sb.item("1", "First", "outer", "runtime");
    sb.raw_append(b"### 2. Second\nBody.\n> OLP-EVENT {\"schema\":\"olp-board/v1\",\"id\":\"ab");
    let state = sb.state();
    assert_eq!(
        drift_kinds(&state),
        vec!["unpaired_item", "malformed_event"]
    );
    let heading = state["drift"][0].clone();
    let tail = state["drift"][1].clone();
    for field in ["offset", "length", "sha256"] {
        assert_eq!(state["partial_tail"][field], tail[field]);
    }
    let before = fs::read(&sb.board).unwrap();
    failed(sb.item_output("3"), "partial trailing line");
    failed(sb.void(&heading), "partial line");
    assert_eq!(fs::read(&sb.board).unwrap(), before);
    success_json(sb.void(&tail));
    success_json(sb.void(&heading));
    let state = sb.state();
    assert_eq!(state["mode"], "structured");
    assert!(state["drift"].as_array().unwrap().is_empty(), "{state}");
    assert_eq!(&fs::read(&sb.board).unwrap()[..before.len()], &before[..]);
    success_json(sb.item_output("3"));

    let sb = Sandbox::new("void-partial-fence");
    sb.item("1", "First", "outer", "runtime");
    sb.raw_append(b"```");
    let partial = sb.state()["partial_tail"].clone();
    assert!(partial.is_object());
    failed(sb.void(&partial), "close the open code fence first");
    let close = sb.file("close-fence.txt", b"\n```\n");
    let plain_append = |extra: &[&str]| {
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-append.py"))
            .args(["append", "--board"])
            .arg(&sb.board)
            .arg("--body-file")
            .arg(&close)
            .args(extra)
            .output()
            .unwrap()
    };
    failed(plain_append(&[]), "must end with LF");
    success_json(plain_append(&["--terminate-partial-line"]));
    failed(
        plain_append(&["--terminate-partial-line"]),
        "needs a board that ends",
    );
    let state = sb.state();
    assert!(state["partial_tail"].is_null());
    assert!(state["drift"].as_array().unwrap().is_empty(), "{state}");
    success_json(sb.item_output("2"));

    let sb = Sandbox::new("void-partial-text");
    sb.item("1", "First", "outer", "runtime");
    sb.raw_append(b"RECEIVE(item=abc");
    let state = sb.state();
    assert!(state["drift"].as_array().unwrap().is_empty());
    let partial = state["partial_tail"].clone();
    assert!(partial.is_object(), "{state}");
    failed(sb.item_output("2"), "partial trailing line");
    success_json(sb.void(&partial));
    assert!(sb.state()["partial_tail"].is_null());
    success_json(sb.item_output("2"));
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: scripts/olp-board-append.sh.
/// - Production path: the legacy helper checks the board for opt-in evidence under its own lock.
/// - External edges faked: temporary boards only.
/// - What this proves: opted-in boards refuse hand-appended event lines, while legacy boards and ordinary text keep the old behaviour.
/// - What this intentionally does not exercise: event replay, covered by the quarantine test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_legacy_shell_refuses_event_lines_on_opted_in_boards
#[test]
fn olp_board_legacy_shell_refuses_event_lines_on_opted_in_boards() {
    let legacy = Sandbox::new("shell-legacy");
    assert!(legacy.shell_append(b"### 1. Legacy\n").status.success());
    assert!(
        legacy
            .shell_append(b"> OLP-EVENT {quoted on a legacy board}\n")
            .status
            .success()
    );

    let marked = Sandbox::new("shell-marker");
    fs::write(&marked.board, b"# Board\n\n<!-- olp-board/v1 -->\n").unwrap();
    let structured = Sandbox::new("shell-structured");
    structured.item("1", "Opted in", "outer", "runtime");
    for sb in [&marked, &structured] {
        let before = fs::read(&sb.board).unwrap();
        for body in [
            b"Quote:\n> OLP-EVENT {copied}\n".as_slice(),
            b"ts=2026-09-27T00:00:00Z\n".as_slice(),
        ] {
            let refused = sb.shell_append(body);
            assert_eq!(refused.status.code(), Some(2));
            assert!(String::from_utf8_lossy(&refused.stderr).contains("opted in to olp-board/v1"));
            assert_eq!(fs::read(&sb.board).unwrap(), before);
        }
        assert!(sb.shell_append(b"Plain note.\n").status.success());
        // Both helpers judge a standalone ts= line alike: an optional CR before
        // LF is still a timestamp line, a trailing space is not.
        for (body, refused) in [
            (b"ts=2026-09-27T00:00:00Z\r\n".as_slice(), true),
            (b"ts=2026-09-27T00:00:00Z \n".as_slice(), false),
            (b"```text\nts=2026-09-27T00:00:00Z\n```\n".as_slice(), true),
        ] {
            let shell = sb.shell_append(body);
            let python = Command::new("python3")
                .arg("-B")
                .arg(script("olp-board-append.py"))
                .args(["append", "--board"])
                .arg(&sb.board)
                .arg("--body-file")
                .arg(sb.file("ts-body.txt", body))
                .output()
                .unwrap();
            for (tool, output) in [("shell", &shell), ("python", &python)] {
                assert_eq!(
                    output.status.success(),
                    !refused,
                    "{tool} {:?} stderr={}",
                    String::from_utf8_lossy(body),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state and ack/void CLIs after legacy shell appends.
/// - Production path: shared normative/suspected ACK classification feeds DRIFT, recovery and void.
/// - External edges faked: temporary boards only.
/// - What this proves: every hand-written ACK form lanes produce in practice blocks dispatch, recovers only with an exact matching outcome, and otherwise needs an explicit void.
/// - What this intentionally does not exercise: harvest cards for the same lines.
/// - Focused command: cargo test --test olp_board_protocol olp_board_ack_variants_raise_drift_and_recover_exactly
#[test]
fn olp_board_ack_variants_raise_drift_and_recover_exactly() {
    for (line, kind) in [
        ("ACK(done): column zero\n", "unpaired_ack"),
        ("  ACK(done): indented\n", "unpaired_ack"),
        ("ACK(done)： full-width colon\n", "unpaired_ack"),
        ("ACK(done): CRLF line ending\r\n", "unpaired_ack"),
        ("> ACK(done): quoted\n", "suspected_ack"),
        ("- ACK(done): bulleted\n", "suspected_ack"),
        ("### ACK(done): heading\n", "suspected_ack"),
        ("**ACK(done)**: emphasised\n", "suspected_ack"),
        ("Finished; ACK(done) inline\n", "suspected_ack"),
        ("### ACK done\n", "suspected_ack"),
        ("ACK: retired bare form\n", "suspected_ack"),
    ] {
        let sb = Sandbox::new("ack-variant");
        let item = sb.item("1", "Variant", "outer", "runtime");
        let item_id = item["event"].as_str().unwrap();
        success_json(sb.receive("runtime", item_id));
        let offset = sb.len();
        assert!(sb.shell_append(line.as_bytes()).status.success());
        let state = sb.state();
        assert_eq!(drift_kinds(&state), vec![kind], "{line}");
        assert_eq!(state["drift"][0]["offset"], offset);
        assert_eq!(state["dispatch_blocked"], true);
        assert_eq!(state["received_pending"].as_array().unwrap().len(), 1);
        let evidence = source_ref(offset, line.as_bytes());
        let before = fs::read(&sb.board).unwrap();
        if line.contains("ACK(done)") {
            failed(
                sb.ack_recovering(item_id, "blocked", &evidence),
                "outcome does not match",
            );
            assert_eq!(fs::read(&sb.board).unwrap(), before);
            success_json(sb.ack_recovering(item_id, "done", &evidence));
        } else {
            failed(sb.ack_recovering(item_id, "done", &evidence), "void it");
            assert_eq!(fs::read(&sb.board).unwrap(), before);
            success_json(sb.void(&evidence));
        }
        assert!(sb.state()["drift"].as_array().unwrap().is_empty(), "{line}");
    }

    // Prose that merely starts with the word ACK is not a hand-written ACK.
    let sb = Sandbox::new("ack-prose");
    sb.item("1", "Prose", "outer", "runtime");
    assert!(sb.shell_append(b"ACK received, thanks\n").status.success());
    assert!(sb.state()["drift"].as_array().unwrap().is_empty());
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event item/receive/ack/review/withdraw/resolve and state CLIs.
/// - Production path: lifecycle validation and projection of the two closing events.
/// - External edges faked: temporary boards only.
/// - What this proves: mis-addressed items and escalations can be closed by their author, with exact refusals for every other actor or state.
/// - What this intentionally does not exercise: inbox/sentinel baselines, covered by the since-head test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_withdraw_and_resolve_close_open_queues
#[test]
fn olp_board_withdraw_and_resolve_close_open_queues() {
    let sb = Sandbox::new("close-queues");
    let wrong = sb.item("9", "Mis-addressed", "outer", "runtmie");
    let wrong_id = wrong["event"].as_str().unwrap();
    failed(sb.withdraw("runtime", wrong_id), "Only the item author");
    success_json(sb.withdraw("outer", wrong_id));
    failed(sb.receive("runtmie", wrong_id), "withdrawn");
    failed(sb.withdraw("outer", wrong_id), "already withdrawn");
    assert!(sb.state()["unreceived"].as_array().unwrap().is_empty());

    let item = sb.item("1", "Needs a decision", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    success_json(sb.receive("runtime", item_id));
    failed(sb.withdraw("outer", item_id), "cannot be withdrawn");
    let ack = success_json(sb.ack("runtime", item_id, "blocked"));
    let review = success_json(sb.review("outer", ack["event"].as_str().unwrap(), "escalate"));
    let review_id = review["event"].as_str().unwrap();
    assert_eq!(sb.state()["escalated"].as_array().unwrap().len(), 1);
    failed(
        sb.resolve("runtime", review_id, None),
        "Only the item author",
    );
    failed(
        sb.resolve("outer", review_id, Some(item_id)),
        "different existing item",
    );
    let follow = success_json(sb.item_output("2"));
    let follow_id = follow["event"].as_str().unwrap();
    success_json(sb.resolve("outer", review_id, Some(follow_id)));
    let state = sb.state();
    assert!(state["escalated"].as_array().unwrap().is_empty());
    assert_eq!(state["unreceived"][0]["id"], follow_id);
    failed(sb.resolve("outer", review_id, None), "already resolved");

    success_json(sb.receive("runtime", follow_id));
    let body = sb.file("sha256-ack.txt", b"Committed in a SHA-256 repository");
    let ack = success_json(sb.event(&[
        "ack",
        "--actor",
        "runtime",
        "--item",
        follow_id,
        "--outcome",
        "done",
        "--r2",
        "verified",
        "--commit",
        &"a".repeat(64),
        "--body-file",
        body.to_str().unwrap(),
    ]));
    let accepted = success_json(sb.review("outer", ack["event"].as_str().unwrap(), "accept"));
    failed(
        sb.resolve("outer", accepted["event"].as_str().unwrap(), None),
        "Only an escalated review",
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-inbox.py and olp-board-sentinel.py with --since-head.
/// - Production path: projection entries are filtered by the ledger position of the event that made them actionable.
/// - External edges faked: temporary boards, ready files and short polling intervals only.
/// - What this proves: work the caller already saw (an open escalation) no longer wakes the outer loop, while new actionable events still do.
/// - What this intentionally does not exercise: startup catch-up without a baseline, covered by the sentinel tests.
/// - Focused command: cargo test --test olp_board_protocol olp_board_since_head_signals_only_new_actionable_state
#[test]
fn olp_board_since_head_signals_only_new_actionable_state() {
    let sb = Sandbox::new("since-head");
    let first = sb.item("1", "Escalated", "outer", "runtime");
    let first_id = first["event"].as_str().unwrap();
    success_json(sb.receive("runtime", first_id));
    let ack = success_json(sb.ack("runtime", first_id, "blocked"));
    success_json(sb.review("outer", ack["event"].as_str().unwrap(), "escalate"));
    let second = sb.item("2", "Still running", "outer", "runtime");
    let second_id = second["event"].as_str().unwrap();
    success_json(sb.receive("runtime", second_id));
    let head = sb.state()["head"].as_str().unwrap().to_owned();

    let inbox = |extra: &[&str]| {
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .arg("--board")
            .arg(&sb.board)
            .args(["--for", "outer"])
            .args(extra)
            .output()
            .unwrap()
    };
    assert_eq!(success_json(inbox(&[]))["matched"], true);
    let waited = inbox(&[
        "--wait",
        "--interval",
        "0.05",
        "--timeout",
        "0.5",
        "--since-head",
        &head,
    ]);
    assert_eq!(waited.status.code(), Some(3));
    failed(
        inbox(&["--since-head", "0123456789abcdef0123456789abcdef"]),
        "Unknown --since-head",
    );

    let ready = sb.root.join("since-ready.json");
    let sentinel = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .arg("--board")
        .arg(&sb.board)
        .args([
            "--token",
            "NEVER",
            "--for",
            "outer",
            "--interval",
            "0.02",
            "--timeout",
            "10",
            "--since-head",
            &head,
            "--ready-file",
        ])
        .arg(&ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&ready);
    let second_ack = success_json(sb.ack("runtime", second_id, "done"));
    let output = sentinel.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: Value =
        serde_json::from_str(stdout.strip_prefix("LEDGER-SIGNAL: ").unwrap().trim()).unwrap();
    let ids: Vec<&str> = payload["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![second_ack["event"].as_str().unwrap()]);

    // A runtime baseline hides only its own in-flight item, never queued work.
    let queued = sb.item("3", "Queued", "outer", "runtime");
    let working = sb.item("4", "Working", "outer", "runtime");
    let working_id = working["event"].as_str().unwrap();
    success_json(sb.receive("runtime", working_id));
    let runtime_head = sb.state()["head"].as_str().unwrap().to_owned();
    let runtime = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .arg("--board")
            .arg(&sb.board)
            .args([
                "--for",
                "runtime",
                "--actor",
                "runtime",
                "--since-head",
                &runtime_head,
            ])
            .output()
            .unwrap(),
    );
    let ids: Vec<&str> = runtime["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![queued["event"].as_str().unwrap()]);
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run.
/// - Production path: the lock-free opt-in precheck keeps legacy boards on the unchanged legacy scanner.
/// - External edges faked: a recording python3 shim on PATH, a lock holder process, and temporary boards.
/// - What this proves: a legacy board with the `.lock` from the legacy helper is harvested exactly like the legacy path, without Python or the board lock, and a stray quoted event line with no valid event is replayed but still falls back to identical legacy output.
/// - What this intentionally does not exercise: structured harvest, covered by the attribution tests.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_keeps_legacy_boards_byte_identical
#[test]
fn olp_board_harvest_keeps_legacy_boards_byte_identical() {
    let sb = Sandbox::new("harvest-legacy-identical");
    assert!(
        sb.shell_append(b"### 7. Legacy task\nACK(blocked): legacy condition\n")
            .status
            .success()
    );
    let missing_tool = sb.root.join("no-event-tool.py");
    let reference = sb.harvest(&missing_tool, None);
    assert!(reference.status.success());
    let expected = mask_timestamps(&String::from_utf8_lossy(&reference.stdout));
    assert_eq!(expected.matches("### EVO-").count(), 1);

    let shim = sb.root.join("shim");
    fs::create_dir_all(&shim).unwrap();
    let marker = sb.root.join("python-was-called");
    let shim_python = shim.join("python3");
    fs::write(
        &shim_python,
        format!("#!/bin/sh\ntouch '{}'\nexit 99\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&shim_python, fs::Permissions::from_mode(0o755)).unwrap();
    let mut holder = hold_lock(&sb, "exclusive");
    let started = Instant::now();
    let actual = sb.harvest(&script("olp-board-event.py"), Some(&shim));
    let elapsed = started.elapsed();
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert!(
        actual.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&actual.stderr)
    );
    // A lock wait would take at least the 10 s default timeout.
    assert!(
        elapsed < Duration::from_secs(8),
        "harvest waited {elapsed:?}"
    );
    assert!(!marker.exists(), "legacy harvest must not start python3");
    assert_eq!(
        mask_timestamps(&String::from_utf8_lossy(&actual.stdout)),
        expected
    );

    assert!(
        sb.shell_append(b"> OLP-EVENT {copied from the docs}\n")
            .status
            .success()
    );
    // A board with an event line no longer degrades silently without the event
    // tool (REQ-OLP-BOARD-HARVEST), so that run cannot serve as the reference.
    // The quoted line triggers nothing, so the reference taken before it holds.
    let degraded = sb.harvest(&missing_tool, None);
    assert!(!degraded.status.success());
    let actual = sb.harvest(&script("olp-board-event.py"), None);
    assert!(
        actual.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        mask_timestamps(&String::from_utf8_lossy(&actual.stdout)),
        expected
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state and item CLIs.
/// - Production path: readers take the board lock shared and only while copying bytes; writers need it exclusively.
/// - External edges faked: a lock holder process and temporary boards.
/// - What this proves: polling readers never block each other, and the lock still serialises writers.
/// - What this intentionally does not exercise: replay cost on large boards.
/// - Focused command: cargo test --test olp_board_protocol olp_board_readers_share_the_lock_and_writers_wait
#[test]
fn olp_board_readers_share_the_lock_and_writers_wait() {
    let sb = Sandbox::new("shared-lock");
    sb.item("1", "Seed", "outer", "runtime");
    let mut shared = hold_lock(&sb, "shared");
    let state = success_json(sb.event(&["state", "--lock-timeout", "2"]));
    assert_eq!(state["events"].as_array().unwrap().len(), 1);
    failed(
        sb.event(&[
            "item",
            "--actor",
            "outer",
            "--number",
            "2",
            "--title",
            "Blocked writer",
            "--lock-timeout",
            "0.2",
            "--body-file",
            sb.file("blocked-item.txt", b"Wait.\n").to_str().unwrap(),
        ]),
        "Board lock unavailable",
    );
    shared.kill().unwrap();
    shared.wait().unwrap();
    let mut exclusive = hold_lock(&sb, "exclusive");
    failed(
        sb.event(&["state", "--lock-timeout", "0.2"]),
        "Board lock unavailable",
    );
    exclusive.kill().unwrap();
    exclusive.wait().unwrap();
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-append.py append, event state and ack CLIs.
/// - Production path: the append guard and replay share one LF-only line model.
/// - External edges faked: temporary boards only.
/// - What this proves: a bare CR can neither smuggle an event line past the append guard nor create DRIFT that cannot be recovered.
/// - What this intentionally does not exercise: CRLF fences, covered by the fence tests.
/// - Focused command: cargo test --test olp_board_protocol olp_board_replay_splits_lines_on_lf_only
#[test]
fn olp_board_replay_splits_lines_on_lf_only() {
    let sb = Sandbox::new("lf-only");
    let item = sb.item("1", "Carriage returns", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    success_json(sb.receive("runtime", item_id));
    let append = |body: &[u8]| {
        let file = sb.file("cr-note.txt", body);
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-append.py"))
            .args(["append", "--board"])
            .arg(&sb.board)
            .arg("--body-file")
            .arg(file)
            .output()
            .unwrap()
    };
    success_json(append(b"progress 50%\r> OLP-EVENT {}\n"));
    let state = sb.state();
    assert!(state["drift"].as_array().unwrap().is_empty(), "{state}");
    assert_eq!(state["events"].as_array().unwrap().len(), 2);

    let line = b"log line\rACK(done): stray\n";
    let offset = sb.len();
    success_json(append(line));
    let state = sb.state();
    assert_eq!(drift_kinds(&state), vec!["suspected_ack"]);
    assert_eq!(state["drift"][0]["length"], line.len());
    success_json(sb.ack_recovering(item_id, "done", &source_ref(offset, line)));
    assert!(sb.state()["drift"].as_array().unwrap().is_empty());
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run on a structured board.
/// - Production path: structured ACK rows are `|`-delimited before the unchanged card emitter reads them.
/// - External edges faked: temporary boards and an absent MCP source only.
/// - What this proves: free-form display numbers containing `|` keep every card identity and envelope well formed.
/// - What this intentionally does not exercise: evolution-board commits.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_escapes_pipes_in_item_numbers
#[test]
fn olp_board_harvest_escapes_pipes_in_item_numbers() {
    let sb = Sandbox::new("harvest-pipes");
    for number in ["A|1", "A|2"] {
        let item = sb.item(number, "Piped number", "outer", "runtime");
        let item_id = item["event"].as_str().unwrap();
        success_json(sb.receive("runtime", item_id));
        success_json(sb.ack("runtime", item_id, "blocked"));
    }
    let output = sb.harvest(&script("olp-board-event.py"), None);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(text.matches("### EVO-").count(), 2, "{text}");
    assert!(text.contains("#A¦1#blocked#"), "{text}");
    assert!(text.contains("#A¦2#blocked#"), "{text}");
    for envelope in text.lines().filter(|line| line.starts_with("envelope: ")) {
        let fields: Vec<&str> = envelope["envelope: ".len()..].split(' ').collect();
        assert_eq!(fields.len(), 3, "{envelope}");
        assert!(
            fields[0]
                .strip_prefix("line=")
                .unwrap()
                .parse::<u64>()
                .is_ok()
        );
        assert!(
            fields[1]
                .strip_prefix("offset=")
                .unwrap()
                .parse::<u64>()
                .is_ok()
        );
        assert!(fields[2].starts_with("ts="), "{envelope}");
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event item/state and append CLIs.
/// - Production path: board path canonicalisation shared by every tool.
/// - External edges faked: a symlink in a temporary directory.
/// - What this proves: a symlinked board path, which would split the lock domain from the legacy shell, is refused before any effect.
/// - What this intentionally does not exercise: directory symlinks, which keep the lock beside the board and stay allowed.
/// - Focused command: cargo test --test olp_board_protocol olp_board_tools_refuse_symlinked_boards
#[test]
fn olp_board_tools_refuse_symlinked_boards() {
    let sb = Sandbox::new("symlink");
    sb.item("1", "Real path", "outer", "runtime");
    let link = sb.root.join("LINK.md");
    std::os::unix::fs::symlink(&sb.board, &link).unwrap();
    let before = fs::read(&sb.board).unwrap();
    let body = sb.file("link-item.txt", b"Through a link.\n");
    let via_link = |args: &[&str], tool: &str| {
        Command::new("python3")
            .arg("-B")
            .arg(script(tool))
            .args(args)
            .arg("--board")
            .arg(&link)
            .output()
            .unwrap()
    };
    failed(
        via_link(
            &[
                "item",
                "--actor",
                "outer",
                "--number",
                "2",
                "--title",
                "Link",
                "--body-file",
                body.to_str().unwrap(),
            ],
            "olp-board-event.py",
        ),
        "symlink",
    );
    failed(via_link(&["state"], "olp-board-event.py"), "symlink");
    failed(
        via_link(
            &["append", "--body-file", body.to_str().unwrap()],
            "olp-board-append.py",
        ),
        "symlink",
    );
    let mut child = Command::new("bash")
        .arg(script("olp-board-append.sh"))
        .arg(&link)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Through a link.\n")
        .unwrap();
    failed(child.wait_with_output().unwrap(), "symlink");
    assert!(
        !sb.root.join("LINK.md.lock").exists(),
        "the shell must not lock beside the link"
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);
}

/// The harvest script needs GNU flock and `stat -c` outside dry-run; say so
/// explicitly and skip on hosts without them (CI runs the full path).
fn gnu_harvest_tooling_or_skip() -> bool {
    let flock = Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let stat_c = Command::new("stat")
        .args(["-c", "%s", "/dev/null"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !(flock && stat_c) {
        eprintln!("SKIP (explicit): host lacks GNU flock/stat -c; harvest script requires them");
    }
    flock && stat_c
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh committing to a real evolution board, twice.
/// - Production path: the structured card for a recovering ACK reuses the legacy identity of the recovered line.
/// - External edges faked: temporary boards, state directory and an absent MCP source only.
/// - What this proves: a hand-written blocked ACK carded before its recovery does not produce a second card once a formal ACK adopts it.
/// - What this intentionally does not exercise: dry-run output shape, covered by the recovered-ACK dedup test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_keeps_card_identity_after_recovery
#[test]
fn olp_board_harvest_keeps_card_identity_after_recovery() {
    if !gnu_harvest_tooling_or_skip() {
        return;
    }
    let sb = Sandbox::new("harvest-identity");
    let item = sb.item("1", "Recovered later", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    success_json(sb.receive("runtime", item_id));
    let line = b"ACK(blocked): dependency unavailable\n";
    let offset = sb.len();
    assert!(sb.shell_append(line).status.success());
    let repo_root = sb.root.join("repo");
    fs::create_dir_all(&repo_root).unwrap();
    let evolution = sb.root.join("EVOLUTION.md");
    let harvest = || {
        Command::new("bash")
            .arg(script("olp-evo-harvest.sh"))
            .arg(&repo_root)
            .env("OLP_EVO_REVIEW_BOARD", &sb.board)
            .env("OLP_EVO_MCP_BOARD", sb.root.join("missing-mcp.md"))
            .env("OLP_EVO_STATE", sb.root.join("harvest-state"))
            .env("OLP_EVO_BOARD", &evolution)
            .env("OLP_BOARD_EVENT_TOOL", script("olp-board-event.py"))
            .output()
            .unwrap()
    };
    let first = harvest();
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    let cards = fs::read_to_string(&evolution).unwrap();
    assert_eq!(cards.matches("### EVO-").count(), 1, "{cards}");

    success_json(sb.ack_recovering(item_id, "blocked", &source_ref(offset, line)));
    let second = harvest();
    assert!(
        second.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    let cards = fs::read_to_string(&evolution).unwrap();
    assert_eq!(cards.matches("### EVO-").count(), 1, "{cards}");
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state and void CLIs.
/// - Production path: replay checks where an event source starts and whether it overlaps paired or consumed text.
/// - External edges faked: raw appends stand in for a writer that bypasses every guard.
/// - What this proves: a source may neither swallow a paired record nor start mid-line, so no byte range is paired twice or paired and voided at once.
/// - What this intentionally does not exercise: the writers, which only ever append fresh sources.
/// - Focused command: cargo test --test olp_board_protocol olp_board_replay_quarantines_misaligned_and_overlapping_sources
#[test]
fn olp_board_replay_quarantines_misaligned_and_overlapping_sources() {
    let sb = Sandbox::new("source-ranges");
    sb.raw_append(b"### B. decoy\n");
    sb.item("A", "Real", "outer", "runtime");
    let quoted_at = sb.len();
    sb.raw_append(b"> ACK(done): quoted, not a real ACK\n");
    let state = sb.state();
    let real = state["events"][0].clone();
    let head = state["head"].clone();

    // An item whose source starts at the top of the board swallows the paired record.
    let bytes = fs::read(&sb.board).unwrap();
    let mut swallowing = real.clone();
    swallowing["id"] = json!("b".repeat(32));
    swallowing["prev"] = head.clone();
    swallowing["number"] = json!("B");
    swallowing["title"] = json!("decoy");
    swallowing["source"] = source_ref(0, &bytes);
    let ts = real["ts"].as_str().unwrap();
    sb.raw_append(
        format!(
            "> OLP-EVENT {}\nts={ts}\n",
            serde_json::to_string(&swallowing).unwrap()
        )
        .as_bytes(),
    );

    // An item whose source starts in the middle of a prose line.
    let prose_at = sb.len();
    let prose = b"note ### C. mid-line\nBody.\n";
    sb.raw_append(prose);
    let mut misaligned = real.clone();
    misaligned["id"] = json!("c".repeat(32));
    misaligned["prev"] = head.clone();
    misaligned["number"] = json!("C");
    misaligned["title"] = json!("mid-line");
    misaligned["source"] = source_ref(prose_at + 5, &prose[5..]);
    sb.raw_append(
        format!(
            "> OLP-EVENT {}\nts={ts}\n",
            serde_json::to_string(&misaligned).unwrap()
        )
        .as_bytes(),
    );

    let state = sb.state();
    assert_eq!(state["events"].as_array().unwrap().len(), 1, "{state}");
    assert_eq!(state["head"], head);
    let reasons: Vec<String> = state["drift"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "malformed_event")
        .map(|entry| entry["reason"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(reasons.len(), 2, "{state}");
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("overlaps text already paired or consumed")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("must start at a line boundary")),
        "{reasons:?}"
    );

    // The quoted ACK inside the rejected source is not paired: it is voided exactly once.
    let quoted = state["drift"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["offset"] == json!(quoted_at))
        .unwrap_or_else(|| panic!("quoted ACK is not drift in {state}"))
        .clone();
    success_json(sb.void(&quoted));
    failed(sb.void(&quoted), "already consumed");
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run.
/// - Production path: the opt-in gate in front of the structured and legacy board scanners.
/// - External edges faked: a missing event tool path and a renamed board lock.
/// - What this proves: a board with event lines is never scanned by the legacy scanner just because a tool or the lock is missing, since that changes each ACK's identity and cards it twice.
/// - What this intentionally does not exercise: boards without event lines, which stay on the legacy scanner.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_refuses_to_degrade_structured_boards
#[test]
fn olp_board_harvest_refuses_to_degrade_structured_boards() {
    let sb = Sandbox::new("harvest-no-degrade");
    let item = sb.item("R8-1", "Blocked work", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap();
    success_json(sb.receive("runtime", item_id));
    success_json(sb.ack("runtime", item_id, "blocked"));

    let missing_tool = sb.root.join("no-event-tool.py");
    let without_tool = sb.harvest(&missing_tool, None);
    assert!(!without_tool.status.success());
    assert!(String::from_utf8_lossy(&without_tool.stderr).contains("event tool"));
    assert!(!String::from_utf8_lossy(&without_tool.stdout).contains("### EVO-"));

    let lock = sb.root.join("BOARD.md.lock");
    let parked = sb.root.join("BOARD.md.lock.parked");
    fs::rename(&lock, &parked).unwrap();
    let without_lock = sb.harvest(&script("olp-board-event.py"), None);
    fs::rename(&parked, &lock).unwrap();
    assert!(!without_lock.status.success());
    assert!(String::from_utf8_lossy(&without_lock.stderr).contains(".lock"));
    assert!(!String::from_utf8_lossy(&without_lock.stdout).contains("### EVO-"));

    let complete = sb.harvest(&script("olp-board-event.py"), None);
    assert!(
        complete.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&complete.stderr)
    );
    let text = String::from_utf8_lossy(&complete.stdout);
    assert_eq!(text.matches("### EVO-").count(), 1, "{text}");
    assert!(text.contains("#R8-1#blocked#"), "{text}");
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run, twice.
/// - Production path: the structured path reads one snapshot for replay, the legacy scan and the structured scan.
/// - External edges faked: an event-tool wrapper that forwards every call to the real tool and, once, appends a real ACK right after its second call returns.
/// - What this proves: an ACK appended while a run is in progress is left for the next run, which cards it once under its structured identity; it is never carded under a legacy identity first.
/// - What this intentionally does not exercise: the gate's own replay, which only picks the path.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_reads_one_snapshot_per_run
#[test]
fn olp_board_harvest_reads_one_snapshot_per_run() {
    let sb = Sandbox::new("harvest-snapshot");
    let item = sb.item("R8-1", "Blocked work", "outer", "runtime");
    let item_id = item["event"].as_str().unwrap().to_owned();
    success_json(sb.receive("runtime", &item_id));
    let body = sb.file("late-ack.txt", b"dependency unavailable");
    let pending = sb.file("inject-ack", item_id.as_bytes());
    let calls = sb.root.join("wrapper-calls");
    let wrapper = sb.file(
        "event-tool-wrapper.py",
        format!(
            r#"import os, subprocess, sys
real, board, body, pending, calls = {real:?}, {board:?}, {body:?}, {pending:?}, {calls:?}
result = subprocess.run([sys.executable, "-B", real, *sys.argv[1:]])
count = int(open(calls).read()) + 1 if os.path.exists(calls) else 1
open(calls, "w").write(str(count))
if count == 2 and os.path.exists(pending):
    item = open(pending).read().strip()
    os.remove(pending)
    subprocess.run([sys.executable, "-B", real, "ack", "--board", board, "--actor", "runtime",
                    "--item", item, "--outcome", "blocked", "--r2", "verified",
                    "--commit", "0123456789abcdef0123456789abcdef01234567", "--body-file", body],
                   check=True, capture_output=True)
sys.exit(result.returncode)
"#,
            real = script("olp-board-event.py").to_str().unwrap(),
            board = sb.board.to_str().unwrap(),
            body = body.to_str().unwrap(),
            pending = pending.to_str().unwrap(),
            calls = calls.to_str().unwrap(),
        )
        .as_bytes(),
    );

    let identities = |output: &Output| -> Vec<String> {
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("identity: ").map(str::to_owned))
            .collect()
    };
    let first = identities(&sb.harvest(&wrapper, None));
    assert!(
        !pending.exists(),
        "the wrapper did not append the ACK during the first run"
    );
    let second = identities(&sb.harvest(&wrapper, None));
    assert!(
        first.is_empty(),
        "an ACK appended mid-run was carded in the same run: {first:?}"
    );
    assert_eq!(second.len(), 1, "{second:?}");
    assert!(second[0].contains("#R8-1#blocked#"), "{second:?}");
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run.
/// - Production path: the structured path's candidate merge and its temporary directory.
/// - External edges faked: a temporary TMPDIR, and an event-tool wrapper that returns a corrupt snapshot state in the second run.
/// - What this proves: a legacy trigger line with bytes that are not UTF-8 (the legacy shell appender never validates) still cards on an opted-in board, and a failing run reports an error line and leaves no temporary directory.
/// - What this intentionally does not exercise: boards without event lines, which never reach the merge.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_keeps_non_utf8_triggers_and_cleans_up
#[test]
fn olp_board_harvest_keeps_non_utf8_triggers_and_cleans_up() {
    let sb = Sandbox::new("harvest-non-utf8");
    let item = sb.item("R8-1", "Blocked work", "outer", "runtime");
    success_json(sb.receive("runtime", item["event"].as_str().unwrap()));
    let line: &[u8] = b"ACK(blocked): cr\xe9er avec bytes \xff invalid";
    let mut appended = line.to_vec();
    appended.push(b'\n');
    assert!(sb.shell_append(&appended).status.success());

    let tmp = sb.root.join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    let repo_root = sb.root.join("repo");
    fs::create_dir_all(&repo_root).unwrap();
    let run = |tool: &PathBuf| {
        Command::new("bash")
            .arg(script("olp-evo-harvest.sh"))
            .arg(&repo_root)
            .arg("--dry-run")
            .env("OLP_EVO_REVIEW_BOARD", &sb.board)
            .env("OLP_EVO_MCP_BOARD", sb.root.join("missing-mcp.md"))
            .env("OLP_EVO_STATE", sb.root.join("harvest-state"))
            .env("OLP_EVO_BOARD", sb.root.join("EVOLUTION.md"))
            .env("OLP_BOARD_EVENT_TOOL", tool)
            .env("TMPDIR", &tmp)
            .output()
            .unwrap()
    };
    let leftovers = || {
        fs::read_dir(&tmp)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("olp-evo-board")
            })
            .count()
    };

    let carded = run(&script("olp-board-event.py"));
    assert!(
        carded.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&carded.stderr)
    );
    let text = String::from_utf8_lossy(&carded.stdout);
    assert_eq!(text.matches("### EVO-").count(), 1, "{text}");
    assert!(
        text.contains(&format!("#-1#blocked#{}", sha256_hex(line))),
        "{text}"
    );
    assert_eq!(leftovers(), 0);

    let wrapper = sb.file(
        "corrupt-snapshot.py",
        format!(
            r#"import subprocess, sys
real = {real:?}
if sys.argv[1:2] == ["snapshot"]:
    result = subprocess.run([sys.executable, "-B", real, *sys.argv[1:]], capture_output=True)
    sys.stdout.write('{{"events": [], "drift": [], "fenced_ranges": 7, "quarantine_evidence": [], "recovery_evidence": []}}\n')
    sys.exit(result.returncode)
sys.exit(subprocess.run([sys.executable, "-B", real, *sys.argv[1:]]).returncode)
"#,
            real = script("olp-board-event.py").to_str().unwrap(),
        )
        .as_bytes(),
    );
    let failed_run = run(&wrapper);
    assert!(!failed_run.status.success());
    assert!(
        String::from_utf8_lossy(&failed_run.stderr).contains("error:"),
        "stderr={}",
        String::from_utf8_lossy(&failed_run.stderr)
    );
    assert_eq!(
        leftovers(),
        0,
        "a failed harvest left its temporary directory behind"
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event record, verify, item and void CLIs.
/// - Production path: strict JSON loading of every JSON input file and the CLI error contract.
/// - External edges faked: a generated JSON file nested 200,000 levels deep.
/// - What this proves: the recursion error such input raises is reported like any other input error, as one machine JSON on stderr with exit 2, and nothing is written.
/// - What this intentionally does not exercise: deep JSON on the board itself, which replay already turns into malformed_event.
/// - Focused command: cargo test --test olp_board_protocol olp_board_event_cli_reports_deep_json_as_one_machine_error
#[test]
fn olp_board_event_cli_reports_deep_json_as_one_machine_error() {
    let sb = Sandbox::new("deep-json");
    sb.item("1", "First", "outer", "runtime");
    let deep = sb.file(
        "deep.json",
        format!("{}{}", "[".repeat(200_000), "]".repeat(200_000)).as_bytes(),
    );
    let record_body = sb.file("deep-record.txt", b"### 2. Second\nBody.\n");
    let item_body = sb.file("deep-item.txt", b"Body.\n");
    let explanation = sb.file("deep-void.txt", b"quarantine");
    let board = sb.board.to_str().unwrap().to_owned();
    let deep = deep.to_str().unwrap().to_owned();
    let before = fs::read(&sb.board).unwrap();
    let invocations: Vec<Vec<String>> = vec![
        vec![
            "record",
            "--board",
            &board,
            "--event-file",
            &deep,
            "--body-file",
            record_body.to_str().unwrap(),
        ],
        vec!["verify", "--receipt-file", &deep],
        vec![
            "item",
            "--board",
            &board,
            "--actor",
            "outer",
            "--number",
            "2",
            "--title",
            "Second",
            "--to",
            "runtime",
            "--body-file",
            item_body.to_str().unwrap(),
            "--recovery-file",
            &deep,
        ],
        vec![
            "void",
            "--board",
            &board,
            "--actor",
            "outer",
            "--target-file",
            &deep,
            "--body-file",
            explanation.to_str().unwrap(),
        ],
    ]
    .into_iter()
    .map(|args| args.into_iter().map(str::to_owned).collect())
    .collect();
    for args in invocations {
        let output = Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-event.py"))
            .args(&args)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?} stderr={stderr}");
        let lines: Vec<&str> = stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 1, "{args:?} stderr={stderr}");
        let error: Value = serde_json::from_str(lines[0]).unwrap();
        assert!(error["error"].is_string(), "{args:?} {error}");
        assert_eq!(fs::read(&sb.board).unwrap(), before, "{args:?}");
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state, runtime inbox, sentinel and olp-evo-harvest.sh --dry-run.
/// - Production path: a malformed event line's reason flows into every consumer's JSON output.
/// - External edges faked: a raw file append stands in for a writer that bypasses every guard.
/// - What this proves: an event line whose JSON repeats a key spelled as a lone surrogate escape is one malformed_event like any other; no consumer fails on the whole board because the reason cannot be printed.
/// - What this intentionally does not exercise: voiding the line, covered by the void test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_drift_reasons_stay_printable_for_crafted_json_keys
#[test]
fn olp_board_drift_reasons_stay_printable_for_crafted_json_keys() {
    let sb = Sandbox::new("surrogate-key");
    sb.item("1", "First", "outer", "runtime");
    let offset = sb.len();
    sb.raw_append(b"> OLP-EVENT {\"\\ud800\":1,\"\\ud800\":2}\n");

    let state = sb.state();
    let malformed: Vec<&Value> = state["drift"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "malformed_event")
        .collect();
    assert_eq!(malformed.len(), 1, "{state}");
    assert_eq!(malformed[0]["offset"], json!(offset));
    assert!(
        malformed[0]["reason"]
            .as_str()
            .unwrap()
            .contains("Duplicate JSON key: \"\\ud800\""),
        "{state}"
    );

    let inbox = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .arg("--board")
            .arg(&sb.board)
            .args(["--for", "runtime"])
            .output()
            .unwrap(),
    );
    assert_eq!(inbox["dispatch_blocked"], json!(true), "{inbox}");

    let sentinel = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .arg("--board")
        .arg(&sb.board)
        .args(["--token", "unused", "--for", "runtime"])
        .output()
        .unwrap();
    assert!(
        sentinel.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&sentinel.stdout),
        String::from_utf8_lossy(&sentinel.stderr)
    );
    assert!(String::from_utf8_lossy(&sentinel.stdout).starts_with("DRIFT: "));

    let harvest = sb.harvest(&script("olp-board-event.py"), None);
    assert!(
        harvest.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&harvest.stderr)
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-sentinel.py polling loop and scripts/olp-board-append.sh.
/// - Production path: the sentinel's scan of text appended after its baseline.
/// - External edges faked: temporary boards and a ready file only.
/// - What this proves: a line from the legacy shell appender, which never validates UTF-8, wakes the sentinel with BOARD-SIGNAL instead of ending it with ERROR.
/// - What this intentionally does not exercise: ledger signals, covered by the dynamic-signal test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_sentinel_matches_tokens_in_non_utf8_text
#[test]
fn olp_board_sentinel_matches_tokens_in_non_utf8_text() {
    let sb = Sandbox::new("sentinel-bytes");
    sb.item("1", "First", "outer", "runtime");
    let ready = sb.root.join("ready.json");
    let watcher = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .arg("--board")
        .arg(&sb.board)
        .args([
            "--token",
            "WAKE-TOKEN",
            "--for",
            "outer",
            "--interval",
            "0.02",
            "--timeout",
            "10",
            "--ready-file",
        ])
        .arg(&ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&ready);
    assert!(
        sb.shell_append(b"WAKE-TOKEN from a legacy writer: caf\xe9\n")
            .status
            .success()
    );
    let output = watcher.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let signal: Value = serde_json::from_str(
        stdout
            .strip_prefix("BOARD-SIGNAL: ")
            .unwrap_or_else(|| panic!("no BOARD-SIGNAL in {stdout}"))
            .trim(),
    )
    .unwrap();
    assert_eq!(
        signal["matches"],
        json!(["WAKE-TOKEN from a legacy writer: caf\u{fffd}"])
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression, under every bash on the host.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run, twice, under each distinct bash (the one on PATH and /bin/bash; on macOS the latter is 3.2).
/// - Production path: the candidate merge places legacy rows by line number, and a recovering ACK adopts the identity the legacy scanner gave the recovered line.
/// - External edges faked: temporary boards only; the harvest script and event tool are not replaced.
/// - What this proves: whether bash `read` drops a NUL byte (4+) or cuts the line there (3.2), a NUL on the board neither cards one ACK twice across its recovery nor misplaces the offset of a later legacy trigger.
/// - What this intentionally does not exercise: committing cards, covered by the identity-after-recovery test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_ignores_nul_bytes_like_the_legacy_scanner
#[test]
fn olp_board_harvest_ignores_nul_bytes_like_the_legacy_scanner() {
    for shell in harvest_shells() {
        let sb = Sandbox::new("harvest-nul");
        let item = sb.item("8", "Blocked work", "outer", "runtime");
        let item_id = item["event"].as_str().unwrap();
        success_json(sb.receive("runtime", item_id));
        let line: &[u8] = b"ACK(blocked): power\0 supply lost\n";
        let offset = sb.len();
        assert!(sb.shell_append(line).status.success());
        let first = evo_cards(sb.harvest_under(&shell));
        assert_eq!(first.len(), 1, "{shell:?} {first:?}");
        assert!(first[0].contains("#8#blocked#"), "{shell:?} {first:?}");
        let identity = card_identity(&first[0]);

        success_json(sb.ack_recovering(item_id, "blocked", &source_ref(offset, line)));
        let signed_at = sb.len();
        assert!(
            sb.shell_append("> 外环(outer)·R2 记档(#8): checked by hand\n".as_bytes())
                .status
                .success()
        );
        let second = evo_cards(sb.harvest_under(&shell));
        assert_eq!(second.len(), 2, "{shell:?} {second:?}");
        let blocked: Vec<&String> = second
            .iter()
            .filter(|card| card.contains("#blocked#"))
            .collect();
        assert_eq!(blocked.len(), 1, "{shell:?} {second:?}");
        assert_eq!(card_identity(blocked[0]), identity, "{shell:?}");
        let signed = second
            .iter()
            .find(|card| card.contains("trigger: r2_record"))
            .unwrap_or_else(|| panic!("no R2 card in {second:?}"));
        assert!(
            signed.contains(&format!(" offset={signed_at} ")),
            "{shell:?} {signed}"
        );
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: scripts/olp-board-append.sh.
/// - Production path: the legacy helper's opted-in guard, under its own lock.
/// - External edges faked: temporary boards; a raw append stands in for a writer that ignores the lock.
/// - What this proves: on an opted-in board the legacy helper, like the Python appender, neither leaves a partial line nor continues one, so two appends cannot splice an event line; legacy boards keep the old behaviour.
/// - What this intentionally does not exercise: voiding the partial line, covered by the void test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_legacy_shell_keeps_lines_whole_on_opted_in_boards
#[test]
fn olp_board_legacy_shell_keeps_lines_whole_on_opted_in_boards() {
    let legacy = Sandbox::new("shell-whole-legacy");
    assert!(legacy.shell_append(b"> OLP-").status.success());
    assert!(legacy.shell_append(b"EVENT {spliced}\n").status.success());
    assert_eq!(fs::read(&legacy.board).unwrap(), b"> OLP-EVENT {spliced}\n");

    let sb = Sandbox::new("shell-whole");
    sb.item("1", "Opted in", "outer", "runtime");
    let before = fs::read(&sb.board).unwrap();
    for body in [b"> OLP-".as_slice(), b"note without newline".as_slice()] {
        let refused = sb.shell_append(body);
        assert_eq!(refused.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("must end with a newline"),
            "stderr={}",
            String::from_utf8_lossy(&refused.stderr)
        );
        assert_eq!(fs::read(&sb.board).unwrap(), before);
    }
    assert!(sb.shell_append(b"EVENT {spliced}\n").status.success());
    assert_eq!(drift_kinds(&sb.state()), Vec::<String>::new());

    sb.raw_append(b"partial line from a writer that ignores the lock");
    let tail = fs::read(&sb.board).unwrap();
    let refused = sb.shell_append(b"next entry\n");
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("ends with a partial line"),
        "stderr={}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(fs::read(&sb.board).unwrap(), tail);
}

/// Test Path Statement:
/// - Tier: Real-path regression, under every bash on the host.
/// - Production entrypoint: event state and ack --recovery-file CLIs, and olp-evo-harvest.sh --dry-run under each distinct bash.
/// - Production path: replay's NUL-blind normative classification and recovery matching, the legacy scanner's bash `read`, and the recovered identity.
/// - External edges faked: temporary boards only; the harvest script and event tool are not replaced.
/// - What this proves: a NUL byte splitting the ACK keyword always blocks dispatch, harvest cards the line once when its bash reads the keyword (4+) and not at all when it cuts the line (3.2), and the ACK has one identity across its recovery either way.
/// - What this intentionally does not exercise: NUL inside an explanation, covered by the NUL offset test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_nul_split_ack_blocks_dispatch_and_cards_once
#[test]
fn olp_board_nul_split_ack_blocks_dispatch_and_cards_once() {
    for shell in harvest_shells() {
        let sb = Sandbox::new("nul-split-ack");
        let item = sb.item("8", "Blocked work", "outer", "runtime");
        let item_id = item["event"].as_str().unwrap();
        success_json(sb.receive("runtime", item_id));
        let line: &[u8] = b"A\0CK(blocked): hidden by NUL\n";
        let offset = sb.len();
        assert!(sb.shell_append(line).status.success());

        let state = sb.state();
        assert_eq!(state["dispatch_blocked"], json!(true), "{state}");
        assert_eq!(drift_kinds(&state), vec!["unpaired_ack".to_owned()]);
        let first = evo_cards(sb.harvest_under(&shell));
        let expected = usize::from(read_drops_nul(&shell));
        assert_eq!(first.len(), expected, "{shell:?} {first:?}");

        success_json(sb.ack_recovering(item_id, "blocked", &source_ref(offset, line)));
        let state = sb.state();
        assert_eq!(drift_kinds(&state), Vec::<String>::new());
        assert_eq!(state["dispatch_blocked"], json!(false), "{state}");
        let second = evo_cards(sb.harvest_under(&shell));
        assert_eq!(second.len(), 1, "{shell:?} {second:?}");
        if let Some(card) = first.first() {
            assert_eq!(card_identity(card), card_identity(&second[0]), "{shell:?}");
        }
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event item/state CLIs, olp-board-append.py append and scripts/olp-board-append.sh.
/// - Production path: the board path check shared by the Python tools, and the legacy helper's opted-in guard.
/// - External edges faked: temporary boards and hard links only.
/// - What this proves: a board reachable under two names, each locking its own `<name>.lock`, is refused by every tool, and the legacy helper creates no lock beside the extra name; with one name again everything works.
/// - What this intentionally does not exercise: symlinked boards, covered by their own test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_tools_refuse_hard_linked_boards
#[test]
fn olp_board_tools_refuse_hard_linked_boards() {
    let sb = Sandbox::new("hard-link");
    sb.item("1", "Opted in", "outer", "runtime");
    let alias = sb.root.join("ALIAS.md");
    fs::hard_link(&sb.board, &alias).unwrap();
    fs::write(sb.root.join("ALIAS.md.lock"), b"").unwrap();
    let unlocked = sb.root.join("UNLOCKED.md");
    fs::hard_link(&sb.board, &unlocked).unwrap();
    let before = fs::read(&sb.board).unwrap();
    let body = sb.file("hard-link-body.txt", b"Body.\n");
    let python = |tool: &str, args: &[&str], board: &PathBuf| {
        Command::new("python3")
            .arg("-B")
            .arg(script(tool))
            .args(args)
            .arg("--board")
            .arg(board)
            .output()
            .unwrap()
    };
    let shell = |board: &PathBuf| {
        let mut child = Command::new("bash")
            .arg(script("olp-board-append.sh"))
            .arg(board)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"note\n").unwrap();
        child.wait_with_output().unwrap()
    };
    let item_args = [
        "item",
        "--actor",
        "outer",
        "--number",
        "2",
        "--title",
        "Second",
        "--body-file",
        body.to_str().unwrap(),
    ];
    let append_args = ["append", "--body-file", body.to_str().unwrap()];
    for board in [&sb.board, &alias, &unlocked] {
        failed(python("olp-board-event.py", &item_args, board), "hard link");
        failed(python("olp-board-event.py", &["state"], board), "hard link");
        failed(
            python("olp-board-append.py", &append_args, board),
            "hard link",
        );
        let refused = shell(board);
        assert_eq!(refused.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("hard link"),
            "stderr={}",
            String::from_utf8_lossy(&refused.stderr)
        );
        assert_eq!(fs::read(&sb.board).unwrap(), before);
    }
    assert!(!sb.root.join("UNLOCKED.md.lock").exists());

    // The legacy helper counts links as a number: a name that is only a
    // newline, which command substitution strips from printed text, is caught.
    let newline_dir = sb.root.join("newline-name");
    fs::create_dir_all(&newline_dir).unwrap();
    fs::hard_link(&sb.board, newline_dir.join("\n")).unwrap();
    fs::write(newline_dir.join("\n.lock"), b"").unwrap();
    let mut child = Command::new("bash")
        .arg(script("olp-board-append.sh"))
        .arg("\n")
        .current_dir(&newline_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"note\n").unwrap();
    let refused = child.wait_with_output().unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("hard link"),
        "stderr={}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);
    fs::remove_file(newline_dir.join("\n")).unwrap();

    fs::remove_file(&alias).unwrap();
    fs::remove_file(&unlocked).unwrap();
    success_json(python("olp-board-event.py", &item_args, &sb.board));
    assert!(sb.shell_append(b"note\n").status.success());
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-evo-harvest.sh --dry-run.
/// - Production path: the structured path's guard in front of its `|`-separated candidate rows.
/// - External edges faked: temporary boards whose paths contain `|`, an inner newline or a trailing newline.
/// - What this proves: a board path the row protocol cannot carry fails with one physical error line and no traceback, instead of breaking the merge; a trailing newline, which command substitution drops from the resolved path, is caught too.
/// - What this intentionally does not exercise: `|` in item numbers, which are escaped (covered by the pipe test).
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_refuses_board_paths_with_row_separators
#[test]
fn olp_board_harvest_refuses_board_paths_with_row_separators() {
    let sb = Sandbox::new("harvest-pipe-path");
    let body = sb.file("pipe-body.txt", b"Body.\n");
    let why = sb.file("pipe-why.txt", b"dependency unavailable");
    let repo_root = sb.root.join("repo");
    fs::create_dir_all(&repo_root).unwrap();
    for (dir, name) in [
        ("repo|pipe", "BOARD.md"),
        ("inner\nline", "BOARD.md"),
        ("trailing", "BOARD.md\n"),
    ] {
        let dir = sb.root.join(dir);
        fs::create_dir_all(&dir).unwrap();
        let board = dir.join(name);
        fs::write(&board, b"").unwrap();
        fs::write(dir.join(format!("{name}.lock")), b"").unwrap();
        let event = |args: &[&str]| {
            success_json(
                Command::new("python3")
                    .arg("-B")
                    .arg(script("olp-board-event.py"))
                    .args(args)
                    .arg("--board")
                    .arg(&board)
                    .output()
                    .unwrap(),
            )
        };
        let item = event(&[
            "item",
            "--actor",
            "outer",
            "--number",
            "8",
            "--title",
            "Pipe",
            "--body-file",
            body.to_str().unwrap(),
        ]);
        let item_id = item["event"].as_str().unwrap();
        event(&["receive", "--actor", "runtime", "--item", item_id]);
        event(&[
            "ack",
            "--actor",
            "runtime",
            "--item",
            item_id,
            "--outcome",
            "blocked",
            "--r2",
            "unverified",
            "--body-file",
            why.to_str().unwrap(),
        ]);

        let output = Command::new("bash")
            .arg(script("olp-evo-harvest.sh"))
            .arg(&repo_root)
            .arg("--dry-run")
            .env("OLP_EVO_REVIEW_BOARD", &board)
            .env("OLP_EVO_MCP_BOARD", sb.root.join("missing-mcp.md"))
            .env("OLP_EVO_STATE", sb.root.join("harvest-state"))
            .env("OLP_EVO_BOARD", sb.root.join("EVOLUTION.md"))
            .env("OLP_BOARD_EVENT_TOOL", script("olp-board-event.py"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{board:?} stderr={stderr}");
        let lines: Vec<&str> = stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 1, "{board:?} stderr={stderr}");
        assert!(
            lines[0].starts_with("error: ") && lines[0].contains("'|'"),
            "{board:?} stderr={stderr}"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("### EVO-"));
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-event.py and olp-board-append.py command lines.
/// - Production path: argument parsing in front of every write.
/// - External edges faked: temporary boards only.
/// - What this proves: a bad value, an unknown choice, a missing argument or an unknown subcommand is a write-entry failure like any other: exit 2 and exactly one machine JSON with may_have_appended=false, and nothing is written.
/// - What this intentionally does not exercise: --help, which still prints usage and exits 0.
/// - Focused command: cargo test --test olp_board_protocol olp_board_write_clis_report_argument_errors_as_one_machine_json
#[test]
fn olp_board_write_clis_report_argument_errors_as_one_machine_json() {
    let sb = Sandbox::new("argument-errors");
    sb.item("1", "First", "outer", "runtime");
    let before = fs::read(&sb.board).unwrap();
    let body = sb.file("argument-body.txt", b"Body.\n");
    let board = sb.board.to_str().unwrap();
    let body = body.to_str().unwrap();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        (
            "olp-board-event.py",
            vec![
                "item",
                "--board",
                board,
                "--actor",
                "outer",
                "--number",
                "2",
                "--title",
                "Second",
                "--body-file",
                body,
                "--lock-timeout",
                "nan",
            ],
        ),
        (
            "olp-board-event.py",
            vec![
                "ack",
                "--board",
                board,
                "--actor",
                "runtime",
                "--item",
                "x",
                "--outcome",
                "maybe",
                "--r2",
                "verified",
                "--body-file",
                body,
            ],
        ),
        ("olp-board-event.py", vec!["receive", "--board", board]),
        ("olp-board-event.py", vec!["frobnicate"]),
        (
            "olp-board-append.py",
            vec![
                "append",
                "--board",
                board,
                "--body-file",
                body,
                "--lock-timeout",
                "-1",
            ],
        ),
        ("olp-board-append.py", vec!["append", "--board", board]),
    ];
    for (tool, args) in cases {
        let output = Command::new("python3")
            .arg("-B")
            .arg(script(tool))
            .args(&args)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{tool} {args:?} stderr={stderr}"
        );
        let lines: Vec<&str> = stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 1, "{tool} {args:?} stderr={stderr}");
        let error: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(error["may_have_appended"], json!(false), "{tool} {error}");
        assert!(error["error"].is_string(), "{tool} {error}");
        assert_eq!(fs::read(&sb.board).unwrap(), before, "{tool} {args:?}");
    }
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-append.py read_snapshot/run/append_generated (the paths behind state, append and every event writer) and scripts/olp-board-append.sh.
/// - Production path: the link-count check on the opened board once the lock is held.
/// - External edges faked: in the test process only, the production module's lock step and a `flock` shim first on PATH add a second name for the board right as the lock is taken.
/// - What this proves: a name added while a reader or writer awaited the lock is caught before any byte is read or written, not only one that existed when the path was checked.
/// - What this intentionally does not exercise: links made and removed concurrently with a write, which path-derived locks cannot guard.
/// - Focused command: cargo test --test olp_board_protocol olp_board_writers_recheck_links_after_taking_the_lock
#[test]
fn olp_board_writers_recheck_links_after_taking_the_lock() {
    let sb = Sandbox::new("link-after-lock");
    sb.item("1", "Opted in", "outer", "runtime");
    let before = fs::read(&sb.board).unwrap();
    let alias = sb.root.join("LATE.md");
    let body = sb.file("late-body.txt", b"Body.\n");
    let probe = sb.file(
        "late-link-probe.py",
        br#"
import contextlib, importlib.util, json, os, sys
from types import SimpleNamespace

path, board, body, alias = sys.argv[1:]
spec = importlib.util.spec_from_file_location("append", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
real_locked = module.locked

@contextlib.contextmanager
def linking(fd, timeout, shared=False):
    with real_locked(fd, timeout, shared):
        os.link(board, alias)  # a second name appeared while the lock was awaited
        try:
            yield
        finally:
            os.remove(alias)

module.locked = linking
results = {}
def attempt(name, call):
    progress = {"may_have_appended": False}
    try:
        call(progress)
        results[name] = {"ok": True, "progress": progress}
    except Exception as error:
        results[name] = {"ok": False, "error": str(error), "progress": progress}

attempt("read", lambda progress: module.read_snapshot(board, 1.0))
attempt("append", lambda progress: module.run(
    SimpleNamespace(command="append", board=board, body_file=body, lock_timeout=1.0,
                    max_bytes=None), progress))
attempt("event", lambda progress: module.append_generated(
    board, 1.0, lambda data, offset, ts: b"never written\n", progress))
print(json.dumps(results))
"#,
    );
    let results = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(&probe)
            .arg(script("olp-board-append.py"))
            .arg(&sb.board)
            .arg(&body)
            .arg(&alias)
            .output()
            .unwrap(),
    );
    for name in ["read", "append", "event"] {
        let result = &results[name];
        assert_eq!(result["ok"], json!(false), "{name} {result}");
        assert!(
            result["error"].as_str().unwrap().contains("hard link"),
            "{name} {result}"
        );
        assert_eq!(
            result["progress"]["may_have_appended"],
            json!(false),
            "{name}"
        );
    }
    assert_eq!(fs::read(&sb.board).unwrap(), before);

    let real_flock = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v flock"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let shim_dir = sb.root.join("flock-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let shim = shim_dir.join("flock");
    fs::write(
        &shim,
        b"#!/bin/sh\nln \"$OLP_TEST_BOARD\" \"$OLP_TEST_ALIAS\"\nexec \"$OLP_TEST_REAL_FLOCK\" \"$@\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&shim).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&shim, permissions).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    let mut child = Command::new("bash")
        .arg(script("olp-board-append.sh"))
        .arg(&sb.board)
        .env("PATH", format!("{}:{path}", shim_dir.display()))
        .env("OLP_TEST_BOARD", &sb.board)
        .env("OLP_TEST_ALIAS", &alias)
        .env("OLP_TEST_REAL_FLOCK", &real_flock)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"note\n").unwrap();
    let refused = child.wait_with_output().unwrap();
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stderr={}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(String::from_utf8_lossy(&refused.stderr).contains("hard link"));
    assert_eq!(fs::read(&sb.board).unwrap(), before);
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state CLI and olp-evo-harvest.sh --dry-run, twice.
/// - Production path: the candidate merge suppresses legacy triggers inside any event's source.
/// - External edges faked: temporary boards only; the harvest script and event tool are not replaced.
/// - What this proves: text a formal item quotes (an ACK, a NUL-split ACK, a signed R2 line) is the item's own prose for harvest as it is for replay, while the same hand-written line outside any source still cards and blocks dispatch.
/// - What this intentionally does not exercise: fenced examples, covered by the out-of-order harvest test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_harvest_ignores_triggers_inside_event_sources
#[test]
fn olp_board_harvest_ignores_triggers_inside_event_sources() {
    let sb = Sandbox::new("harvest-event-sources");
    let body = sb.file(
        "quoting-item.txt",
        "Report blockers like this:\nACK(blocked): example only\nA\0CK(blocked): example only\n> 外环(outer)·R2 记档(#1): example only\n"
            .as_bytes(),
    );
    success_json(sb.event(&[
        "item",
        "--actor",
        "outer",
        "--number",
        "8",
        "--title",
        "Quoting",
        "--body-file",
        body.to_str().unwrap(),
    ]));
    let state = sb.state();
    assert_eq!(drift_kinds(&state), Vec::<String>::new());
    assert_eq!(state["dispatch_blocked"], json!(false), "{state}");
    let quiet = evo_cards(sb.harvest(&script("olp-board-event.py"), None));
    assert!(quiet.is_empty(), "{quiet:?}");

    assert!(
        sb.shell_append(b"ACK(blocked): hand-written\n")
            .status
            .success()
    );
    assert_eq!(drift_kinds(&sb.state()), vec!["unpaired_ack".to_owned()]);
    let carded = evo_cards(sb.harvest(&script("olp-board-event.py"), None));
    assert_eq!(carded.len(), 1, "{carded:?}");
    assert!(
        carded[0].contains(&format!(
            "#8#blocked#{}",
            sha256_hex(b"ACK(blocked): hand-written")
        )),
        "{carded:?}"
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: scripts/olp-board-append.sh, called with the relative name `-x`.
/// - Production path: the opted-in and hard-link checks, which hand the board path to find and grep.
/// - External edges faked: temporary boards named `-x` in two directories only.
/// - What this proves: a board name that looks like an option is still a path, so neither the hard-link refusal nor the event-line refusal can be skipped by naming the board `-x`.
/// - What this intentionally does not exercise: absolute paths, covered by the other shell tests.
/// - Focused command: cargo test --test olp_board_protocol olp_board_legacy_shell_treats_option_like_names_as_paths
#[test]
fn olp_board_legacy_shell_treats_option_like_names_as_paths() {
    let sb = Sandbox::new("option-like-name");
    let one = sb.root.join("one");
    let two = sb.root.join("two");
    fs::create_dir_all(&one).unwrap();
    fs::create_dir_all(&two).unwrap();
    fs::write(one.join("-x"), b"# Board\n\n<!-- olp-board/v1 -->\n").unwrap();
    fs::write(one.join("-x.lock"), b"").unwrap();
    fs::hard_link(one.join("-x"), two.join("-x")).unwrap();
    fs::write(two.join("-x.lock"), b"").unwrap();
    let shell = |dir: &PathBuf, body: &[u8]| {
        let mut child = Command::new("bash")
            .arg(script("olp-board-append.sh"))
            .arg("-x")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(body).unwrap();
        child.wait_with_output().unwrap()
    };
    let refused = |output: Output, needle: &str| {
        assert_eq!(
            output.status.code(),
            Some(2),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(needle),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let before = fs::read(one.join("-x")).unwrap();
    for dir in [&one, &two] {
        refused(shell(dir, b"note through a second name\n"), "hard link");
        assert_eq!(fs::read(one.join("-x")).unwrap(), before);
    }

    fs::remove_file(two.join("-x")).unwrap();
    refused(
        shell(&one, b"> OLP-EVENT {forged}\n"),
        "opted in to olp-board/v1",
    );
    assert_eq!(fs::read(one.join("-x")).unwrap(), before);
    assert!(shell(&one, b"Plain note.\n").status.success());
    assert!(
        fs::read(one.join("-x"))
            .unwrap()
            .ends_with(b"Plain note.\n")
    );
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: event state and item CLIs.
/// - Production path: the item heading pattern, recovery matching and the item writer's number check.
/// - External edges faked: temporary boards; a raw append stands in for a hand-written heading.
/// - What this proves: a heading whose title contains ". " recovers with its own number and title, and no writer can produce a number that would make its heading ambiguous.
/// - What this intentionally does not exercise: ACK recovery, covered by the recovery tests.
/// - Focused command: cargo test --test olp_board_protocol olp_board_item_numbers_stay_parseable_for_titles_with_dot_space
#[test]
fn olp_board_item_numbers_stay_parseable_for_titles_with_dot_space() {
    let sb = Sandbox::new("item-dot-space");
    sb.item("1", "Opted in", "outer", "runtime");
    let heading: &[u8] = b"### 9. Fix. the thing with dots\n";
    let offset = sb.len();
    sb.raw_append(heading);
    sb.raw_append(b"Body.\n");
    assert_eq!(drift_kinds(&sb.state()), vec!["unpaired_item".to_owned()]);
    let before = fs::read(&sb.board).unwrap();
    let body = sb.file("dot-space-body.txt", b"Recovered item.\n");
    let evidence = sb.file(
        "dot-space-evidence.json",
        &serde_json::to_vec(&source_ref(offset, heading)).unwrap(),
    );
    let item = |number: &str, title: &str| {
        sb.event(&[
            "item",
            "--actor",
            "outer",
            "--number",
            number,
            "--title",
            title,
            "--body-file",
            body.to_str().unwrap(),
            "--recovery-file",
            evidence.to_str().unwrap(),
        ])
    };

    failed(
        item("9. Fix", "the thing with dots"),
        "must not contain '. '",
    );
    assert_eq!(fs::read(&sb.board).unwrap(), before);
    success_json(item("9", "Fix. the thing with dots"));
    assert_eq!(drift_kinds(&sb.state()), Vec::<String>::new());
}

/// Test Path Statement:
/// - Tier: Real-path regression.
/// - Production entrypoint: olp-board-sentinel.py and olp-board-inbox.py with non-UTF-8 arguments.
/// - Production path: the shared printable helper in front of every machine output.
/// - External edges faked: temporary boards and non-UTF-8 command-line arguments only.
/// - What this proves: a token, actor or --since-head carrying bytes that are not UTF-8 is shown escaped; the sentinel still reports its match, the inbox still answers, and an unknown --since-head is one parseable ERROR line instead of a traceback.
/// - What this intentionally does not exercise: non-UTF-8 board text, covered by the sentinel byte-matching test.
/// - Focused command: cargo test --test olp_board_protocol olp_board_cli_output_survives_non_utf8_arguments
#[test]
fn olp_board_cli_output_survives_non_utf8_arguments() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let sb = Sandbox::new("non-utf8-args");
    sb.item("1", "First", "outer", "runtime");
    let ready = sb.root.join("ready.json");
    let watcher = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .arg("--board")
        .arg(&sb.board)
        .arg("--token")
        .arg(OsStr::from_bytes(b"WAKE-\xff"))
        .arg("--actor")
        .arg(OsStr::from_bytes(b"out\xffer"))
        .args([
            "--for",
            "outer",
            "--interval",
            "0.02",
            "--timeout",
            "10",
            "--ready-file",
        ])
        .arg(&ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_file(&ready);
    assert!(sb.shell_append(b"note WAKE-\xff here\n").status.success());
    let output = watcher.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let signal: Value = serde_json::from_str(
        stdout
            .strip_prefix("BOARD-SIGNAL: ")
            .unwrap_or_else(|| panic!("no BOARD-SIGNAL in {stdout}"))
            .trim(),
    )
    .unwrap();
    assert_eq!(signal["token"], json!("WAKE-\\udcff"));
    assert_eq!(signal["matches"], json!(["note WAKE-\u{fffd} here"]));
    let ready_state: Value = serde_json::from_slice(&fs::read(&ready).unwrap()).unwrap();
    assert_eq!(ready_state["actor"], json!("out\\udcffer"));

    let inbox_ready = sb.root.join("inbox-ready.json");
    let inbox = success_json(
        Command::new("python3")
            .arg("-B")
            .arg(script("olp-board-inbox.py"))
            .arg("--board")
            .arg(&sb.board)
            .args(["--for", "runtime", "--actor"])
            .arg(OsStr::from_bytes(b"run\xfftime"))
            .arg("--ready-file")
            .arg(&inbox_ready)
            .output()
            .unwrap(),
    );
    assert_eq!(inbox["actor"], json!("run\\udcfftime"));
    let inbox_state: Value = serde_json::from_slice(&fs::read(&inbox_ready).unwrap()).unwrap();
    assert_eq!(inbox_state["actor"], json!("run\\udcfftime"));

    let unknown_head = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-inbox.py"))
        .arg("--board")
        .arg(&sb.board)
        .args(["--for", "runtime", "--since-head"])
        .arg(OsStr::from_bytes(b"no\xffsuch"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&unknown_head.stderr);
    assert_eq!(unknown_head.status.code(), Some(2), "stderr={stderr}");
    assert!(!stderr.contains("Traceback"), "stderr={stderr}");
    let error: Value = serde_json::from_slice(&unknown_head.stderr).unwrap();
    assert!(
        error["error"].as_str().unwrap().contains("no\\udcffsuch"),
        "{error}"
    );

    let refused = Command::new("python3")
        .arg("-B")
        .arg(script("olp-board-sentinel.py"))
        .arg("--board")
        .arg(&sb.board)
        .args([
            "--token",
            "x",
            "--for",
            "outer",
            "--timeout",
            "1",
            "--since-head",
        ])
        .arg(OsStr::from_bytes(b"no\xffsuch"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&refused.stdout);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(!stderr.contains("Traceback"), "stderr={stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "stdout={stdout}");
    let error: Value = serde_json::from_str(lines[0].strip_prefix("ERROR: ").unwrap()).unwrap();
    assert!(
        error["error"].as_str().unwrap().contains("no\\udcffsuch"),
        "{error}"
    );
}
