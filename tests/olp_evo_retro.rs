//! Evolution loop phase-1 retro contract tests (#42, SDD spec:
//! `specs/task-req-olp-evo-p1.spec.md`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn script() -> PathBuf {
    repo_root().join("scripts/olp-evo-retro.sh")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("fixtures/evolution/retro").join(name)
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    #[allow(dead_code)]
    root: PathBuf,
    repo: PathBuf,
    state_root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("olp-evo-retro-{tag}-{}-{seq}", std::process::id()));
        let repo = root.join("repo");
        let state_root = root.join("state");
        std::fs::create_dir_all(repo.join(".octos")).unwrap();
        std::fs::create_dir_all(&state_root).unwrap();
        Self {
            root,
            repo,
            state_root,
        }
    }

    fn evo_board(&self) -> PathBuf {
        self.repo.join(".octos/EVOLUTION.md")
    }

    fn install_board(&self, fixture_name: &str) {
        std::fs::copy(fixture(fixture_name), self.evo_board()).unwrap();
    }

    fn write_board(&self, text: &str) {
        std::fs::write(self.evo_board(), text).unwrap();
    }

    fn run(&self, dry_run: bool) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(script()).arg(&self.repo);
        if dry_run {
            cmd.arg("--dry-run");
        }
        cmd.env("OLP_EVO_STATE", &self.state_root);
        cmd.output().unwrap()
    }

    fn run_env(&self, extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(script())
            .arg(&self.repo)
            .env("OLP_EVO_STATE", &self.state_root);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn project_dir(&self) -> PathBuf {
        std::fs::read_dir(&self.state_root)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().is_dir())
            .map(|e| e.path())
            .unwrap_or_else(|| self.state_root.clone())
    }

    fn retro_json(&self) -> PathBuf {
        self.project_dir().join("retro.json")
    }

    fn retro_dir(&self) -> PathBuf {
        self.project_dir().join("retro")
    }

    fn latest_brief_path(&self) -> PathBuf {
        let text = std::fs::read_to_string(self.retro_json()).unwrap_or_default();
        let mut best = PathBuf::new();
        if let Some(pos) = text.find("\"brief\": \"") {
            let rest = &text[pos + 10..];
            if let Some(end) = rest.find('"') {
                best = PathBuf::from(&rest[..end]);
            }
        }
        best
    }

    fn brief_text(&self) -> String {
        std::fs::read_to_string(self.latest_brief_path()).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Scenario: 不同错误码不合并
#[test]
fn olp_evo_retro_error_codes_are_distinct_candidates() {
    let sb = Sandbox::new("errcodes");
    // Contract Given: exactly the two ack_blocked cards differing only by
    // error code (the shared fixture adds a turn_error third card).
    sb.write_board(
        "### EVO-0001（t，harvest）
trigger: ack_blocked
source: review /repo/a/.octos/OUTER_LOOP_REVIEW.md
identity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#12#blocked#1111
symptom: ACK(blocked): cargo test fails with E0596 unresolved import
### EVO-0002（t，harvest）
trigger: ack_blocked
source: review /repo/a/.octos/OUTER_LOOP_REVIEW.md
identity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#13#blocked#2222
symptom: ACK(blocked): cargo test fails with E0382 unresolved import
",
    );
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief = sb.brief_text();
    // E0596 vs E0382 must NOT merge (letter+digits = code, preserved by <num>)
    assert!(
        brief.contains("candidates: 2"),
        "E0596 vs E0382 must stay distinct: {brief}"
    );
}

/// Scenario: events 卡按 detail 分组
#[test]
fn olp_evo_retro_events_group_by_detail() {
    let sb = Sandbox::new("events-detail");
    sb.write_board(
        "### EVO-0001（t，harvest）\ntrigger: turn_error\nsource: events /e.jsonl\nidentity: events:/e.jsonl#2026-09-05T01:00:00Z#turn_error#slug-a#1111\nsymptom: {\"ts\":\"2026-09-05T01:00:00Z\",\"kind\":\"turn_error\",\"slug\":\"slug-a\",\"data\":{\"detail\":\"writer stalled\"}}\n\
### EVO-0002（t，harvest）\ntrigger: turn_error\nsource: events /e.jsonl\nidentity: events:/e.jsonl#2026-09-05T02:00:00Z#turn_error#slug-b#2222\nsymptom: {\"ts\":\"2026-09-05T02:00:00Z\",\"kind\":\"turn_error\",\"slug\":\"slug-b\",\"data\":{\"detail\":\"provider 429\"}}\n",
    );
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief = sb.brief_text();
    assert!(brief.contains("candidates: 2"), "{brief}");
    let keys: Vec<&str> = brief
        .lines()
        .filter(|l| l.starts_with("key: "))
        .map(|l| &l[5..])
        .collect();
    assert!(
        keys.iter().any(|k| k.contains("writer stalled")),
        "{keys:?}"
    );
    assert!(keys.iter().any(|k| k.contains("provider")), "{keys:?}");
}

/// Scenario: 仅数字与路径不同的卡合并并数出锚点
#[test]
fn olp_evo_retro_merges_num_path_variants_and_counts_anchors() {
    let sb = Sandbox::new("merge");
    sb.install_board("board-two-candidates.md");
    // board-two-candidates has E0596 vs E0382 (distinct); craft merged pair
    sb.write_board(
        "### EVO-0001（t，harvest）\ntrigger: ack_blocked\nsource: review /repo/a/.octos/OUTER_LOOP_REVIEW.md\nidentity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#12#blocked#1111\nsymptom: ACK(blocked): cargo test fails at /src/main.rs with 3 errors\n\
### EVO-0002（t，harvest）\ntrigger: ack_blocked\nsource: review /repo/a/.octos/OUTER_LOOP_REVIEW.md\nidentity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#13#blocked#2222\nsymptom: ACK(blocked): cargo test fails at /src/lib.rs with 5 errors\n\
### EVO-0003（t，harvest）\ntrigger: turn_error\nsource: events /e.jsonl\nidentity: events:/e.jsonl#t#turn_error#s#3333\nsymptom: {\"detail\":\"writer stalled\"}\n",
    );
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief = sb.brief_text();
    assert!(brief.contains("candidates: 2"), "{brief}");
    assert!(
        brief.contains("recurrence_hint=2"),
        "the merged ack_blocked pair counts 2 anchors: {brief}"
    );
    let c1 = section(&brief, "## C1");
    assert!(
        c1.contains("anchors: 12, 13"),
        "anchors list both entries: {c1}"
    );
    let c2 = section(&brief, "## C2");
    assert!(
        c2.contains("layer=Execution"),
        "turn_error maps to Execution: {c2}"
    );
}

fn section<'a>(text: &'a str, header: &str) -> &'a str {
    let start = text.find(header).unwrap_or(text.len());
    let rest = &text[start..];
    let end = rest[header.len()..]
        .find("\n## ")
        .map(|i| start + header.len() + i)
        .unwrap_or(text.len());
    &text[start..end]
}

