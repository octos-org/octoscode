#![cfg(unix)]

//! Contract tests for the portable `/octoloop` entry (SDD spec:
//! `specs/task-olp-skill-portable.spec.md`): one agent-neutral body in
//! `skills/octoloop/AGENTS.md`, thin entries that carry no fixed path, and
//! `scripts/olp-install-skill.sh` to install one.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn install(home: &Path, args: &[&str]) -> Output {
    Command::new("/bin/bash")
        .arg(repo_root().join("scripts/olp-install-skill.sh"))
        .args(args)
        .env("HOME", home)
        .env_remove("OCTOSCODE_HOME")
        .output()
        .unwrap()
}

fn description(text: &str) -> &str {
    text.lines()
        .find_map(|l| l.strip_prefix("description:"))
        .expect("description line")
        .trim()
}

/// The canonical checkout path, the way the installer records it.
fn canonical_repo() -> String {
    std::fs::canonicalize(repo_root())
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Scenario: 规程正文只有一份,仓库内的入口是不含路径的薄卡
#[test]
fn skill_body_is_agent_neutral_and_the_claude_entry_is_thin() {
    let body = std::fs::read_to_string(repo_root().join("skills/octoloop/AGENTS.md")).unwrap();
    let entry =
        std::fs::read_to_string(repo_root().join(".claude/skills/octoloop/SKILL.md")).unwrap();

    // One body: the entry names it and repeats none of its sections.
    assert!(entry.contains("skills/octoloop/AGENTS.md"), "{entry}");
    for section in [
        "## 模式 init",
        "## 模式 outer",
        "## 模式 inner",
        "## 自主性纪律",
    ] {
        assert!(body.contains(section), "body has {section}");
        assert!(!entry.contains(section), "entry must not copy {section}");
    }
    // Claude Code lists the skill by this line; it must not drift.
    assert_eq!(description(&entry), description(&body));
    // The body says how its relative paths resolve outside this repository.
    assert!(
        body.contains("OCTOSCODE_HOME") && body.contains("octoscode-home"),
        "{body}"
    );
    for text in [&body, &entry] {
        assert!(
            !text.contains("/Users/") && !text.contains("/home/"),
            "no fixed path"
        );
    }
}

/// Scenario: 文档地图覆盖每一份 OctoLoop 文档,指向的文件都存在
#[test]
fn skill_document_map_reaches_every_octoloop_document() {
    let root = repo_root();
    let body = std::fs::read_to_string(root.join("skills/octoloop/AGENTS.md")).unwrap();
    let map_start = body
        .find("## 文档地图")
        .expect("the skill has a document map");
    let map_end = map_start
        + body[map_start..]
            .find("\n## 模式 init")
            .expect("map precedes init");
    let map = &body[map_start..map_end];

    // Every OctoLoop document in docs/ is reachable from the map. A document
    // nobody links to is a document no agent will read.
    let mut docs: Vec<String> = std::fs::read_dir(root.join("docs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.ends_with(".md")
                && (n.starts_with("OLP_")
                    || n.starts_with("OCTOLOOP_")
                    || n == "OUTER_LOOP_PROTOCOL.md")
        })
        .collect();
    docs.sort();
    assert!(docs.len() >= 10, "{docs:?}");
    for doc in &docs {
        assert!(
            map.contains(&format!("`docs/{doc}`")),
            "the document map does not mention docs/{doc}"
        );
    }

    // Everything the skill points at exists: documents and scripts.
    let mut checked = 0;
    for span in body.split('`').skip(1).step_by(2) {
        let path = span.split_whitespace().next().unwrap_or("");
        if (path.starts_with("docs/") && path.ends_with(".md")) || path.starts_with("scripts/olp-")
        {
            assert!(
                root.join(path).is_file(),
                "the skill points at a missing file: {path}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 40, "only {checked} paths were checked");

    // The user-facing tools of the multi-machine deployment are named.
    for tool in [
        "scripts/olp-systemd.py",
        "scripts/olp-launchd.py",
        "scripts/olp-git-hub.py",
        "scripts/olp-alert-forward.py",
        "scripts/olp-review-evidence.py",
        "octoscode olp-runtime-watch",
    ] {
        assert!(map.contains(tool), "the document map does not name {tool}");
    }

    // The init mode routes a multi-machine setup to the bring-up checklist.
    let init = &body[map_end..];
    let init = &init[..init.find("\n## 模式 outer").unwrap()];
    assert!(init.contains("docs/OLP_CLUSTER.md` §7"), "{init}");
    let cluster = std::fs::read_to_string(root.join("docs/OLP_CLUSTER.md")).unwrap();
    assert!(
        cluster.contains("\n## 7. 单元上线清单"),
        "the section the skill points at exists"
    );
}

/// Scenario: 安装的薄卡不含固定路径,仓库位置单独记录
#[test]
fn install_writes_a_path_free_entry_and_records_the_home() {
    let home = tempfile::tempdir().unwrap();
    let out = install(home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let entry_path = home.path().join(".claude/skills/octoloop/SKILL.md");
    let entry = std::fs::read_to_string(&entry_path).unwrap();
    let repo = canonical_repo();
    assert!(
        !entry.contains(&repo),
        "the entry must not embed the checkout path: {entry}"
    );
    assert!(
        !entry.contains("/Users/") && !entry.contains("/home/"),
        "{entry}"
    );
    assert!(entry.starts_with("---\nname: octoloop\n"), "{entry}");
    let body = std::fs::read_to_string(repo_root().join("skills/octoloop/AGENTS.md")).unwrap();
    assert_eq!(
        description(&entry),
        format!("{}。任意项目可用。", description(&body))
    );

    // The one place that holds a path is a one-line file anyone can edit.
    let recorded =
        std::fs::read_to_string(home.path().join(".octos/outer/octoscode-home")).unwrap();
    assert_eq!(recorded.trim_end(), repo);

    // Running it again changes nothing.
    let out = install(home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&entry_path).unwrap(), entry);

    // --print writes nothing; --home-only installs no entry.
    let other = tempfile::tempdir().unwrap();
    let out = install(other.path(), &["--print"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), entry);
    assert!(std::fs::read_dir(other.path()).unwrap().next().is_none());
    let out = install(other.path(), &["--home-only"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(other.path().join(".octos/outer/octoscode-home").exists());
    assert!(!other.path().join(".claude").exists());

    // --to serves an agent that reads its instructions from another place.
    let custom = other.path().join("agent-prompts/octoloop.md");
    let out = install(other.path(), &["--to", custom.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&custom).unwrap(), entry);
}

/// Scenario: 薄卡里的解析命令按 环境变量 → 记录文件 → 当前项目 的顺序找到仓库
#[test]
fn installed_entry_resolves_the_home_in_order() {
    let home = tempfile::tempdir().unwrap();
    assert!(install(home.path(), &[]).status.success());
    let entry =
        std::fs::read_to_string(home.path().join(".claude/skills/octoloop/SKILL.md")).unwrap();
    let command = entry
        .lines()
        .find(|l| l.starts_with("for h in "))
        .expect("the entry carries the resolution command");
    let resolve = |cwd: &Path, env_home: Option<&str>| -> Output {
        let mut cmd = Command::new("/bin/bash");
        cmd.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .env("HOME", home.path())
            .env_remove("OCTOSCODE_HOME");
        if let Some(h) = env_home {
            cmd.env("OCTOSCODE_HOME", h);
        }
        cmd.output().unwrap()
    };
    let elsewhere = tempfile::tempdir().unwrap();
    let repo = canonical_repo();

    // The recorded file answers from an unrelated directory.
    let out = resolve(elsewhere.path(), None);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), repo);

    // The environment variable wins — here a second "checkout" with a body.
    let moved = elsewhere.path().join("moved-octoscode");
    std::fs::create_dir_all(moved.join("skills/octoloop")).unwrap();
    std::fs::write(moved.join("skills/octoloop/AGENTS.md"), "x").unwrap();
    let out = resolve(elsewhere.path(), Some(moved.to_str().unwrap()));
    assert_eq!(stdout(&out).trim(), moved.to_str().unwrap());

    // A stale variable or a stale record is skipped, not trusted.
    let out = resolve(elsewhere.path(), Some("/nonexistent/octoscode"));
    assert_eq!(stdout(&out).trim(), repo, "falls through to the record");
    std::fs::write(
        home.path().join(".octos/outer/octoscode-home"),
        "/gone/octoscode\n",
    )
    .unwrap();
    let out = resolve(&repo_root(), None);
    assert_eq!(
        stdout(&out).trim(),
        repo,
        "falls through to the current project"
    );

    // Nothing found: no output and a failing status, so the agent stops.
    let out = resolve(elsewhere.path(), None);
    assert!(!out.status.success());
    assert_eq!(stdout(&out).trim(), "");
}

/// Scenario: 不覆盖别人写的文件,除非明确要求并留备份
#[test]
fn install_refuses_to_overwrite_a_foreign_file_without_force() {
    let home = tempfile::tempdir().unwrap();
    let entry_path = home.path().join(".claude/skills/octoloop/SKILL.md");
    std::fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
    let foreign = "---\nname: octoloop\n---\nOCTOSCODE_HOME = /old/place/octoscode\n";
    std::fs::write(&entry_path, foreign).unwrap();

    let out = install(home.path(), &[]);
    assert_eq!(out.status.code(), Some(3), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&entry_path).unwrap(), foreign);

    let out = install(home.path(), &["--force"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let backup = entry_path.with_file_name("SKILL.md.bak");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), foreign);
    assert!(
        !std::fs::read_to_string(&entry_path)
            .unwrap()
            .contains("/old/place")
    );

    // A file this script wrote is replaced without ceremony.
    std::fs::remove_file(&backup).unwrap();
    assert!(install(home.path(), &[]).status.success());
    assert!(!backup.exists());

    assert_eq!(install(home.path(), &["--bogus"]).status.code(), Some(2));
    assert_eq!(install(home.path(), &["--to"]).status.code(), Some(2));
}
