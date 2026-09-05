//! Evolution loop phase-3 skeleton + index contract tests (#44b).

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn skeleton() -> PathBuf {
    repo_root().join("scripts/olp-evo-spec-skeleton.sh")
}

fn fixture(rel: &str) -> PathBuf {
    repo_root().join("fixtures/evolution").join(rel)
}

fn run(script: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new("bash")
        .arg(script)
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap()
}

/// Scenario: FLAW 直出骨架可解析
#[test]
fn olp_evo_skeleton_from_template_flaw_parses_and_maps_sections() {
    let out = run(
        &skeleton(),
        &[&fixture("skeleton/FLAW-sample.md").to_string_lossy()],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("satisfies: [REQ-OLP-EVO]"), "{stdout}");
    let pending_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("测试: pending_"))
        .collect();
    assert_eq!(
        pending_lines.len(),
        3,
        "three 修复 decisions → three scenarios: {stdout}"
    );
    // 44b-r1: selectors stay short (pending_ + ≤48-char slug = ≤56)
    for l in &pending_lines {
        let sel = l.trim().trim_start_matches("测试: ");
        assert!(
            sel.len() <= 56,
            "selector {sel} exceeds 56 chars ({}): {stdout}",
            sel.len()
        );
    }
    // 预防 items land under Forbidden
    let forbid_pos = stdout.find("### Forbidden").expect("Forbidden section");
    let after = &stdout[forbid_pos..];
    let forbid_end = after
        .find("\n## ")
        .map(|i| forbid_pos + i)
        .unwrap_or(stdout.len());
    let forbidden = &stdout[forbid_pos..forbid_end];
    assert!(
        forbidden.contains("预防项一") && forbidden.contains("预防项三"),
        "预防 items under Forbidden: {forbidden}"
    );
    // agent-spec parse accepts it and counts 3 scenarios
    let parse = Command::new("agent-spec")
        .args(["parse", "/dev/stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(stdout.as_bytes()).ok();
            c.wait_with_output()
        })
        .unwrap();
    // 44-r1: STRICT — exit 0 AND the scenario-count line says 3.
    assert!(
        parse.status.success(),
        "agent-spec parse must exit 0: {}{}",
        String::from_utf8_lossy(&parse.stdout),
        String::from_utf8_lossy(&parse.stderr)
    );
    let ptext = format!(
        "{}{}",
        String::from_utf8_lossy(&parse.stdout),
        String::from_utf8_lossy(&parse.stderr)
    );
    assert!(
        ptext.contains("3 scenarios"),
        "parse must report exactly 3 scenarios: {ptext}"
    );
}

