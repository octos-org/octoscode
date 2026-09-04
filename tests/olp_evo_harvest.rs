//! Evolution loop phase-0 contract tests (#41, SDD spec:
//! `specs/task-req-olp-evo-p0.spec.md`).
//!
//! Every test here drives `scripts/olp-evo-harvest.sh` (and for the
//! init scenarios, `scripts/olp-init.sh`) through a fresh fixture tree
//! copied into a unique temp dir, with `OLP_EVO_STATE` pointed at a
//! per-test state root. The 17 test function names match the contract's
//! scenario filters verbatim.

/// #41-r5b ⑪: the script depends on GNU flock and stat -c; on hosts
/// without them (e.g. macOS local), say so explicitly and skip rather
/// than fail opaquely. CI (ubuntu) runs the full set.
fn ensure_gnu_tooling_or_skip() -> bool {
    let flock = Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let stat_c = Command::new("stat")
        .arg("-c")
        .arg("%s")
        .arg("/dev/null")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if flock && stat_c {
        return true;
    }
    eprintln!("SKIP (explicit): host lacks GNU flock/stat -c; harvest script requires them");
    false
}

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn fixtures(tag: &str) -> PathBuf {
    repo_root().join("fixtures/evolution").join(tag)
}

fn script() -> PathBuf {
    repo_root().join("scripts/olp-evo-harvest.sh")
}