/// Scenario: 路径含井号时锚点仍正确
#[test]
fn olp_evo_retro_anchor_rsplit_survives_hash_in_path() {
    let sb = Sandbox::new("hash-path");
    sb.write_board(
        "### EVO-0001（t，harvest）\ntrigger: ack_blocked\nsource: review /repo/c#dir/board.md\nidentity: board:/repo/c#dir/board.md#27#blocked#aaaa\nsymptom: ACK(blocked): hash in path\n",
    );
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief = sb.brief_text();
    assert!(
        brief.contains("anchors: 27"),
        "rsplit anchor survives '#' in path: {brief}"
    );
}

/// Scenario: 锚点为减号的卡各计一次
#[test]
fn olp_evo_retro_dash_anchor_counts_each_card() {
    let sb = Sandbox::new("dash");
    sb.install_board("board-same-anchor.md");
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief = sb.brief_text();
    assert!(
        brief.contains("recurrence_hint=2"),
        "'-' anchors fall back to per-card EVO ids: {brief}"
    );
}

/// Scenario: 草稿标注 draft 且用下一个 FLAW 编号
#[test]
fn olp_evo_retro_draft_marks_todo_and_next_flaw_id() {
    let sb = Sandbox::new("draft");
    // records dir with FLAW-001/002 → next = 003
    let evo = sb.repo.join("knowledge/context/evolution");
    std::fs::create_dir_all(&evo).unwrap();
    std::fs::write(evo.join("FLAW-001.md"), "x").unwrap();
    std::fs::write(evo.join("FLAW-002.md"), "x").unwrap();
    sb.install_board("board-two-candidates.md");
    let out = sb.run(false);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let brief = sb.brief_text();
    assert!(brief.contains("id: FLAW-003"), "{brief}");
    assert!(brief.contains("id: FLAW-004"), "{brief}");
    let drafts = brief.matches("draft: true").count();
    assert!(drafts >= 2, "{brief}");
    let fps = brief.matches("fingerprint: TODO").count();
    assert!(fps >= 2, "{brief}");
    assert!(brief.contains("不得保存为 FLAW-NNN.md"), "{brief}");
    assert!(
        !evo.join("FLAW-003.md").exists(),
        "retro never writes records"
    );
}

