//! Contract tests for `scripts/olp-watch-board.sh` (the OLP board sentinel).
//!
//! Pins the "唯一合法配方": a line-count baseline taken at arm time confines the
//! judgment domain to lines appended afterwards; matching inside the domain is a
//! plain `grep -F` substring match with no prefix guessing; `--skip-signature`
//! removes the outer loop's own signed annotations from the domain.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-watch-board.sh")
}

fn temp_board(name: &str, initial: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olp-watch-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let board = dir.join("OUTER_LOOP_REVIEW.md");
    std::fs::write(&board, initial).unwrap();
    board
}

fn arm(board: &PathBuf, token: &str, extra: &[&str]) -> Child {
    let mut cmd = Command::new("bash");
    cmd.arg(script())
        .arg(board)
        .arg(token)
        .args(["--interval", "1"])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().unwrap()
}

fn append(board: &PathBuf, text: &str) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(board)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

/// Wait up to `timeout` for the sentinel to exit; returns (exited, stdout).
fn wait_exit(child: &mut Child, timeout: Duration) -> (bool, String) {
    let start = Instant::now();
    loop {
        if let Some(_status) = child.try_wait().unwrap() {
            let mut out = String::new();
            if let Some(mut so) = child.stdout.take() {
                use std::io::Read;
                so.read_to_string(&mut out).unwrap();
            }
            return (true, out);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return (false, String::new());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn watch_board_fires_only_on_lines_appended_after_arming() {
    // The token already exists BEFORE arming (a task-book self-mention) and must not fire.
    let board = temp_board("baseline", "### 12. task\n请完成后落 ACK(12 done): …\n");
    let mut child = arm(&board, "ACK(12", &[]);
    std::thread::sleep(Duration::from_millis(1500));
    let (exited, _) = wait_exit(&mut child, Duration::from_millis(1600));
    assert!(!exited, "pre-baseline content must never trigger");

    let mut child = arm(&board, "ACK(12", &[]);
    std::thread::sleep(Duration::from_millis(1200));
    append(&board, "ACK(12 done): 完成,commit abc\n");
    let (exited, out) = wait_exit(&mut child, Duration::from_secs(5));
    assert!(exited, "a line appended after arming must trigger");
    assert!(out.contains("BOARD-SIGNAL: ACK(12"), "stdout: {out}");
    assert!(out.contains("commit abc"), "hit line echoed: {out}");
}

#[test]
fn watch_board_matches_any_prefix_shape() {
    // Quoted (`> `) and heading (`### `) shapes are all matched — no prefix guessing.
    let board = temp_board("prefix", "header\n");
    let mut child = arm(&board, "ACK(7", &[]);
    std::thread::sleep(Duration::from_millis(1200));
    append(&board, "> ACK(7 done)·内环: 完成\n");
    let (exited, out) = wait_exit(&mut child, Duration::from_secs(5));
    assert!(
        exited && out.contains("BOARD-SIGNAL"),
        "quoted ACK must trigger: {out}"
    );
}

#[test]
fn watch_board_skip_signature_excludes_own_annotations() {
    // The outer loop's own signed verdict quotes the token; with --skip-signature it is
    // excluded, and the sentinel keeps waiting until the inner's real ACK lands.
    let board = temp_board("skip", "header\n");
    let mut child = arm(&board, "ACK(45a", &["--skip-signature", "外环(claude)"]);
    std::thread::sleep(Duration::from_millis(1200));
    append(
        &board,
        "> 批注·外环(claude): 完成后请落 ACK(45a done|blocked)\n",
    );
    let (exited, _) = wait_exit(&mut child, Duration::from_millis(2500));
    assert!(
        !exited,
        "own signed annotation must not trigger under --skip-signature"
    );

    let mut child = arm(&board, "ACK(45a", &["--skip-signature", "外环(claude)"]);
    std::thread::sleep(Duration::from_millis(1200));
    append(&board, "ACK(45a done): commit deadbeef\n");
    let (exited, out) = wait_exit(&mut child, Duration::from_secs(5));
    assert!(
        exited && out.contains("deadbeef"),
        "inner ACK must still trigger: {out}"
    );
}

#[test]
fn watch_board_rejects_bad_arguments() {
    let out = Command::new("bash").arg(script()).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "missing args → exit 2");
    let board = temp_board("args", "x\n");
    let out = Command::new("bash")
        .arg(script())
        .arg(&board)
        .arg("tok")
        .args(["--interval", "abc"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "non-numeric interval → exit 2");
}
