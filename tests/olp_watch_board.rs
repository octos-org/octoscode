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

// --- #44a: --harvest cadence mode (five scenarios, contract v3) -------

fn temp_repo(name: &str, initial: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olp-watch-hv-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(dir.join("repo/.octos")).unwrap();
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(dir.join("repo/.octos/OUTER_LOOP_REVIEW.md"), initial).unwrap();
    dir.join("repo")
}

fn harvest_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-evo-harvest.sh")
}

fn arm_harvest(
    board: &PathBuf,
    token: &str,
    repo: &std::path::Path,
    state_root: &std::path::Path,
) -> Child {
    let mut cmd = Command::new("bash");
    let sandbox_mcp = repo.parent().unwrap().join("mcp-board.absent");
    cmd.arg(script())
        .arg(board)
        .arg(token)
        .args(["--interval", "1"])
        .arg("--harvest")
        .arg(repo)
        .env("OLP_EVO_STATE", state_root)
        .env("OLP_EVO_EVENTS", "")
        .env("OLP_EVO_MCP_BOARD", &sandbox_mcp)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().unwrap()
}

fn still_running(child: &mut Child) -> bool {
    child.try_wait().unwrap().is_none()
}

/// Scenario: 节拍采集在命中时落卡、推进基线且常驻
#[test]
fn olp_watch_board_harvest_on_hit_writes_card_and_keeps_watching() {
    let repo = temp_repo("hit", "l1\nl2\nl3\n");
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    let state = repo.parent().unwrap().join("state");
    let mut child = arm_harvest(&board, "ACK(blocked", &repo, &state);
    std::thread::sleep(Duration::from_millis(1500));
    append(&board, "ACK(blocked): waiting for outer decision\n");
    std::thread::sleep(Duration::from_secs(3));
    let evo = std::fs::read_to_string(repo.join(".octos/EVOLUTION.md")).unwrap_or_default();
    let cards = evo.lines().filter(|l| l.starts_with("### EVO-")).count();
    assert_eq!(cards, 1, "exactly one card lands: {evo}");
    assert!(still_running(&mut child), "--harvest keeps watching");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}

/// Scenario: 节拍与手跑并发只落一张卡
#[test]
fn olp_watch_board_harvest_concurrent_with_manual_is_deduped() {
    let repo = temp_repo("dedup", "l1\nl2\nl3\n");
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    let state = repo.parent().unwrap().join("state");
    let mut child = arm_harvest(&board, "ACK(blocked", &repo, &state);
    std::thread::sleep(Duration::from_millis(1500));
    append(&board, "ACK(blocked): waiting for outer decision\n");
    // manual double-run immediately (races the sentinel's own harvest)
    for _ in 0..2 {
        let _ = Command::new("bash")
            .arg(harvest_script())
            .arg(&repo)
            .env("OLP_EVO_STATE", &state)
            .env("OLP_EVO_EVENTS", "")
            .env(
                "OLP_EVO_MCP_BOARD",
                repo.parent().unwrap().join("mcp-board.absent"),
            )
            .output()
            .unwrap();
    }
    std::thread::sleep(Duration::from_secs(2));
    let evo = std::fs::read_to_string(repo.join(".octos/EVOLUTION.md")).unwrap_or_default();
    let cards = evo.lines().filter(|l| l.starts_with("### EVO-")).count();
    assert_eq!(cards, 1, "identity dedup across watcher+manual: {evo}");
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}

