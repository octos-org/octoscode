#![cfg(unix)]

//! Contract tests for the olp/v3 cluster roles (SDD spec:
//! `specs/task-olp-v3-cluster-roles.spec.md`): the hub outer loop names the
//! implementer and reviewers per card, the unit's verification record can
//! name several reviewers, and the hub summarises progress for the
//! user's panorama.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn scripts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run(dir: &Path, program: &str, args: &[&str], stdin: &str) -> Output {
    let mut cmd = if program.ends_with(".py") {
        let mut c = Command::new("python3");
        c.arg("-B").arg(scripts().join(program));
        c
    } else {
        let mut c = Command::new("bash");
        c.arg(scripts().join(program));
        c
    };
    let mut child = cmd
        .args(args)
        .current_dir(dir)
        .env("OLP_UNITS_DIR", dir.join("units"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    child.wait_with_output().unwrap()
}

fn dispatch(dir: &Path, board: &Path, title: &str, roles: &[&str]) -> Output {
    let mut args = vec![
        "m07",
        "--board",
        board.to_str().unwrap(),
        "--title",
        title,
        "--skip-health",
        "--allow-busy",
        "--date",
        "2026-11-14",
    ];
    args.extend_from_slice(roles);
    run(dir, "olp-dispatch.py", &args, "正文。\n")
}

/// Scenario: 大外环按卡指定实现方与复验方
#[test]
fn v3_dispatch_names_the_implementer_and_reviewers_per_card() {
    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    std::fs::write(&board, "# 板\n").unwrap();

    let out = dispatch(
        dir.path(),
        &board,
        "加一个开关",
        &["--impl", "octoscode", "--review", "codex, claude"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(&board).unwrap();
    assert!(
        text.contains("### 1. 加一个开关(2026-11-14,外环(hub))\nunit: m07\nimpl: octoscode\nreview: codex, claude\n\n正文。\n"),
        "{text}"
    );

    // The same card with other reviewers is another dispatch, not a retry.
    let out = dispatch(
        dir.path(),
        &board,
        "加一个开关",
        &["--impl", "octoscode", "--review", "codex"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim().lines().last(), Some("2"));
    // A retry of the first is still recognised as the same card.
    let out = dispatch(
        dir.path(),
        &board,
        "加一个开关",
        &["--impl", "octoscode", "--review", "codex, claude"],
    );
    assert_eq!(
        stdout(&out).trim().lines().last(),
        Some("1"),
        "{}",
        stderr(&out)
    );
    // Without roles, a card is written as before.
    let out = dispatch(dir.path(), &board, "没有角色", &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        std::fs::read_to_string(&board)
            .unwrap()
            .contains("### 3. 没有角色(2026-11-14,外环(hub))\nunit: m07\n\n")
    );

    // The pane woken for a card with roles is the unit host: the wake says
    // so and points at the unit mode, instead of asking it to implement.
    let herdr = dir.path().join("herdr");
    std::fs::write(
        &herdr,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\n",
            dir.path().join("herdr.log").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&herdr, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = Command::new("python3")
        .arg("-B")
        .arg(scripts().join("olp-dispatch.py"))
        .args([
            "m07",
            "--board",
            board.to_str().unwrap(),
            "--title",
            "唤醒主持",
            "--skip-health",
            "--allow-busy",
            "--impl",
            "claude",
            "--review",
            "codex",
            "--wake",
            "w1:p1",
        ])
        .env("OLP_HERDR", &herdr)
        .env("OLP_UNITS_DIR", dir.path().join("units"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .unwrap()
                .write_all("正文。\n".as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let log = std::fs::read_to_string(dir.path().join("herdr.log")).unwrap();
    assert!(log.contains("--machine m07 agent prompt w1:p1 "), "{log}");
    assert!(
        log.contains("实现方 claude、复验方 codex")
            && log.contains("你是单元主持")
            && log.contains("模式 unit"),
        "{log}"
    );

    // The inbox shows the roles.
    let out = run(dir.path(), "olp-inbox.py", &[board.to_str().unwrap()], "");
    assert!(
        stdout(&out).contains("#1 [m07] (实现 octoscode,复验 codex、claude) 加一个开关"),
        "{}",
        stdout(&out)
    );
    let out = run(
        dir.path(),
        "olp-inbox.py",
        &[board.to_str().unwrap(), "--json"],
        "",
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["items"][0]["impl"], "octoscode");
    assert_eq!(
        v["items"][0]["review"],
        serde_json::json!(["codex", "claude"])
    );

    // Bad role arguments never reach the board.
    let before = std::fs::read_to_string(&board).unwrap();
    for roles in [
        vec!["--impl", "octoscode"],
        vec!["--impl", "octoscode", "--review", "codex,codex"],
        vec!["--impl", "Octo Scode", "--review", "codex"],
        vec!["--review", "codex;rm"],
    ] {
        let out = dispatch(dir.path(), &board, "坏角色", &roles);
        assert_eq!(out.status.code(), Some(2), "{roles:?}: {}", stderr(&out));
    }
    assert_eq!(std::fs::read_to_string(&board).unwrap(), before);
}

/// Scenario: 复验记录可以写出多个复验方,归档后角色行留在桩里
#[test]
fn v3_verification_record_names_several_reviewers_and_archive_keeps_roles() {
    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let board = d.join("board.md");
    std::fs::write(&board, "# 板\n").unwrap();
    let out = dispatch(
        d,
        &board,
        "一张卡",
        &["--impl", "claude", "--review", "codex,octoscode"],
    );
    assert!(out.status.success(), "{}", stderr(&out));

    let git = |dir: &Path, args: &[&str]| -> Output {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
    };
    let hub = d.join("hub.git");
    let seed = d.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    assert!(git(&seed, &["init", "-q", "-b", "main"]).status.success());
    std::fs::write(seed.join("README.md"), "x\n").unwrap();
    assert!(git(&seed, &["add", "."]).status.success());
    assert!(git(&seed, &["commit", "-qm", "init"]).status.success());
    let out = run(
        d,
        "olp-git-hub.py",
        &[
            "init",
            hub.to_str().unwrap(),
            "--board",
            board.to_str().unwrap(),
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        git(&seed, &["push", "-q", hub.to_str().unwrap(), "main"])
            .status
            .success()
    );
    assert!(
        git(
            d,
            &["clone", "-q", "-b", "main", hub.to_str().unwrap(), "work"]
        )
        .status
        .success()
    );
    let work = d.join("work");
    let out = Command::new("python3")
        .arg("-B")
        .arg(scripts().join("olp-git-hub.py"))
        .args([
            "connect-local",
            "--unit",
            "m07",
            "--repo",
            hub.to_str().unwrap(),
        ])
        .current_dir(&work)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        git(&work, &["checkout", "-q", "-b", "task/1"])
            .status
            .success()
    );
    std::fs::write(work.join("README.md"), "y\n").unwrap();
    assert!(git(&work, &["commit", "-qam", "task 1"]).status.success());

    let out = Command::new("bash")
        .arg(scripts().join("olp-task-push.sh"))
        .args(["1", "--verified-by", "m07-codex+octoscode"])
        .current_dir(&work)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(&board).unwrap();
    assert!(
        text.contains("> 单元(m07)·本地复验通过(#1): task/1 @ ")
            && text.contains(" 复验 m07-codex+octoscode"),
        "{text}"
    );

    // A clone whose hub remote is the bare repo's path (no connect-local)
    // bypasses serve. The hub refuses such a push when it carries a
    // verification record, instead of accepting it unrecorded.
    assert!(
        git(
            d,
            &["clone", "-q", "-b", "main", hub.to_str().unwrap(), "raw"]
        )
        .status
        .success()
    );
    let raw = d.join("raw");
    assert!(
        git(&raw, &["remote", "add", "hub", hub.to_str().unwrap()])
            .status
            .success()
    );
    assert!(git(&raw, &["fetch", "-q", "hub"]).status.success());
    assert!(
        git(&raw, &["checkout", "-q", "-b", "task/1", "hub/task/1"])
            .status
            .success()
    );
    std::fs::write(raw.join("README.md"), "z\n").unwrap();
    assert!(
        git(&raw, &["commit", "-qam", "task 1 again"])
            .status
            .success()
    );
    let out = Command::new("bash")
        .arg(scripts().join("olp-task-push.sh"))
        .args(["1", "--verified-by", "m07-codex"])
        .current_dir(&raw)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("connect-local"), "{}", stderr(&out));
    // The hub's own pushes (no record) still go through.
    assert!(
        git(&raw, &["push", "-q", "hub", "HEAD:refs/heads/hub-own"])
            .status
            .success()
    );

    // Accept, archive: the stub keeps the role lines.
    let out = run(
        d,
        "olp-board-append.sh",
        &[board.to_str().unwrap()],
        "> 外环(hub)·采认(#1): 手续齐全\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let out = run(
        d,
        "olp-board-archive.py",
        &[board.to_str().unwrap(), "--keep", "0"],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let stub = std::fs::read_to_string(&board).unwrap();
    assert!(
        stub.contains("unit: m07\nimpl: claude\nreview: codex, octoscode\n"),
        "{stub}"
    );
    assert!(!stub.contains("正文。"), "{stub}");
}

/// Scenario: 大外环汇总项目进展,交给用户指定的全景台
#[test]
fn v3_summary_collects_cards_units_prs_and_alerts() {
    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let board = d.join("board.md");
    std::fs::write(&board, "# 板\n").unwrap();
    let out = dispatch(
        d,
        &board,
        "第一张",
        &["--impl", "octoscode", "--review", "codex"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let out = dispatch(d, &board, "第二张", &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let out = run(
        d,
        "olp-board-append.sh",
        &[board.to_str().unwrap()],
        "> 单元(m07)·负哨(#1): stalled_no_progress turn=- age=900s activity=no_events\n> 外环(hub)·采认(#2): 手续齐全\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let out = run(
        d,
        "olp-unit.py",
        &[
            "register",
            "m07",
            "--hostname",
            "m07",
            "--mac",
            "aa:bb:cc:dd:ee:07",
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));

    let out = run(
        d,
        "olp-summary.py",
        &[
            "--board",
            board.to_str().unwrap(),
            "--name",
            "demo",
            "--json",
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let s: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(s["project"], "demo");
    assert_eq!(s["counts"]["working"], 1, "{s}");
    assert_eq!(s["counts"]["settled"], 1, "{s}");
    let first = &s["cards"][0];
    assert_eq!(first["number"], 1);
    assert_eq!(first["impl"], "octoscode");
    assert_eq!(first["review"], serde_json::json!(["codex"]));
    assert_eq!(s["units"][0]["unit"], "m07");
    assert_eq!(s["units"][0]["state"], "stale");
    assert_eq!(s["alerts"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["alerts"][0]["card"], 1);
    assert!(s.get("local_prs").is_none(), "no --repo, no local PRs");

    // Text for a person: open cards with roles, units, alerts; settled cards
    // are left out.
    let out = run(
        d,
        "olp-summary.py",
        &["--board", board.to_str().unwrap(), "--name", "demo"],
        "",
    );
    let text = stdout(&out);
    assert!(text.contains("项目 demo"), "{text}");
    assert!(
        text.contains("#1 [m07] 实现 octoscode / 复验 codex 第一张"),
        "{text}"
    );
    assert!(!text.contains("第二张"), "{text}");
    assert!(text.contains("m07 失联"), "{text}");
    assert!(text.contains("负哨 #1 [m07] stalled_no_progress"), "{text}");

    // A repository that cannot be read is reported, the rest still comes out.
    let out = run(
        d,
        "olp-summary.py",
        &[
            "--board",
            board.to_str().unwrap(),
            "--repo",
            "/nonexistent/hub.git",
            "--json",
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let s: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(s["local_prs"]["unavailable"].is_string(), "{s}");
    assert_eq!(s["counts"]["working"], 1);

    let out = run(
        d,
        "olp-summary.py",
        &["--board", "/nonexistent/board.md"],
        "",
    );
    assert_eq!(out.status.code(), Some(2));
}
