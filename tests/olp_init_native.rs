#![cfg(unix)]

//! Contract tests for `octoscode olp init` and `olp skill install` in Rust
//! (SDD spec: `specs/task-olp-init-rust.spec.md`): the same scaffold and
//! the same thin card as the scripts they replace.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn olp(home: &Path, cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    Command::new(BIN)
        .arg("olp")
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("OCTOSCODE_OLP_BUNDLE", home.join("bundle"))
        .envs(envs.iter().copied())
        .output()
        .unwrap()
}

/// Scenario: Rust 版 init 铺设脚手架,重跑不改动任何已有内容
#[test]
fn native_init_lays_the_scaffold_once() {
    let dir = tempfile::tempdir().unwrap();
    let (home, project) = (dir.path().join("home"), dir.path().join("proj"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&project)
            .status()
            .unwrap()
            .success()
    );
    // A .gitignore whose last line has no newline: nothing may glue onto it.
    std::fs::write(project.join(".gitignore"), "target").unwrap();

    let out = olp(
        &home,
        dir.path(),
        &["init", "proj"],
        &[("OLP_INIT_LANG", "zh")],
    );
    assert!(
        matches!(out.status.code(), Some(0) | Some(2)),
        "0, or 2 when octoscode is not on PATH: {}{}",
        stdout(&out),
        stderr(&out)
    );
    let read = |rel: &str| std::fs::read_to_string(project.join(rel)).unwrap();
    assert!(read(".octos/loop.md").contains("维护循环"));
    assert!(read(".octos/OUTER_LOOP_REVIEW.md").ends_with("\nACK:\n"));
    assert_eq!(
        read("AGENTS.md"),
        std::fs::read_to_string(repo_root().join("docs/OCTOLOOP_AGENTS.zh-CN.md")).unwrap()
    );
    let ignore = read(".gitignore");
    assert!(ignore.starts_with("target\n"), "{ignore}");
    for line in [
        ".octos/OUTER_LOOP_REVIEW.md",
        ".octos/EVOLUTION.md",
        ".octos/UNIT_BOARD.md",
    ] {
        assert_eq!(
            ignore.lines().filter(|l| l.trim() == line).count(),
            1,
            "{line} once, on its own line:\n{ignore}"
        );
    }
    for helper in ["watch-board.sh", "board-append.sh"] {
        let path = home.join(".octos/outer").join(helper);
        let source = repo_root().join("scripts").join(format!("olp-{helper}"));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            std::fs::read(&source).unwrap(),
            "{helper} is the repository's script"
        );
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(path.metadata().unwrap().permissions().mode() & 0o111, 0);
    }

    // Again, after the user changed things: nothing is overwritten or added.
    std::fs::write(project.join("AGENTS.md"), "我自己的卡\n").unwrap();
    std::fs::write(project.join(".octos/loop.md"), "我自己的循环\n").unwrap();
    let before = read(".gitignore");
    let out = olp(&home, &project, &["init"], &[]);
    assert!(matches!(out.status.code(), Some(0) | Some(2)));
    assert_eq!(read("AGENTS.md"), "我自己的卡\n");
    assert_eq!(read(".octos/loop.md"), "我自己的循环\n");
    assert_eq!(read(".gitignore"), before);
    assert!(stdout(&out).contains("已存在,跳过"), "{}", stdout(&out));

    let out = olp(&home, dir.path(), &["init", "no-such-dir"], &[]);
    assert_eq!(out.status.code(), Some(2));
}

/// Scenario: Rust 版 skill install 的薄卡与脚本生成的一字不差
#[test]
fn native_skill_card_is_the_scripts_card() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let native = olp(&home, dir.path(), &["skill", "install", "--print"], &[]);
    assert!(native.status.success(), "{}", stderr(&native));
    let script = Command::new("/bin/bash")
        .arg(repo_root().join("scripts/olp-install-skill.sh"))
        .arg("--print")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(script.status.success(), "{}", stderr(&script));
    assert_eq!(stdout(&native), stdout(&script));
    assert!(!home.join(".octos").exists(), "--print writes nothing");

    // Installed to a file an agent reads; a foreign file is not overwritten.
    let target = dir.path().join("agent/SKILL.md");
    let to = target.to_str().unwrap();
    let out = olp(&home, dir.path(), &["skill", "install", "--to", to], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), stdout(&native));
    let recorded = std::fs::read_to_string(home.join(".octos/outer/octoscode-home")).unwrap();
    assert!(recorded.trim().ends_with("/current"), "{recorded}");
    assert!(
        Path::new(recorded.trim())
            .join("skills/octoloop/AGENTS.md")
            .is_file()
    );

    std::fs::write(&target, "别人写的文件\n").unwrap();
    let out = olp(&home, dir.path(), &["skill", "install", "--to", to], &[]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "别人写的文件\n");
    let out = olp(
        &home,
        dir.path(),
        &["skill", "install", "--to", to, "--force"],
        &[],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(format!("{to}.bak")).unwrap(),
        "别人写的文件\n"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), stdout(&native));
}
