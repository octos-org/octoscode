//! Evolution loop phase-3 index contract tests (#44b).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn index_script() -> PathBuf {
    repo_root().join("scripts/olp-evo-index.sh")
}

fn fixture_repo() -> PathBuf {
    // copy fixtures/evolution/index → temp so INDEX.md writes stay sandboxed
    // 44-r1: per-TEST temp dir (parallel-safe), cleaned by the caller
    let dst = std::env::temp_dir().join(format!(
        "olp-evo-index-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("t")
    ));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(&repo_root().join("fixtures/evolution/index"), &dst);
    dst
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

fn mtime(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p).unwrap().modified().unwrap()
}

/// Scenario: 索引生成并识别退役散文
#[test]
fn olp_evo_index_lists_flaws_and_retired_prose() {
    let repo = fixture_repo();
    let out = Command::new("bash")
        .arg(index_script())
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let index = std::fs::read_to_string(repo.join("knowledge/context/evolution/INDEX.md")).unwrap();
    let flaw_rows = index.lines().filter(|l| l.starts_with("| FLAW-00")).count();
    assert_eq!(flaw_rows, 2, "{index}");
    let row1 = index
        .lines()
        .find(|l| l.starts_with("| FLAW-001"))
        .expect("row");
    assert!(
        !row1.contains("| — | — | — | — |"),
        "FLAW-001 取代散文列非 —: {row1}"
    );
    assert!(row1.contains("预算硬约束"), "{row1}");
    // 44-r2: issue/PR same column — both → "<issue> / <pr>".
    assert!(
        row1.contains("https://example/1 / https://example/pr1"),
        "FLAW-001 issue/PR combined column: {row1}"
    );

    let row2 = index
        .lines()
        .find(|l| l.starts_with("| FLAW-002"))
        .expect("row");
    assert!(row2.ends_with("| — |"), "FLAW-002 取代散文为 —: {row2}");
    // 44-r2: neither issue nor pr → dash in the combined column.
    let cols2: Vec<&str> = row2.trim().trim_matches('|').split('|').collect();
    assert_eq!(
        cols2.get(3).map(|c| c.trim()),
        Some("—"),
        "FLAW-002 combined issue/PR column is dash: {row2}"
    );
    assert!(index.trim_end().ends_with("retired_prose: 1"), "{index}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// Scenario: 索引幂等不改 mtime
#[test]
fn olp_evo_index_is_idempotent() {
    let repo = fixture_repo();
    let out = Command::new("bash")
        .arg(index_script())
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let idx = repo.join("knowledge/context/evolution/INDEX.md");
    let before_text = std::fs::read_to_string(&idx).unwrap();
    let before_m = mtime(&idx);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let out2 = Command::new("bash")
        .arg(index_script())
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out2.status.success());
    let after_text = std::fs::read_to_string(&idx).unwrap();
    let after_m = mtime(&idx);
    assert_eq!(before_text, after_text);
    assert_eq!(before_m, after_m, "identical content must preserve mtime");
    let _ = std::fs::remove_dir_all(&repo);
}

/// 44-r2: issue AND pr both present → combined `<issue> / <pr>` column.
#[test]
fn olp_evo_index_lists_issue_and_pr_together() {
    let repo = fixture_repo();
    let out = Command::new("bash")
        .arg(index_script())
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let idx = std::fs::read_to_string(repo.join("knowledge/context/evolution/INDEX.md")).unwrap();
    let row1 = idx.lines().find(|l| l.starts_with("| FLAW-001")).unwrap();
    assert!(
        row1.contains("https://example/1 / https://example/pr1"),
        "{row1}"
    );
    let row2 = idx.lines().find(|l| l.starts_with("| FLAW-002")).unwrap();
    let cols: Vec<&str> = row2.trim().trim_matches('|').split('|').collect();
    assert_eq!(cols.get(3).map(|c| c.trim()), Some("—"), "{row2}");
    let _ = std::fs::remove_dir_all(&repo);
}
