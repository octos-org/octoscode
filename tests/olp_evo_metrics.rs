//! Evolution loop phase-2 metrics + shared-lib contract tests (#43c-2,
//! SDD spec v2: `specs/task-req-olp-evo-p2.spec.md`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn metrics_script() -> PathBuf {
    repo_root().join("scripts/olp-evo-metrics.sh")
}

fn retro_script() -> PathBuf {
    repo_root().join("scripts/olp-evo-retro.sh")
}

fn write_board(repo: &Path, cards: &[(&str, &str, &str)]) {
    // (num, trigger, source)
    let mut text = String::new();
    for (i, (trigger, source, symptom)) in cards.iter().enumerate() {
        let id_hash = format!("{:016x}", i + 1);
        text.push_str(&format!(
            "### EVO-{:04}（t，harvest）\ntrigger: {trigger}\nsource: {source}\nidentity: events:/e.jsonl#t#{trigger}#sess-{i}#{id_hash}\nsymptom: {symptom}\n\n",
            i + 1,
        ));
    }
    std::fs::write(repo.join(".octos/EVOLUTION.md"), text).unwrap();
}

/// Scenario: 指标文本输出
#[test]
fn olp_evo_metrics_text_output() {
    let root = std::env::temp_dir().join(format!("m-text-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    write_board(
        &repo,
        &[
            (
                "ack_blocked",
                "review /r",
                "ACK(blocked): inner stuck on step 3",
            ),
            (
                "ack_blocked",
                "review /r",
                "ACK(blocked): inner stuck on step 4",
            ),
            (
                "goal_blocked",
                "events /e",
                "{\"detail\":\"goal transitioned to `blocked`\"}",
            ),
            (
                "goal_blocked",
                "events /e",
                "{\"detail\":\"goal transitioned to `blocked`\"}",
            ),
            (
                "turn_error",
                "events /e",
                "{\"detail\":\"connection closed\"}",
            ),
        ],
    );
    let out = Command::new("bash")
        .arg(metrics_script())
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("note: diagnostic only, not a KPI"),
        "{stdout}"
    );
    assert!(stdout.contains("through_evo: EVO-0005"), "{stdout}");
    assert!(stdout.contains("cards: 5"), "{stdout}");
    assert!(
        stdout.contains("by_trigger: ack_blocked=2 goal_blocked=2 turn_error=1"),
        "{stdout}"
    );
    assert!(
        stdout.contains("by_source: review=2 events=3 mcp=0"),
        "{stdout}"
    );
    assert!(stdout.contains("recurring_candidates: 2"), "{stdout}");
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("- ack_blocked hint=2")),
        "{stdout}"
    );
    assert!(!stdout.contains("regress"), "no regress wording: {stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: 指标 JSON 与 since 窗口
#[test]
fn olp_evo_metrics_json_and_since_window() {
    let root = std::env::temp_dir().join(format!("m-json-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    write_board(
        &repo,
        &[
            ("ack_blocked", "review /r", "ACK(blocked): a"),
            ("ack_blocked", "review /r", "ACK(blocked): b"),
            ("goal_blocked", "events /e", "{\"detail\":\"d\"}"),
            ("goal_blocked", "events /e", "{\"detail\":\"d\"}"),
            ("turn_error", "events /e", "{\"detail\":\"c\"}"),
        ],
    );
    let out = Command::new("bash")
        .arg(metrics_script())
        .arg(&repo)
        .args(["--json", "--since", "EVO-0003"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["cards"].as_u64().unwrap(), 2, "{stdout}");
    assert_eq!(v["through_evo"].as_str().unwrap(), "EVO-0005");
    assert!(
        v["note"]
            .as_str()
            .unwrap()
            .contains("diagnostic only, not a KPI")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: 基线窗口诊断
#[test]
fn olp_evo_metrics_baseline_window_diagnostics() {
    let root = std::env::temp_dir().join(format!("m-base-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    // current: goal_blocked 2, ack_blocked 2 (ids 3,4 ack; wait, order:
    // we want goal_blocked grew 1->2 and ack fell 3->2)
    write_board(
        &repo,
        &[
            ("ack_blocked", "review /r", "ACK(blocked): a"),
            ("ack_blocked", "review /r", "ACK(blocked): b"),
            ("goal_blocked", "events /e", "{\"detail\":\"d\"}"),
            ("goal_blocked", "events /e", "{\"detail\":\"d\"}"),
        ],
    );
    // baseline covering the SAME window start (since EVO-0000) with
    // through_evo EVO-0004 and counts goal_blocked=1 ack_blocked=3
    let baseline = root.join("base.json");
    std::fs::write(
        &baseline,
        r#"{"note":"x","through_evo":"EVO-0000","cards":4,"by_trigger":{"ack_blocked":3,"goal_blocked":1},"by_source":{"review":3,"events":1,"mcp":0},"recurring_candidates":0,"recurring":[],"deltas":[]}"#,
    )
    .unwrap();
    let out = Command::new("bash")
        .arg(metrics_script())
        .arg(&repo)
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "diagnostics always exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("increase: goal_blocked 1->2"), "{stdout}");
    assert!(
        stdout.contains("decrease: ack_blocked 3->2"),
        "decreases are diagnostics too: {stdout}"
    );
    assert!(!stdout.contains("regress"), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: 指标脚本零写入
#[test]
fn olp_evo_metrics_writes_nothing() {
    let root = std::env::temp_dir().join(format!("m-zero-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    write_board(&repo, &[("ack_blocked", "review /r", "ACK(blocked): a")]);
    // 43-r2: compare a COPY of scripts/ (imports happen there), the repo
    // tree, and the state root — three surfaces, unchanged.
    let scripts_copy = root.join("scripts-copy");
    copy_dir(&repo_root().join("scripts"), &scripts_copy);
    let digest = |p: &Path| -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        for e in walk(p) {
            let h = Command::new("sha256sum").arg(&e).output().unwrap();
            let h = String::from_utf8_lossy(&h.stdout)
                .split_whitespace()
                .next()
                .unwrap()
                .to_string();
            out.push((e, h));
        }
        out.sort();
        out
    };
    let before_repo = digest(&repo);
    let before_scripts = digest(&scripts_copy);
    let before_state = digest(&state);
    let out = Command::new("bash")
        .arg(metrics_script())
        .arg(&repo)
        .env("OLP_EVO_STATE", &state)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(digest(&repo), before_repo, "repo unchanged");
    assert_eq!(
        digest(&scripts_copy),
        before_scripts,
        "scripts copy unchanged (no __pycache__)"
    );
    assert_eq!(digest(&state), before_state, "state root unchanged");
    let _ = std::fs::remove_dir_all(&root);
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        let t = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

/// Scenario: 空黑板仍可输出
#[test]
fn olp_evo_metrics_empty_board_outputs_zero() {
    let root = std::env::temp_dir().join(format!("m-empty-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    // no EVOLUTION.md at all
    let out = Command::new("bash")
        .arg(metrics_script())
        .arg(&repo)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cards: 0"), "{stdout}");
    assert!(stdout.contains("through_evo: EVO-0000"), "{stdout}");
    assert!(stdout.contains("recurring_candidates: 0"), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario: retro 与 metrics 共享同一 lib
#[test]
fn olp_evo_lib_shared_by_retro_and_metrics() {
    // Both scripts import scripts/olp-evo-lib.py — assert the import is
    // literally present in each (no duplicated inline implementation).
    let metrics = std::fs::read_to_string(metrics_script()).unwrap();
    let retro = std::fs::read_to_string(retro_script()).unwrap();
    for (name, text) in [("metrics", &metrics), ("retro", &retro)] {
        assert!(
            text.contains("olp-evo-lib.py"),
            "{name} must load the shared lib"
        );
    }
    // and the grouping logic must NOT be duplicated inline: the lib is the
    // only place with the LAYER table.
    assert!(
        !metrics.contains("\"ack_blocked\": \"Lifecycle\""),
        "metrics must not inline the layer table"
    );
    assert!(
        !retro.contains("\"ack_blocked\": \"Lifecycle\""),
        "retro must not inline the layer table"
    );
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
