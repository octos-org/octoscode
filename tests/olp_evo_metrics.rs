//! Evolution loop phase-2 replay + metrics contract tests (#43c, SDD
//! spec: `specs/task-req-olp-evo-p2.spec.md`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn metrics() -> PathBuf {
    repo_root().join("scripts/olp-evo-metrics.sh")
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Sb {
    #[allow(dead_code)]
    root: PathBuf,
    repo: PathBuf,
    state_root: PathBuf,
}

impl Sb {
    fn new(tag: &str) -> Self {
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("olp-p2-{tag}-{}-{seq}", std::process::id()));
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
}

impl Drop for Sb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Scenario: 指标文本输出
#[test]
fn olp_evo_metrics_text_output() {
    let sb = Sb::new("metrics-text");
    // 5 cards + a brief with one goal_blocked hint=2 candidate
    let mut board = String::new();
    for (i, t) in [
        "ack_blocked",
        "ack_blocked",
        "goal_blocked",
        "goal_blocked",
        "turn_error",
    ]
    .into_iter()
    .enumerate()
    {
        board.push_str(&format!(
            "### EVO-{:04}（t，harvest）\ntrigger: {t}\nsource: events /e\n\n",
            i + 1
        ));
    }
    std::fs::write(sb.repo.join(".octos/EVOLUTION.md"), board).unwrap();
    let retro_dir = sb.state_root.join("proj/retro");
    std::fs::create_dir_all(&retro_dir).unwrap();
    std::fs::write(
        retro_dir.join("2026-09-05T00-00-00Z-1.md"),
        "# retro t · k · run 1\n\n## C1 goal_blocked · recurrence_hint=2 · layer=Lifecycle\n",
    )
    .unwrap();
    let out = Command::new("bash")
        .arg(metrics())
        .arg(&sb.repo)
        .env("OLP_EVO_STATE", &sb.state_root)
        .env("OLP_EVO_RETRO_DIR", &retro_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cards: 5"), "{stdout}");
    assert!(
        stdout.contains("by_trigger: ack_blocked=2 goal_blocked=2 turn_error=1"),
        "{stdout}"
    );
    assert!(stdout.contains("recurring_candidates: 1"), "{stdout}");
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("- goal_blocked hint=2")),
        "{stdout}"
    );
}

/// Scenario: 指标 JSON 与 since
#[test]
fn olp_evo_metrics_json_and_since() {
    let sb = Sb::new("metrics-json");
    let mut board = String::new();
    for (i, t) in [
        "ack_blocked",
        "ack_blocked",
        "goal_blocked",
        "goal_blocked",
        "turn_error",
    ]
    .into_iter()
    .enumerate()
    {
        board.push_str(&format!(
            "### EVO-{:04}（t，harvest）\ntrigger: {t}\nsource: events /e\n\n",
            i + 1
        ));
    }
    std::fs::write(sb.repo.join(".octos/EVOLUTION.md"), board).unwrap();
    let out = Sb::run_env(
        &metrics(),
        &[sb.repo.to_str().unwrap(), "--json", "--since", "EVO-0003"],
        &sb.state_root,
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["cards"].as_u64().unwrap(), 2, "{stdout}");
}

/// Scenario: 基线比对标注回归
#[test]
fn olp_evo_metrics_baseline_flags_regress() {
    let sb = Sb::new("metrics-base");
    let mut board = String::new();
    for (i, t) in ["ack_blocked", "ack_blocked", "goal_blocked", "goal_blocked"]
        .into_iter()
        .enumerate()
    {
        board.push_str(&format!(
            "### EVO-{:04}（t，harvest）\ntrigger: {t}\nsource: events /e\n\n",
            i + 1
        ));
    }
    std::fs::write(sb.repo.join(".octos/EVOLUTION.md"), board).unwrap();
    let baseline = sb.root.join("baseline.json");
    std::fs::write(
        &baseline,
        r#"{"cards": 3, "by_trigger": {"ack_blocked": 3, "goal_blocked": 1}}"#,
    )
    .unwrap();
    let base_str = baseline.to_str().unwrap().to_string();
    let out = Sb::run_env(
        &metrics(),
        &[sb.repo.to_str().unwrap(), "--baseline", &base_str],
        &sb.state_root,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("regress: goal_blocked 1->2"), "{stdout}");
    assert!(
        !stdout.contains("regress: ack_blocked"),
        "decrease must not be flagged: {stdout}"
    );
}

/// Scenario: 指标脚本零写入
#[test]
fn olp_evo_metrics_writes_nothing() {
    let sb = Sb::new("metrics-zero");
    std::fs::write(
        sb.repo.join(".octos/EVOLUTION.md"),
        "### EVO-0001（t，harvest）\ntrigger: ack_blocked\nsource: events /e\n",
    )
    .unwrap();
    let digest = |p: &Path| -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        for e in walk_files(p) {
            let h = Command::new("sha256sum").arg(&e).output().unwrap();
            let h = String::from_utf8_lossy(&h.stdout)
                .split_whitespace()
                .next()
                .unwrap()
                .to_string();
            out.push((e.clone(), h));
        }
        out.sort();
        out
    };
    let before_repo = digest(&sb.repo);
    let before_state = digest(&sb.state_root);
    let out = Sb::run_env(&metrics(), &[sb.repo.to_str().unwrap()], &sb.state_root);
    assert!(out.status.success());
    assert_eq!(digest(&sb.repo), before_repo);
    assert_eq!(digest(&sb.state_root), before_state);
}

/// Scenario: 缺少 retro 状态时仍可输出
#[test]
fn olp_evo_metrics_without_retro_state() {
    let sb = Sb::new("metrics-nostate");
    std::fs::write(
        sb.repo.join(".octos/EVOLUTION.md"),
        "### EVO-0001（t，harvest）\ntrigger: ack_blocked\nsource: events /e\n",
    )
    .unwrap();
    let out = Sb::run_env(&metrics(), &[sb.repo.to_str().unwrap()], &sb.state_root);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("recurring_candidates: 0"), "{stdout}");
}

impl Sb {
    fn run_env(script: &Path, args: &[&str], state_root: &Path) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(script);
        for a in args {
            cmd.arg(a);
        }
        cmd.env("OLP_EVO_STATE", state_root);
        cmd.output().unwrap()
    }
}

fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk_files(&p));
        } else {
            out.push(p);
        }
    }
    out
}