fn init_script() -> PathBuf {
    repo_root().join("scripts/olp-init.sh")
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp sandbox with a repo dir (containing the live board) and a
/// separate state root; per the contract, state never lands in the repo.
struct Sandbox {
    root: PathBuf,
    repo: PathBuf,
    state_root: PathBuf,
    events_path: PathBuf,
    mcp_path: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "olp-evo-harvest-{tag}-{}-{seq}",
            std::process::id()
        ));
        let repo = root.join("repo");
        let state_root = root.join("state");
        std::fs::create_dir_all(repo.join(".octos")).unwrap();
        std::fs::create_dir_all(&state_root).unwrap();
        // Default optional sources point INSIDE the sandbox (nonexistent
        // until a fixture installs them) — the host's real MCP board must
        // never leak into a test.
        let events_path = root.join("events.jsonl");
        let mcp_path = root.join("mcp-board.md");
        Self {
            root,
            repo,
            state_root,
            events_path,
            mcp_path,
        }
    }

    fn copy_fixture(&self, rel: &str, dest: &Path) {
        let src = fixtures(rel);
        if src.is_dir() {
            copy_dir(&src, dest);
        } else {
            std::fs::copy(&src, dest).unwrap();
        }
    }

    fn full_trigger_board(&self) {
        self.copy_fixture(
            "review-board.md",
            &self.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        );
        std::fs::copy(fixtures("events.jsonl"), &self.events_path).unwrap();
        std::fs::copy(fixtures("mcp-board.md"), &self.mcp_path).unwrap();
    }

    fn run(&self, dry_run: bool) -> Output {
        if !ensure_gnu_tooling_or_skip() {
            panic!("host lacks required tooling; see stderr");
        }
        let mut cmd = Command::new("bash");
        cmd.arg(script()).arg(&self.repo);
        if dry_run {
            cmd.arg("--dry-run");
        }
        cmd.env("OLP_EVO_STATE", &self.state_root)
            .env("OLP_EVO_EVENTS", &self.events_path)
            .env("OLP_EVO_MCP_BOARD", &self.mcp_path);
        cmd.output().unwrap()
    }

    /// Run with explicit source env overrides (fault injection etc.).
    fn run_env(&self, extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(script())
            .arg(&self.repo)
            .env("OLP_EVO_STATE", &self.state_root)
            .env("OLP_EVO_EVENTS", &self.events_path)
            .env("OLP_EVO_MCP_BOARD", &self.mcp_path);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn evo_board(&self) -> PathBuf {
        self.repo.join(".octos/EVOLUTION.md")
    }

    fn evo_count(&self) -> usize {
        read_lines(self.evo_board())
            .iter()
            .filter(|l| l.starts_with("### EVO-"))
            .count()
    }

    fn evo_ids(&self) -> Vec<String> {
        read_lines(self.evo_board())
            .iter()
            .filter(|l| l.starts_with("### EVO-"))
            .map(|l| {
                let tok = l.split_whitespace().nth(1).unwrap_or("");
                tok.split(['（', ' ']).next().unwrap_or("").to_string()
            })
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn read_lines(path: PathBuf) -> Vec<String> {
    match std::fs::read_to_string(&path) {
        Ok(text) => text.lines().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn sha256(path: &Path) -> String {
    let out = Command::new("sha256sum").arg(path).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

/// Scenario: 全部触发器种类各落一卡
#[test]

fn olp_evo_harvest_produces_cards_for_all_trigger_kinds() {
    let sb = Sandbox::new("all-kinds");
    sb.full_trigger_board();
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(sb.evo_count(), 8);
    assert_eq!(
        sb.evo_ids(),
        (1..=8).map(|n| format!("EVO-{n:04}")).collect::<Vec<_>>()
    );
    let lines = read_lines(sb.evo_board());
    for card in lines.iter().filter(|l| l.starts_with("### EVO-")) {
        let _ = card;
    }
    // each card has the five fixed lines in order
    let text = std::fs::read_to_string(sb.evo_board()).unwrap();
    for block in text.split("### EVO-").skip(1) {
        let trigger = block.lines().find(|l| l.starts_with("trigger:"));
        let source = block.lines().find(|l| l.starts_with("source:"));
        let identity = block.lines().find(|l| l.starts_with("identity:"));
        let envelope = block.lines().find(|l| l.starts_with("envelope:"));
        let symptom = block.lines().find(|l| l.starts_with("symptom:"));
        assert!(trigger.is_some(), "card missing trigger: {block}");
        assert!(source.is_some(), "card missing source: {block}");
        assert!(identity.is_some(), "card missing identity: {block}");
        assert!(envelope.is_some(), "card missing envelope: {block}");
        assert!(symptom.is_some(), "card missing symptom: {block}");
    }
    // #41-r1 ①: each card's envelope offset must equal the BYTE offset of
    // its trigger line in the fixture source, and be strictly increasing
    // per source (cards are emitted in source order).
    let board_bytes = std::fs::read(fixtures("review-board.md")).unwrap();
    let expected_board = line_byte_offsets(&board_bytes, |l| {
        l.starts_with("ACK(blocked):") || l.starts_with("ACK(wontdo):")
    });
    let mcp_bytes = std::fs::read(fixtures("mcp-board.md")).unwrap();
    // The blocked line carries CJK detail (#41-r3): char-counting offsets
    // drift on it, byte-true `wc -c` does not — this equality IS the CJK
    // byte-offset assertion.
    let expected_mcp = line_byte_offsets(&mcp_bytes, |l| {
        l.contains("MCP(ask_outer) blocked:") || l.contains("MCP(ask_outer) timeout:")
    });
    let events_bytes = std::fs::read(fixtures("events.jsonl")).unwrap();
    let expected_events = line_byte_offsets(&events_bytes, |l| !l.trim().is_empty());
    let mut got_review: Vec<usize> = Vec::new();
    let mut got_mcp: Vec<usize> = Vec::new();
    let mut got_events: Vec<usize> = Vec::new();
    for block in text.split("### EVO-").skip(1) {
        let envelope = block
            .lines()
            .find(|l| l.starts_with("envelope:"))
            .expect("envelope line");
        let offset: usize = envelope
            .split("offset=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .expect("offset value");
        if block.contains("source: review ") {
            got_review.push(offset);
        } else if block.contains("source: mcp ") {
            got_mcp.push(offset);
        } else {
            got_events.push(offset);
        }
        // ②: envelope ts must EQUAL the card title timestamp (collection
        // time, UTC RFC3339) — not the source line's own time.
        let title_ts = block.lines().next().unwrap_or("");
        let title_ts = title_ts
            .split('（')
            .nth(1)
            .and_then(|rest| rest.split('，').next())
            .unwrap_or("");
        let env_ts = envelope.split("ts=").nth(1).unwrap_or("").trim();
        assert_eq!(
            env_ts, title_ts,
            "envelope ts must equal the title (collection) ts: {block}"
        );
    }
    assert_eq!(
        got_review, expected_board,
        "review card offsets = fixture line byte offsets (increasing)"
    );
    assert_eq!(
        got_mcp, expected_mcp,
        "mcp card offsets = fixture line byte offsets (increasing)"
    );
    assert_eq!(
        got_events, expected_events,
        "events card offsets = fixture line byte offsets (increasing)"
    );
}

/// Byte offset of the FIRST byte of each line matching `pred`, in file order.
fn line_byte_offsets(data: &[u8], pred: impl Fn(&str) -> bool) -> Vec<usize> {
    let mut out = Vec::new();
    let mut off = 0usize;
    for line in data.split(|b| *b == b'\n') {
        let len = line.len();
        let text = String::from_utf8_lossy(line);
        if pred(&text) {
            out.push(off);
        }
        off += len + 1;
    }
    out
}

/// Scenario: 负例矩阵零卡
#[test]

fn olp_evo_harvest_negative_matrix_yields_zero_cards() {
    let sb = Sandbox::new("negative");
    sb.copy_fixture(
        "negative/review-board.md",
        &sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
    );
    std::fs::copy(fixtures("negative/events.jsonl"), &sb.events_path).unwrap();
    std::fs::copy(fixtures("negative/mcp-board.md"), &sb.mcp_path).unwrap();
    let out = Command::new("bash")
        .arg(script())
        .arg(&sb.repo)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_EVENTS", &sb.events_path)
        .env("OLP_EVO_MCP_BOARD", &sb.mcp_path)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(sb.evo_count(), 0);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("malformed:"),
        "stderr must note malformed line: {stderr}"
    );
}

/// Scenario: identity 区分条目并在重跑时去重
#[test]

fn olp_evo_harvest_identity_distinguishes_entries_and_dedups_reruns() {
    let sb = Sandbox::new("dedup");
    // ### 12 and ### 13 each carry an identical ACK(blocked) line
    let board = sb.repo.join(".octos/OUTER_LOOP_REVIEW.md");
    std::fs::write(
        &board,
        "### 12. A\n\nACK(blocked): identical text here\n\n### 13. B\n\nACK(blocked): identical text here\n",
    )
    .unwrap();
    let out1 = sb.run(false);
    assert!(out1.status.success());
    assert_eq!(sb.evo_count(), 2);
    let identities: Vec<String> = read_lines(sb.evo_board())
        .into_iter()
        .filter(|l| l.starts_with("identity:"))
        .collect();
    assert_ne!(identities[0], identities[1]);
    let board_hash = sha256(&sb.evo_board());
    let state_files: Vec<String> = find_files(&sb.state_root)
        .iter()
        .map(|p| sha256(p))
        .collect();
    let out2 = sb.run(false);
    assert!(out2.status.success());
    assert_eq!(sb.evo_count(), 2);
    assert_eq!(sha256(&sb.evo_board()), board_hash);
    let state_files2: Vec<String> = find_files(&sb.state_root)
        .iter()
        .map(|p| sha256(p))
        .collect();
    assert_eq!(state_files, state_files2);
}

/// Scenario: 采集从不触碰活板
#[test]

fn olp_evo_harvest_never_writes_review_board_or_ack() {
    let sb = Sandbox::new("no-touch");
    sb.full_trigger_board();
    let board = sb.repo.join(".octos/OUTER_LOOP_REVIEW.md");
    let before = sha256(&board);
    let out = sb.run(false);
    assert!(out.status.success());
    assert_eq!(sha256(&board), before);
    let ack_lines = read_lines(sb.evo_board())
        .into_iter()
        .filter(|l| l.starts_with("ACK("))
        .count();
    assert_eq!(ack_lines, 0);
}

/// Scenario: docs 冻结快照被忽略
#[test]

fn olp_evo_harvest_ignores_docs_snapshot() {
    let sb = Sandbox::new("docs-snapshot");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing triggering here\n",
    )
    .unwrap();
    std::fs::create_dir_all(sb.repo.join("docs")).unwrap();
    std::fs::write(
        sb.repo.join("docs/OUTER_LOOP_REVIEW.md"),
        "### 2. snapshot\n\nACK(blocked): frozen snapshot line\n",
    )
    .unwrap();
    let out = sb.run(false);
    assert!(out.status.success());
    assert_eq!(sb.evo_count(), 0);
}

/// Scenario: MCP 卡不复制问询正文
#[test]

fn olp_evo_harvest_mcp_symptom_excludes_question_text() {
    let sb = Sandbox::new("mcp-privacy");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let mcp = sb.mcp_path.clone();
    std::fs::write(
        &mcp,
        "- 2026-08-30 05:00:00 MCP(ask_outer) blocked: id=abc123 reason=inner stuck needs=op question=SECRET-QUESTION\n",
    )
    .unwrap();
    let out = Command::new("bash")
        .arg(script())
        .arg(&sb.repo)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_EVENTS", "")
        .env("OLP_EVO_MCP_BOARD", &mcp)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(sb.evo_board()).unwrap_or_default();
    assert!(!text.contains("SECRET-QUESTION"));
    let symptom = text
        .lines()
        .find(|l| l.starts_with("symptom:"))
        .expect("one card");
    assert!(
        symptom.starts_with("symptom: kind=blocked"),
        "got: {symptom}"
    );
}

/// Scenario: 半行不触发,补齐换行后恰一卡
#[test]

fn olp_evo_harvest_partial_line_then_completed_yields_one_card() {
    let sb = Sandbox::new("partial");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let events = sb.events_path.clone();
    let complete =
        "{\"ts\":\"2026-08-30T02:00:00Z\",\"kind\":\"turn_error\",\"data\":{\"detail\":\"x\"}}\n";
    let partial: &str =
        "{\"ts\":\"2026-08-30T03:00:00Z\",\"kind\":\"turn_error\",\"data\":{\"detail\":\"un";
    std::fs::write(&events, complete).unwrap();
    let complete_only_len = std::fs::metadata(&events).unwrap().len() as usize;
    std::fs::write(&events, format!("{complete}{partial}")).unwrap();
    let out = Command::new("bash")
        .arg(script())
        .arg(&sb.repo)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_EVENTS", &events)
        .env("OLP_EVO_MCP_BOARD", "/nonexistent")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(sb.evo_count(), 1, "only the complete line fires");
    let state_text = std::fs::read_to_string(
        find_files(&sb.state_root)
            .into_iter()
            .find(|p| p.ends_with("state.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(
        state_text.contains(&format!("\"offset\": {}", complete_only_len)),
        "events offset must be the pre-partial byte count: {state_text}"
    );
    // complete the partial line and rerun
    std::fs::write(&events, format!("{complete}{{\"ts\":\"2026-08-30T03:00:00Z\",\"kind\":\"turn_error\",\"data\":{{\"detail\":\"y\"}}}}\n")).unwrap();
    let out2 = Command::new("bash")
        .arg(script())
        .arg(&sb.repo)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_EVENTS", &events)
        .env("OLP_EVO_MCP_BOARD", "/nonexistent")
        .output()
        .unwrap();
    assert!(out2.status.success());
    assert_eq!(sb.evo_count(), 2);
}

/// #41-r5a: an UNCHANGED rerun must print NO reset: (the cursor logic
/// reads state correctly; reset fires only on real change).
#[test]
fn olp_evo_harvest_unchanged_rerun_prints_no_reset() {
    let sb = Sandbox::new("no-reset");
    sb.full_trigger_board();
    let out1 = sb.run(false);
    assert!(
        out1.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    let out2 = sb.run(false);
    assert!(out2.status.success());
    let stderr = String::from_utf8_lossy(&out2.stderr);
    assert!(
        !stderr.contains("reset:"),
        "unchanged rerun must not log reset: {stderr}"
    );
    assert_eq!(sb.evo_count(), 8);
}

/// #41-r5a ③: events WITHOUT a ts field must still dedup across reruns
/// (identity uses '-', not a per-run now()).
#[test]
fn olp_evo_harvest_missing_ts_events_dedup_across_reruns() {
    let sb = Sandbox::new("no-ts");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let no_ts = b"{\"kind\":\"escalation\",\"data\":{\"goal_id\":\"g1\",\"detail\":\"d\"}}\n";
    std::fs::write(&sb.events_path, no_ts).unwrap();
    let out1 = sb.run(false);
    assert!(
        out1.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    assert_eq!(sb.evo_count(), 1, "first run harvests the ts-less event");
    let out2 = sb.run(false);
    assert!(out2.status.success());
    assert_eq!(sb.evo_count(), 1, "ts-less event must dedup on rerun");
}

/// #41-r5a ④: a non-numeric EVO heading (### EVO-draft) on the board must
/// not abort reconcile — harvesting still lands cards.
#[test]
fn olp_evo_harvest_survives_non_numeric_evo_id_on_board() {
    let sb = Sandbox::new("draft-id");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 12. A\n\nACK(blocked): x\n",
    )
    .unwrap();
    let out1 = sb.run(false);
    assert!(out1.status.success());
    assert_eq!(sb.evo_count(), 1);
    // Plant a draft heading on the evolution board, then rerun with a new
    // trigger to prove reconcile survived it.
    std::fs::write(
        sb.repo.join(".octos/EVOLUTION.md"),
        "### EVO-draft（manual，human）\ntrigger: manual\n",
    )
    .unwrap();
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 12. A\n\nACK(blocked): x\n\n### 13. B\n\nACK(wontdo): y\n",
    )
    .unwrap();
    let out2 = sb.run(false);
    assert!(
        out2.status.success(),
        "reconcile must survive non-numeric ids: stderr={}",
        String::from_utf8_lossy(&out2.stderr)
    );
    assert_eq!(sb.evo_count(), 2, "the new trigger still lands a card");
}

/// #41-r5a ⑤: a missing repo-root exits 2 with zero side effects.
#[test]
fn olp_evo_harvest_missing_repo_root_exits_two() {
    let root = std::env::temp_dir().join(format!("olp-evo-norepo-{}", std::process::id()));
    let missing = root.join("no-such-repo");
    let state_root = root.join("state");
    std::fs::create_dir_all(&state_root).unwrap();
    let out = Command::new("bash")
        .arg(script())
        .arg(&missing)
        .env("OLP_EVO_STATE", &state_root)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(find_files(&state_root).is_empty(), "zero side effects");
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: 截断或替换后重置且不重复
#[test]

fn olp_evo_harvest_resets_on_truncate_or_replace_without_duplicates() {
    let sb = Sandbox::new("reset");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let events = sb.events_path.clone();
    let esc = |n: u32| {
        format!(
            "{{\"ts\":\"2026-08-30T0{n}:00:00Z\",\"kind\":\"escalation\",\"data\":{{\"goal_id\":\"g{n}\",\"detail\":\"d\"}}}}\n"
        )
    };
    std::fs::write(&events, format!("{}{}", esc(1), esc(2))).unwrap();
    let run = || {
        Command::new("bash")
            .arg(script())
            .arg(&sb.repo)
            .env("OLP_EVO_STATE", &sb.state_root)
            .env("OLP_EVO_EVENTS", &events)
            .env("OLP_EVO_MCP_BOARD", "/nonexistent")
            .output()
            .unwrap()
    };
    let out = run();
    assert!(out.status.success());
    assert_eq!(sb.evo_count(), 2);
    // truncate to just the last line
    std::fs::write(&events, esc(2)).unwrap();
    let out2 = run();
    assert!(out2.status.success());
    let stderr = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr.contains("reset:"),
        "truncate must log reset: {stderr}"
    );
    assert_eq!(sb.evo_count(), 2, "no duplicate after truncate");
    // replace with a larger file with two NEW escalations
    std::fs::write(&events, format!("{}{}", esc(3), esc(4))).unwrap();
    let out3 = run();
    assert!(out3.status.success());
    let stderr3 = String::from_utf8_lossy(&out3.stderr);
    assert!(
        stderr3.contains("reset:"),
        "replace must log reset: {stderr3}"
    );
    assert_eq!(
        sb.evo_count(),
        4,
        "exactly +2 new cards after replace (contract: two new escalations)"
    );
}

/// Scenario: 追加卡后提交状态前崩溃可恢复
#[test]

fn olp_evo_harvest_recovers_after_crash_between_append_and_commit() {
    let sb = Sandbox::new("crash");
    sb.full_trigger_board();
    let out = sb.run_env(&[("OLP_EVO_TEST", "1"), ("OLP_EVO_FAULT", "after-append")]);
    assert_eq!(out.status.code(), Some(70));
    let out2 = sb.run(false);
    assert!(
        out2.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    assert_eq!(sb.evo_count(), 8);
    // state offsets must equal the source file sizes
    let state_file = find_files(&sb.state_root)
        .into_iter()
        .find(|p| p.ends_with("state.json"))
        .unwrap();
    let state_text = std::fs::read_to_string(&state_file).unwrap();
    let events_size = std::fs::metadata(&sb.events_path).unwrap().len();
    let mcp_size = std::fs::metadata(&sb.mcp_path).unwrap().len();
    assert!(
        state_text.contains(&format!("\"offset\": {}", events_size)),
        "{state_text}"
    );
    assert!(
        state_text.contains(&format!("\"offset\": {}", mcp_size)),
        "{state_text}"
    );
}

/// Scenario: 并发采集编号唯一
#[test]

fn olp_evo_harvest_concurrent_runs_allocate_unique_ids() {
    let sb = Sandbox::new("concurrent");
    sb.full_trigger_board();
    let mut handles = Vec::new();
    for _ in 0..2 {
        let script = script();
        let repo = sb.repo.clone();
        let state = sb.state_root.clone();
        let events = sb.events_path.clone();
        let mcp = sb.mcp_path.clone();
        handles.push(std::thread::spawn(move || {
            Command::new("bash")
                .arg(script)
                .arg(repo)
                .env("OLP_EVO_STATE", state)
                .env("OLP_EVO_EVENTS", events)
                .env("OLP_EVO_MCP_BOARD", mcp)
                .output()
                .unwrap()
        }));
    }
    for h in handles {
        let out = h.join().unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert_eq!(sb.evo_count(), 8);
    let ids = sb.evo_ids();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), unique.len(), "ids must be unique: {ids:?}");
}

/// Scenario: 活板缺失即失败且零创建
#[test]

fn olp_evo_harvest_fails_without_review_board_before_creating_state() {
    let sb = Sandbox::new("no-board");
    // no .octos/OUTER_LOOP_REVIEW.md created; state root exists and is writable
    let out = sb.run(false);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        find_files(&sb.state_root).is_empty(),
        "state root must stay empty"
    );
    assert!(!sb.evo_board().exists());
}

/// Scenario: dry-run 对已有状态零写入
#[test]

fn olp_evo_harvest_dry_run_is_read_only_with_existing_state() {
    let sb = Sandbox::new("dry-run");
    // Given: one full harvest landed (8 cards), then a NEW ack line.
    sb.full_trigger_board();
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(sb.evo_count(), 8);
    let board = sb.repo.join(".octos/OUTER_LOOP_REVIEW.md");
    let mut text = std::fs::read_to_string(&board).unwrap();
    text.push_str("\n### 99. new\n\nACK(blocked): second run line\n");
    std::fs::write(&board, text).unwrap();
    let before_board = sha256(&sb.evo_board());
    let before_state: Vec<String> = find_files(&sb.state_root)
        .iter()
        .map(|p| sha256(p))
        .collect();
    let out2 = sb.run(true);
    assert!(out2.status.success());
    let stdout = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout.contains("### EVO-0009"),
        "dry-run prints the next card id: {stdout}"
    );
    assert_eq!(sha256(&sb.evo_board()), before_board);
    let after_state: Vec<String> = find_files(&sb.state_root)
        .iter()
        .map(|p| sha256(p))
        .collect();
    assert_eq!(before_state, after_state);
}

/// Scenario: 可选来源缺失时跳过
#[test]

fn olp_evo_harvest_skips_missing_optional_sources() {
    let sb = Sandbox::new("skip");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let out = Command::new("bash")
        .arg(script())
        .arg(&sb.repo)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_EVENTS", "")
        .env("OLP_EVO_MCP_BOARD", "/nonexistent/path")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skip:"),
        "stderr must note the skip: {stderr}"
    );
}

/// Scenario: 状态按项目隔离
#[test]

fn olp_evo_harvest_state_is_per_project() {
    let board_text = "### 1. base\n\nACK(blocked): one\n";
    let root = std::env::temp_dir().join(format!("olp-evo-two-projects-{}", std::process::id()));
    let state_root = root.join("state");
    std::fs::create_dir_all(&state_root).unwrap();
    let run = |repo: &Path| {
        Command::new("bash")
            .arg(script())
            .arg(repo)
            .env("OLP_EVO_STATE", &state_root)
            .env("OLP_EVO_EVENTS", "")
            .env("OLP_EVO_MCP_BOARD", "/nonexistent")
            .output()
            .unwrap()
    };
    let repos: Vec<PathBuf> = ["proj-a", "proj-b"]
        .iter()
        .map(|name| {
            let repo = root.join(name);
            std::fs::create_dir_all(repo.join(".octos")).unwrap();
            std::fs::write(repo.join(".octos/OUTER_LOOP_REVIEW.md"), board_text).unwrap();
            repo
        })
        .collect();
    for repo in &repos {
        let out = run(repo);
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    for repo in &repos {
        let evo = repo.join(".octos/EVOLUTION.md");
        let count = read_lines(evo)
            .iter()
            .filter(|l| l.starts_with("### EVO-"))
            .count();
        assert_eq!(count, 1);
    }
    let project_dirs: Vec<_> = std::fs::read_dir(&state_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    assert_eq!(project_dirs.len(), 2, "two distinct project subdirs");
    let _ = std::fs::remove_dir_all(&root);
}

/// #41-r3: the script must work from any cwd (board-append resolved via
/// $(dirname "$0")); run it from a temp dir with an absolute repo-root.
#[test]
fn olp_evo_harvest_runs_from_outside_repo_cwd() {
    let sb = Sandbox::new("outside-cwd");
    sb.full_trigger_board();
    let outside = sb.root.join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let out = Command::new("bash")
        .arg(script())
        .arg(sb.repo.canonicalize().unwrap())
        .current_dir(&outside)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_EVENTS", &sb.events_path)
        .env("OLP_EVO_MCP_BOARD", &sb.mcp_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "exit={} stdout={} stderr={}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        sb.evo_count(),
        8,
        "cards must land when run from an outside cwd"
    );
}

/// #41-r5b ⑥: JSON-valid but non-object events lines (null / arrays)
/// count as malformed and processing CONTINUES past them.
#[test]
fn olp_evo_harvest_non_object_json_is_malformed_and_continues() {
    let sb = Sandbox::new("non-object");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let esc = |n: u32| {
        format!(
            "{{\"ts\":\"2026-08-30T0{n}:00:00Z\",\"kind\":\"escalation\",\"data\":{{\"goal_id\":\"g{n}\"}}}}\n"
        )
    };
    std::fs::write(
        &sb.events_path,
        format!("null\n[1,2]\n{}\"42\"\n{}", esc(1), esc(2)),
    )
    .unwrap();
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.matches("malformed:").count() >= 3,
        "null/array/number lines are malformed: {stderr}"
    );
    assert_eq!(sb.evo_count(), 2, "events AFTER the non-objects still fire");
}

/// #41-r5b ⑦: a board line without a trailing newline (half line) must
/// not fire; completing it fires exactly once.
#[test]
fn olp_evo_harvest_board_partial_line_no_fire_until_newline() {
    let sb = Sandbox::new("board-partial");
    let board = sb.repo.join(".octos/OUTER_LOOP_REVIEW.md");
    std::fs::write(&board, "### 12. A\n\nACK(blocked): complete\n").unwrap();
    let out1 = sb.run(false);
    assert!(out1.status.success());
    assert_eq!(sb.evo_count(), 1);
    // append a NEW trigger without trailing newline — must not fire yet
    std::fs::write(
        &board,
        "### 12. A\n\nACK(blocked): complete\n\n### 13. B\n\nACK(wontdo): half",
    )
    .unwrap();
    let out2 = sb.run(false);
    assert!(out2.status.success());
    assert_eq!(sb.evo_count(), 1, "half line (no newline) must not fire");
    // complete it
    std::fs::write(
        &board,
        "### 12. A\n\nACK(blocked): complete\n\n### 13. B\n\nACK(wontdo): half\n",
    )
    .unwrap();
    let out3 = sb.run(false);
    assert!(out3.status.success());
    assert_eq!(sb.evo_count(), 2, "completed line fires exactly once");
}

/// #41-r5b ⑦ (MCP side): same newline rule for the MCP board.
#[test]
fn olp_evo_harvest_mcp_partial_line_no_fire_until_newline() {
    let sb = Sandbox::new("mcp-partial");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    let full = "- 2026-08-30 05:00:00 MCP(ask_outer) blocked: id=a1 reason=r1\n";
    std::fs::write(&sb.mcp_path, full).unwrap();
    let out1 = sb.run(false);
    assert!(out1.status.success());
    assert_eq!(sb.evo_count(), 1);
    std::fs::write(
        &sb.mcp_path,
        "- 2026-08-30 05:00:00 MCP(ask_outer) blocked: id=a1 reason=r1\n- 2026-08-30 06:00:00 MCP(ask_outer) timeout: id=a2 reason=r2",
    )
    .unwrap();
    let out2 = sb.run(false);
    assert!(out2.status.success());
    assert_eq!(sb.evo_count(), 1, "MCP half line must not fire");
    std::fs::write(
        &sb.mcp_path,
        "- 2026-08-30 05:00:00 MCP(ask_outer) blocked: id=a1 reason=r1\n- 2026-08-30 06:00:00 MCP(ask_outer) timeout: id=a2 reason=r2\n",
    )
    .unwrap();
    let out3 = sb.run(false);
    assert!(out3.status.success());
    assert_eq!(sb.evo_count(), 2);
}

/// #41-r5b ⑧: the MCP symptom carries reason= up to the NEXT key token
/// (not just the first word).
#[test]
fn olp_evo_harvest_mcp_reason_takes_phrase_not_first_word() {
    let sb = Sandbox::new("reason-phrase");
    std::fs::write(
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
        "### 1. base\n\nnothing\n",
    )
    .unwrap();
    std::fs::write(
        &sb.mcp_path,
        "- 2026-08-30 05:00:00 MCP(ask_outer) blocked: id=a1 reason=inner loop stuck on step three needs=op question=SECRET-QUESTION\n",
    )
    .unwrap();
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(sb.evo_board()).unwrap_or_default();
    assert!(!text.contains("SECRET-QUESTION"));
    let symptom = text
        .lines()
        .find(|l| l.starts_with("symptom:"))
        .expect("card");
    assert!(
        symptom.contains("reason=inner loop stuck on step three"),
        "reason keeps the phrase until the next key: {symptom}"
    );
    assert!(
        !symptom.contains("needs="),
        "reason stops at the next key token: {symptom}"
    );
}

/// #41-r5b ⑨: appending the EVOLUTION ignore when .gitignore lacks a
/// trailing newline must not glue onto the last line.
#[test]
fn olp_evo_init_pads_missing_trailing_newline() {
    let root = std::env::temp_dir().join(format!("olp-evo-init-pad-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    // note: NO trailing newline on the last line
    std::fs::write(repo.join(".gitignore"), ".octos/OUTER_LOOP_REVIEW.md").unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let out = Command::new("bash")
        .arg(init_script())
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        matches!(out.status.code(), Some(0) | Some(2)),
        "exit={} stderr={}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr)
    );
    let gi = std::fs::read_to_string(repo.join(".gitignore")).unwrap();
    assert!(
        gi.lines().any(|l| l.trim() == ".octos/EVOLUTION.md"),
        "EVOLUTION.md must be its OWN line, got:\n{gi}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: 记录目录与记录校验
#[test]

fn olp_evo_records_dir_frontmatter_is_valid() {
    let dir = repo_root().join("knowledge/context/evolution");
    for name in ["README.md", "FLAW-template.md", "memory.md", "operators.md"] {
        assert!(dir.join(name).exists(), "missing {name}");
    }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("FLAW-") || !name.ends_with(".md") || name == "FLAW-template.md" {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).unwrap();
        let fm = frontmatter(&text).unwrap_or_else(|| panic!("{name} has no frontmatter"));
        assert_eq!(
            fm.get("kind").map(String::as_str),
            Some("context"),
            "{name}"
        );
        assert!(
            fm.get("id")
                .is_some_and(|v| v.starts_with("FLAW-") && v.len() > "FLAW-".len()),
            "{name} id"
        );
        ids.push(fm["id"].clone());
        assert!(fm.contains_key("repo"), "{name}");
        assert!(fm.contains_key("layers"), "{name}");
        let status = fm.get("status").map(String::as_str);
        assert!(
            matches!(
                status,
                Some("open")
                    | Some("consolidated")
                    | Some("filed")
                    | Some("accepted")
                    | Some("specified")
                    | Some("patched")
                    | Some("verified")
                    | Some("closed")
                    | Some("rejected")
                    | Some("reopened")
            ),
            "{name} status: {status:?}"
        );
        let severity = fm.get("severity").map(String::as_str);
        assert!(
            matches!(severity, Some("S1") | Some("S2") | Some("S3")),
            "{name} severity: {severity:?}"
        );
        let recurrence = fm.get("recurrence").map(String::as_str).unwrap_or("");
        assert!(
            recurrence.parse::<u32>().is_ok(),
            "{name} recurrence: {recurrence}"
        );
        assert!(
            fm.get("fingerprint").is_some_and(|v| !v.is_empty()),
            "{name}"
        );
    }
    ids.sort();
    ids.dedup();
    let flaw_count = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            let n = e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .to_string();
            n.starts_with("FLAW-") && n.ends_with(".md") && n != "FLAW-template.md"
        })
        .count();
    assert_eq!(ids.len(), flaw_count, "ids must be unique");
    // issue fields
    let f1 = std::fs::read_to_string(dir.join("FLAW-001.md")).unwrap();
    let f2 = std::fs::read_to_string(dir.join("FLAW-002.md")).unwrap();
    assert!(f1.contains("issues/2236"), "FLAW-001 issue");
    assert!(f2.contains("issues/2237"), "FLAW-002 issue");
}

fn frontmatter(text: &str) -> Option<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            return Some(map);
        }
        if let Some((k, v)) = line.split_once(": ") {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    None
}

/// Scenario: olp-init 为只忽略活板的项目追加 EVOLUTION.md 忽略
#[test]

fn olp_evo_init_appends_evolution_gitignore_once() {
    let root = std::env::temp_dir().join(format!("olp-evo-init-append-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    std::fs::write(repo.join(".gitignore"), ".octos/OUTER_LOOP_REVIEW.md\n").unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let run = || {
        Command::new("bash")
            .arg(init_script())
            .current_dir(&repo)
            .output()
            .unwrap()
    };
    // Exit 0 (all deps present) or 2 (dependency check found gaps, e.g. a
    // CI runner without octoscode) — both are the script's defined
    // semantics; the contract Then pins only the .gitignore content.
    let out = run();
    let diag = |o: &std::process::Output| {
        format!(
            "exit={} stdout={} stderr={}",
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };
    assert!(
        matches!(out.status.code(), Some(0) | Some(2)),
        "run 1 must exit 0 or 2: {}",
        diag(&out)
    );
    let out2 = run();
    assert!(
        matches!(out2.status.code(), Some(0) | Some(2)),
        "run 2 must exit 0 or 2: {}",
        diag(&out2)
    );
    let gi = std::fs::read_to_string(repo.join(".gitignore")).unwrap();
    assert_eq!(
        gi.matches(".octos/EVOLUTION.md").count(),
        1,
        "gitignore content: {gi:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: olp-init 对整目录已忽略的项目不追加
#[test]

fn olp_evo_init_skips_when_octos_dir_ignored() {
    let root = std::env::temp_dir().join(format!("olp-evo-init-skip-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    std::fs::write(repo.join(".gitignore"), ".octos\n").unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let out = Command::new("bash")
        .arg(init_script())
        .current_dir(&repo)
        .output()
        .unwrap();
    // 0 (deps present) or 2 (dependency-check gap on the runner) are both
    // defined; the contract Then pins only that nothing was appended.
    assert!(
        matches!(out.status.code(), Some(0) | Some(2)),
        "must exit 0 or 2: exit={} stdout={} stderr={}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let gi = std::fs::read_to_string(repo.join(".gitignore")).unwrap();
    assert!(
        !gi.contains(".octos/EVOLUTION.md"),
        "gitignore must not gain the line: {gi:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn find_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(find_files(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}