/// Scenario: 简报与状态 schema 完整
#[test]
fn olp_evo_retro_brief_and_runs_schema() {
    let sb = Sandbox::new("schema");
    sb.install_board("board-two-candidates.md");
    let out1 = sb.run(false);
    assert!(out1.status.success());
    // append one more card and run again
    let mut text = std::fs::read_to_string(sb.evo_board()).unwrap();
    text.push_str("### EVO-0004（t，harvest）\ntrigger: ack_blocked\nsource: review /repo/a/.octos/OUTER_LOOP_REVIEW.md\nidentity: board:/repo/a/.octos/OUTER_LOOP_REVIEW.md#14#blocked#dddd\nsymptom: ACK(blocked): a fourth card entirely different text\n");
    std::fs::write(sb.evo_board(), text).unwrap();
    let out2 = sb.run(false);
    assert!(out2.status.success());

    let state_text = std::fs::read_to_string(sb.retro_json()).unwrap();
    assert!(state_text.contains("\"last_id\": 4"), "{state_text}");
    let runs = state_text.matches("\"run\":").count();
    assert_eq!(runs, 2, "{state_text}");
    let brief_path = sb.latest_brief_path();
    assert!(brief_path.exists(), "brief must exist: {brief_path:?}");
    // every brief has the schema lines
    let all_briefs = std::fs::read_dir(sb.retro_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();
    assert_eq!(all_briefs, 2);
    let brief = sb.brief_text();
    for needed in ["cards:", "candidates:", "note:"] {
        assert!(brief.contains(needed), "missing {needed}: {brief}");
    }
    for needed in [
        "key: ",
        "anchors: ",
        "cards: EVO-",
        "| source=",
        "| envelope=",
    ] {
        assert!(brief.contains(needed), "missing {needed}: {brief}");
    }
}

/// Scenario: 游标推进后重跑零新卡
#[test]
fn olp_evo_retro_cursor_advances_and_rerun_is_empty() {
    let sb = Sandbox::new("cursor");
    sb.install_board("board-two-candidates.md");
    let out1 = sb.run(false);
    assert!(out1.status.success());
    let out2 = sb.run(false);
    assert!(out2.status.success());
    let stdout = String::from_utf8_lossy(&out2.stdout);
    assert!(stdout.contains("retro: 0 new card(s)"), "stdout: {stdout}");
    let briefs = std::fs::read_dir(sb.retro_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();
    assert_eq!(briefs, 1, "no second brief on empty rerun");
}

/// Scenario: 并发两次运行不丢记录
#[test]
fn olp_evo_retro_concurrent_runs_keep_records() {
    let sb = Sandbox::new("concurrent");
    sb.install_board("board-two-candidates.md");
    let mut handles = Vec::new();
    for _ in 0..2 {
        let script = script();
        let repo = sb.repo.clone();
        let state = sb.state_root.clone();
        handles.push(std::thread::spawn(move || {
            Command::new("bash")
                .arg(script)
                .arg(repo)
                .env("OLP_EVO_STATE", state)
                .output()
                .unwrap()
        }));
    }
    let mut empty_seen = false;
    for h in handles {
        let out = h.join().unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if String::from_utf8_lossy(&out.stdout).contains("retro: 0 new card(s)") {
            empty_seen = true;
        }
    }
    assert!(empty_seen, "one of the two runs must see 0 new cards");
    let state_text = std::fs::read_to_string(sb.retro_json()).unwrap();
    let runs = state_text.matches("\"run\":").count();
    assert_eq!(runs, 1, "{state_text}");
}

/// Scenario: 简报落盘后写游标前崩溃可恢复
#[test]
fn olp_evo_retro_recovers_after_crash_before_cursor() {
    let sb = Sandbox::new("crash");
    sb.install_board("board-two-candidates.md");
    let out = sb.run_env(&[("OLP_EVO_TEST", "1"), ("OLP_EVO_FAULT", "after-brief")]);
    assert_eq!(out.status.code(), Some(70));
    let out2 = sb.run(false);
    assert!(
        out2.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let state_text = std::fs::read_to_string(sb.retro_json()).unwrap();
    assert!(
        state_text.contains("\"last_id\": 3"),
        "last_id = max card number: {state_text}"
    );
    let runs = state_text.matches("\"run\":").count();
    assert_eq!(runs, 1, "{state_text}");
}

/// Scenario: dry-run 零写入
#[test]
fn olp_evo_retro_dry_run_writes_nothing() {
    let sb = Sandbox::new("dry");
    sb.install_board("board-two-candidates.md");
    let out = sb.run(true);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("candidates:"), "{stdout}");
    // state dir must contain NOTHING retro-related
    assert!(!sb.retro_json().exists());
    assert!(!sb.project_dir().join("retro.lock").exists());
    assert!(!sb.retro_dir().exists());
}

/// Scenario: 畸形卡被报告一次并跳过
#[test]
fn olp_evo_retro_malformed_card_reported_once() {
    let sb = Sandbox::new("malformed");
    sb.install_board("board-malformed.md");
    let out1 = sb.run(false);
    assert!(
        out1.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    let stderr1 = String::from_utf8_lossy(&out1.stderr);
    assert!(stderr1.contains("malformed-card: EVO-0001"), "{stderr1}");
    let brief = sb.brief_text();
    assert!(brief.contains("candidates: 1"), "{brief}");
    let out2 = sb.run(false);
    assert!(out2.status.success());
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        !stderr2.contains("malformed-card:"),
        "second run must not re-report: {stderr2}"
    );
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(stdout2.contains("retro: 0 new card(s)"), "{stdout2}");
}

/// Scenario: 无新卡退出 0
#[test]
fn olp_evo_retro_no_cards_exit_zero() {
    let sb = Sandbox::new("empty");
    // no EVOLUTION.md at all
    let out = sb.run(false);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("retro: 0 new card(s)"), "{stdout}");
    assert!(!sb.retro_dir().exists());
}

/// #42c: skill outer step 5 present; protected sections byte-identical
/// to the phase-0 baseline (origin/main 18907aa).
#[test]
fn olp_evo_retro_skill_step5_and_protected_sections_golden() {
    let skill =
        std::fs::read_to_string(repo_root().join(".claude/skills/octoloop/SKILL.md")).unwrap();

    // protected-section constants (sha256, baseline 18907aa)
    const DESC: &str = "b2c32593cc6a5dacbc6e42ded9186c9ac94171c0d9d1883aa19f6ac760c5ccb4";
    const INIT: &str = "71cd93889dcf5a437f0cd35e08ce069e1f8f589f2d0243553dd9017d6d73bcee";
    const INNER: &str = "1e21d03921302a2125df06eb439bcc02edf14e1dc91bf77395238f8b78289597";
    const DISC: &str = "adbb09472cc51deae4184c80c989e916ba9d1c03360487bfaa86a5c22a263f84";

    let desc_line = skill
        .lines()
        .find(|l| l.starts_with("description:"))
        .expect("description line");
    assert_eq!(sha256_str(desc_line), DESC);

    fn section<'a>(text: &'a str, header: &str) -> &'a str {
        let start = text.find(header).expect("header");
        let idx = text[start + header.len()..]
            .find("\n## ")
            .map(|i| start + header.len() + i);
        let end = idx.unwrap_or(text.len());
        &text[start..end]
    }
    assert_eq!(
        sha256_str(&format!("{}\n", section(&skill, "## 模式 init"))),
        INIT
    );
    assert_eq!(
        sha256_str(&format!("{}\n", section(&skill, "## 模式 inner"))),
        INNER
    );
    assert_eq!(
        sha256_str(&format!("{}\n", section(&skill, "## 自主性纪律"))),
        DISC
    );

    let outer = section(&skill, "## 模式 outer");
    assert!(outer.contains("上岗五步"), "outer must list five steps");
    assert!(outer.contains("olp-evo-retro.sh"));
    assert!(outer.contains("R2 记档"));
    assert!(outer.contains("operator"));
}

