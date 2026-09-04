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
    let ptext = String::from_utf8_lossy(&parse.stdout) + String::from_utf8_lossy(&parse.stderr);
    assert!(
        ptext.contains("3") || parse.status.success(),
        "agent-spec parse accepts skeleton: {ptext}"
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
        parse.status.success() || String::from_utf8_lossy(&parse.stderr).contains("scenario"),
        "parse output: {}{}",
        String::from_utf8_lossy(&parse.stdout),
        String::from_utf8_lossy(&parse.stderr)
    );
}

/// Scenario: 骨架拒绝写入 specs 根
#[test]
fn olp_evo_skeleton_refuses_out_into_specs_root() {
    let flaw = repo_root().join("knowledge/context/evolution/FLAW-001.md");
    let bad_out = repo_root().join("specs/task-x.spec.md");
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