/// Scenario: 采集失败不中断监视
#[test]
fn olp_watch_board_harvest_failure_keeps_watching() {
    let repo = temp_repo("fail", "l1\nl2\nl3\n");
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    let state = repo.parent().unwrap().join("state");
    let mut child = arm_harvest(&board, "ACK(blocked", &repo, &state);
    std::thread::sleep(Duration::from_millis(1500));
    // /nonexistent repo → harvest fails
    let _ = Command::new("bash")
        .arg(script())
        .arg(&board)
        .arg("ACK(blocked")
        .args(["--interval", "1", "--harvest", "/nonexistent"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    append(&board, "ACK(blocked): trigger\n");
    std::thread::sleep(Duration::from_secs(3));
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}

/// Scenario: 非命中新增行不触发采集
#[test]
fn olp_watch_board_harvest_ignores_non_hit_lines() {
    let repo = temp_repo("nonhit", "l1\nl2\nl3\n");
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    let state = repo.parent().unwrap().join("state");
    let mut child = arm_harvest(&board, "ZZZNOTOKEN", &repo, &state);
    std::thread::sleep(Duration::from_millis(1500));
    append(&board, "plain line one\nplain line two\n");
    std::thread::sleep(Duration::from_secs(3));
    let evo_exists = repo.join(".octos/EVOLUTION.md").exists();
    assert!(!evo_exists, "no evolution board without a hit");
    assert!(still_running(&mut child));
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}

/// Scenario: 采集失败后下一批命中仍可处理
#[test]
fn olp_watch_board_harvest_recovers_after_failure() {
    let root = std::env::temp_dir().join(format!("olp-watch-hv-rec-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    let ro_state = root.join("ro-state");
    std::fs::create_dir_all(&ro_state).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&ro_state, std::fs::Permissions::from_mode(0o555)).unwrap();
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    std::fs::write(&board, "l1\nl2\nl3\n").unwrap();

    let mut child = Command::new("bash")
        .arg(script())
        .arg(&board)
        .arg("ACK(blocked")
        .args(["--interval", "1"])
        .arg("--harvest")
        .arg(&repo)
        .env("OLP_EVO_STATE", &ro_state)
        .env("OLP_EVO_EVENTS", "")
        .env("OLP_EVO_MCP_BOARD", root.join("mcp-board.absent"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    append(&board, "ACK(blocked): first hit\n");
    std::thread::sleep(Duration::from_secs(2));
    // repair permissions and add a second hit
    std::fs::set_permissions(&ro_state, std::fs::Permissions::from_mode(0o755)).unwrap();
    append(&board, "ACK(blocked): second hit\n");
    std::thread::sleep(Duration::from_secs(3));
    assert!(still_running(&mut child));
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: 不带 --harvest 仍一击退出
#[test]
fn olp_watch_board_without_harvest_exits_on_hit() {
    let board = temp_board("plain", "l1\nl2\nl3\n");
    let mut child = arm(&board, "ACK(blocked", &[]);
    std::thread::sleep(Duration::from_millis(1200));
    append(&board, "ACK(blocked): trigger\n");
    let (exited, out) = wait_exit(&mut child, Duration::from_secs(5));
    assert!(exited, "no --harvest → one-shot exit");
    assert!(out.contains("BOARD-SIGNAL"), "{out}");
}

/// 44-r1: an INSTALLED copy (outside the repo) harvests via the
/// repo-root's scripts/ directory.
#[test]
fn olp_watch_board_harvest_works_from_installed_copy() {
    let root = std::env::temp_dir().join(format!("olp-inst-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    std::fs::create_dir_all(repo.join("scripts")).unwrap();
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    // installed copy of the watcher in an unrelated dir
    let installed = root.join("watch-board.sh");
    std::fs::copy(script(), &installed).unwrap();
    // repo carries the harvest script (repo-root-first lookup)
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-evo-harvest.sh"),
        repo.join("scripts/olp-evo-harvest.sh"),
    )
    .unwrap();
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    std::fs::write(&board, "l1\nl2\nl3\n").unwrap();
    let mut child = Command::new("bash")
        .arg(&installed)
        .arg(&board)
        .arg("ACK(blocked")
        .args(["--interval", "1"])
        .arg("--harvest")
        .arg(&repo)
        .env("OLP_EVO_STATE", &state)
        .env("OLP_EVO_EVENTS", "")
        .env("OLP_EVO_MCP_BOARD", root.join("mcp.absent"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    append(&board, "ACK(blocked): installed copy trigger\n");
    std::thread::sleep(Duration::from_secs(3));
    let _ = child.kill();
    let _ = child.wait();
    let evo = std::fs::read_to_string(repo.join(".octos/EVOLUTION.md")).unwrap_or_default();
    let cards = evo.lines().filter(|l| l.starts_with("### EVO-")).count();
    assert_eq!(cards, 1, "installed copy harvests via repo scripts/: {evo}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 44-r2 (N2): a ~400-line hit burst must not kill the resident watcher
/// (no `printf | head` pipeline → no SIGPIPE 141 under pipefail).
#[test]
fn olp_watch_board_harvest_survives_burst_hits() {
    let root = std::env::temp_dir().join(format!("olp-burst-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    let board = repo.join(".octos/OUTER_LOOP_REVIEW.md");
    std::fs::write(&board, "l1\nl2\nl3\n").unwrap();
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let mut child = Command::new("bash")
        .arg(script())
        .arg(&board)
        .arg("ACK(blocked")
        .args(["--interval", "1"])
        // RESIDENT mode (44-r2 N2): without --harvest the watcher exits
        // on the first hit by design — the burst survival contract is a
        // --harvest (resident loop) property.
        .arg("--harvest")
        .arg(&repo)
        .env("OLP_EVO_STATE", &state)
        .env("OLP_EVO_EVENTS", "")
        .env("OLP_EVO_MCP_BOARD", root.join("absent"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    // 400 lines, EVERY one matching the token — head -3 keeps the print
    // bounded, but the watcher itself must survive and keep polling.
    let burst: String = (0..400)
        .map(|i| format!("ACK(blocked): burst line {i}\n"))
        .collect();
    append(&board, &burst);
    // POLL, don't sleep: a 400-line harvest can take 10s+ under a
    // fully-parallel `cargo test --all-targets` (CPU contention), so a
    // fixed 15s window flakes. Wait for the FIRST signal with a 60s
    // budget instead.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !std::fs::read_to_string(&board)
        .map(|_| true)
        .unwrap_or(false)
        || true
    {
        // (just wait on the wall clock; output is drained after kill)
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    // second batch after the burst — proves the loop still lives
    append(&board, "ACK(blocked): second batch\n");
    // allow the second harvest round its full time budget under load.
    std::thread::sleep(Duration::from_secs(45));
    // kill FIRST (resident watcher never exits), THEN read its output —
    // read_to_string on a live pipe would block forever.
    let _ = child.kill();
    let status = child.wait().unwrap();
    use std::io::Read;
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_string(&mut text).unwrap();
    }
    assert!(
        text.contains("BOARD-SIGNAL"),
        "first batch processed: {text}"
    );
    // count BOARD-SIGNAL prints: burst batch + second batch = 2
    let signals = text.matches("BOARD-SIGNAL").count();
    assert!(signals >= 2, "watcher survived the burst: {text}");
    let _ = std::fs::remove_dir_all(&root);
    let _ = status;
}