/// Scenario: 真实 FLAW-001 走别名段且缺段占位
#[test]
fn olp_evo_skeleton_real_flaw_uses_aliases_and_todo() {
    let flaw = repo_root().join("knowledge/context/evolution/FLAW-001.md");
    let out = run(&skeleton(), &[&flaw.to_string_lossy()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("satisfies: []"), "{stdout}");
    assert!(stdout.contains("未绑定需求"), "{stdout}");
    assert!(
        stdout.lines().any(|l| l.contains("测试: pending_")),
        "{stdout}"
    );
    // 44b-r1: the REAL FLAW-001 skeleton's selectors stay ≤ 56 chars
    for l in stdout.lines().filter(|l| l.contains("测试: pending_")) {
        let sel = l.trim().trim_start_matches("测试: ");
        assert!(
            sel.len() <= 56,
            "FLAW-001 selector {sel} exceeds 56 ({}): {stdout}",
            sel.len()
        );
    }
    // 结案 alias feeds decisions; 锚点 paths land in Allowed
    assert!(
        stdout.contains("crates/octos-cli/src/peers/mod.rs"),
        "paths in Allowed: {stdout}"
    );
    // parseable by agent-spec
    let parse = Command::new("agent-spec")
        .args(["parse", "/dev/stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(stdout.as_bytes()).ok();
            c.wait_with_output()
        })
        .unwrap();
    assert!(
        parse.status.success(),
        "agent-spec parse must exit 0: {}{}",
        String::from_utf8_lossy(&parse.stdout),
        String::from_utf8_lossy(&parse.stderr)
    );
}

/// Scenario: 骨架拒绝写入 specs 根(仓库内外两种 cwd)
#[test]
fn olp_evo_skeleton_refuses_out_into_specs_root() {
    let flaw = repo_root().join("knowledge/context/evolution/FLAW-001.md");
    let bad_out = repo_root().join("specs/task-x.spec.md");
    // from INSIDE the repo (current_dir = repo root)
    let out = run(
        &skeleton(),
        &[&flaw.to_string_lossy(), "--out", &bad_out.to_string_lossy()],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("refusing to write outside specs/drafts/"),
        "{stderr}"
    );
    assert!(!bad_out.exists());
    // from OUTSIDE the repo: cwd = temp dir, absolute script + args
    let outside = std::env::temp_dir().join(format!("skel-out-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    let out2 = Command::new("bash")
        .arg(skeleton().canonicalize().unwrap())
        .arg(flaw.canonicalize().unwrap())
        .arg("--out")
        .arg(&bad_out)
        .current_dir(&outside)
        .output()
        .unwrap();
    assert_eq!(
        out2.status.code(),
        Some(2),
        "outside-cwd run: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr2.contains("refusing to write outside specs/drafts/"),
        "{stderr2}"
    );
    assert!(!bad_out.exists());
    let _ = std::fs::remove_dir_all(&outside);
}

/// README kind 候选 section in place.
#[test]
fn olp_evo_readme_lists_kind_candidates() {
    let readme =
        std::fs::read_to_string(repo_root().join("knowledge/context/evolution/README.md")).unwrap();
    assert!(readme.contains("## kind 候选"), "{readme}");
    assert!(readme.contains("`iteration_cap`"), "{readme}");
    assert!(readme.contains("`patch_failed`"), "{readme}");
    assert!(readme.contains("48b 中断记录"), "{readme}");
    assert!(readme.contains("## 索引"), "{readme}");
    // PROTOCOL 登记行
    let protocol =
        std::fs::read_to_string(repo_root().join("docs/OUTER_LOOP_PROTOCOL.md")).unwrap();
    assert!(
        protocol.contains("> 已登记:kind 候选 iteration_cap"),
        "{protocol}"
    );
}

/// Scenario: 骨架字段精确映射(44-r1)
#[test]
fn olp_evo_skeleton_maps_root_cause_paths_and_item_slug_exactly() {
    let root = std::env::temp_dir().join(format!(
        "olp-skel-map-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("t")
    ));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let flaw = root.join("FLAW-map.md");
    std::fs::write(
        &flaw,
        "# FLAW-000\n\n## 症状\n症状是重试后超时\n\n## 根因\n根因文本甲乙丙\n\n## 责任步\n- `src/worker.rs:123` 执行\n\n## 修复\n- 纯中文修复项\n",
    )
    .unwrap();
    let out = Command::new("bash")
        .arg(skeleton())
        .arg(&flaw)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let intent = stdout
        .split("## 意图")
        .nth(1)
        .and_then(|t| t.split("## 已定决策").next())
        .unwrap_or_default();
    assert!(intent.contains("根因文本甲乙丙"), "{stdout}");
    assert!(
        stdout.lines().any(|l| l.trim() == "- src/worker.rs"),
        "path line without lineno: {stdout}"
    );
    assert!(!stdout.contains("src/worker.rs:123"), "{stdout}");
    assert!(
        stdout.lines().any(|l| l.trim() == "测试: pending_item_1"),
        "pure-CJK item → pending_item_1: {stdout}"
    );
    assert!(!stdout.contains("pending_pending_"), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}