fn sha256_str(s: &str) -> String {
    // sha256sum-compatible hex via the sha2 crate already in the tree
    use sha2::Digest as _;
    let d = sha2::Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// #42c: BOOT §7 has the adjudication forms; §0-§6 byte-identical.
#[test]
fn olp_evo_retro_boot_section7_and_golden() {
    let boot = std::fs::read_to_string(repo_root().join("docs/OLP_OUTER_BOOT.md")).unwrap();

    const BOOT_0_TO_6: &str = "89a820c60e5758db9be3d8e6c8573f9b0b15101fa9234ef92dd008c00e464836";
    let slice = &boot[boot.find("## 0.").unwrap()..boot.find("## 7.").unwrap()];
    assert_eq!(sha256_str(slice), BOOT_0_TO_6);

    let s7 = &boot[boot.find("## 7.").unwrap()..];
    assert!(s7.contains("改判(作废 #"), "override form present");
    assert!(s7.contains("R2 记档(#"), "r2-record form present");
    assert!(s7.contains("未 ACK"), "un-ACK rule present");
}

/// Scenario: issue 模板与 FEATURES 就位
#[test]
fn olp_evo_retro_issue_template_and_features_in_place() {
    let tmpl =
        std::fs::read_to_string(repo_root().join("knowledge/context/evolution/ISSUE-template.md"))
            .unwrap();
    for section in [
        "## Summary",
        "## Environment",
        "## Reproduction",
        "## Root cause",
        "## Expected behavior",
        "## Tests requested",
        "## Related",
    ] {
        assert!(tmpl.contains(section), "missing {section}");
    }
    for fm in ["repo:", "evo:", "layers:", "severity:"] {
        assert!(tmpl.contains(fm), "missing frontmatter {fm}");
    }
    let features = std::fs::read_to_string(repo_root().join("docs/OCTOLOOP_FEATURES.md")).unwrap();
    assert!(features.contains("外环私有工作纸"));
}
