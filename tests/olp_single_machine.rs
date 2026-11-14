#![cfg(unix)]

//! Contract tests for the single-machine deployment (SDD spec:
//! `specs/task-olp-single-machine.spec.md`): hub and unit on one machine, the
//! cluster tools working without ssh. Every hub-side rule still applies; only
//! the transport changes. `OLP_SSH` points at a program that records any call
//! and fails, so a test also proves that ssh was never used.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
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

fn write_executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// One machine that is both the hub and unit m01.
struct Box1 {
    dir: tempfile::TempDir,
}

impl Box1 {
    fn new() -> Self {
        let b = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for sub in [
            "outer/mcp/questions",
            "outer/mcp/answers",
            "outer/mcp/consumed",
            "proj/.octos",
        ] {
            std::fs::create_dir_all(b.p(sub)).unwrap();
        }
        std::fs::write(b.board(), "# 板\n\n### 1. 一张卡\nunit: m01\n\n正文。\n").unwrap();
        write_executable(
            &b.p("ssh"),
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\nexit 255\n",
                b.p("ssh.log").display()
            ),
        );
        write_executable(
            &b.p("herdr"),
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\n",
                b.p("herdr.log").display()
            ),
        );
        std::fs::write(
            b.p("outer/unit.conf"),
            format!(
                "hub = local\nunit = m01\nboard = {}\nmirror = {}\n",
                b.board().display(),
                b.board().display()
            ),
        )
        .unwrap();
        b
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn board(&self) -> PathBuf {
        self.p("proj/.octos/OUTER_LOOP_REVIEW.md")
    }

    fn board_text(&self) -> String {
        std::fs::read_to_string(self.board()).unwrap()
    }

    fn ssh_calls(&self) -> String {
        std::fs::read_to_string(self.p("ssh.log")).unwrap_or_default()
    }

    fn cmd(&self, program: &str) -> Command {
        let mut cmd = if program.ends_with(".py") {
            let mut c = Command::new("python3");
            c.arg("-B").arg(scripts().join(program));
            c
        } else {
            let mut c = Command::new("bash");
            c.arg(scripts().join(program));
            c
        };
        cmd.env("OLP_SSH", self.p("ssh"))
            .env("OLP_HERDR", self.p("herdr"))
            .env("OLP_UNITS_DIR", self.p("outer/units"))
            .env("OLP_UNIT_CONF", self.p("outer/unit.conf"))
            .env("OLP_QUEUE_DIR", self.p("outer/queue"))
            .env("OLP_MCP_OUTER_ROOT", self.p("outer"))
            .env_remove("OLP_HUB")
            .env_remove("OLP_UNIT_ID")
            .env_remove("OLP_BOARD")
            .env_remove("OLP_MIRROR");
        cmd
    }

    fn run(&self, program: &str, args: &[&str], stdin: Option<&str>) -> Output {
        let mut child = self
            .cmd(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.unwrap_or("").as_bytes());
        child.wait_with_output().unwrap()
    }

    /// Register m01 with this machine's hostname: that is what makes it local.
    fn register_here(&self) {
        let out = self.run(
            "olp-unit.py",
            &["heartbeat", "--unit", "m01", "--require", "disk"],
            None,
        );
        assert!(out.status.success(), "{}", stderr(&out));
        let beat: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        let host = beat["hostname"].as_str().unwrap().to_string();
        let out = self.run(
            "olp-unit.py",
            &[
                "register",
                "m01",
                "--hostname",
                &host,
                "--mac",
                "aa:bb:cc:dd:ee:01",
            ],
            None,
        );
        assert!(out.status.success(), "{}", stderr(&out));
        let out = self.run(
            "olp-unit.py",
            &[
                "register",
                "m02",
                "--hostname",
                "elsewhere",
                "--mac",
                "aa:bb:cc:dd:ee:02",
            ],
            None,
        );
        assert!(out.status.success(), "{}", stderr(&out));
    }
}

/// Scenario: hub = local 时写板、ACK、镜像都在本机完成,规则照旧
#[test]
fn single_machine_board_writes_go_through_the_hub_rules_without_ssh() {
    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed");
        return;
    }
    let b = Box1::new();

    let out = b.run(
        "olp-board-remote.py",
        &["ack", "1", "done", "commit abc"],
        None,
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let out = b.run(
        "olp-board-remote.py",
        &["append", "--sign"],
        Some("一条进度\n"),
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let board = b.board_text();
    assert!(
        board.contains("> 内环(m01)·#1 ACK(done): commit abc"),
        "{board}"
    );
    assert!(board.contains("> 单元(m01)·一条进度"), "{board}");

    // The hub's rules still hold: no outer-loop verdicts, no numbered cards.
    let out = b.run(
        "olp-board-remote.py",
        &["append"],
        Some("> 外环(hub)·采认(#1): 伪造\n"),
    );
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let out = b.run(
        "olp-board-remote.py",
        &["append"],
        Some("### 9. 自己立卡\n"),
    );
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(!b.board_text().contains("伪造"));

    // The mirror path is the board itself: nothing to do, and the board must
    // not become read-only.
    let out = b.run("olp-board-remote.py", &["mirror"], None);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("不需要同步"), "{}", stdout(&out));
    let mode = std::fs::metadata(b.board()).unwrap().permissions().mode() & 0o777;
    assert_ne!(mode & 0o200, 0, "the board stays writable");
    // A separate mirror still works.
    let copy = b.p("copy.md");
    let out = b.run(
        "olp-board-remote.py",
        &["mirror", "--to", copy.to_str().unwrap()],
        None,
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), b.board_text());

    // hub = local needs the board's absolute path.
    std::fs::write(b.p("outer/unit.conf"), "hub = local\nunit = m01\n").unwrap();
    let out = b.run("olp-board-remote.py", &["ack", "1", "done", "x"], None);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("board="), "{}", stderr(&out));

    assert_eq!(b.ssh_calls(), "", "ssh was never used");
}

/// Scenario: 本机的推送经同一个 serve,推送规则照旧
#[test]
fn single_machine_pushes_go_through_serve_with_the_unit_id() {
    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed");
        return;
    }
    let b = Box1::new();
    let hub = b.p("hub.git");
    let git = |dir: &Path, args: &[&str]| -> Output {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("OLP_UNITS_DIR", b.p("outer/units"))
            .output()
            .unwrap()
    };
    // The hub's own repository and its bare copy.
    let seed = b.p("seed");
    std::fs::create_dir_all(&seed).unwrap();
    assert!(git(&seed, &["init", "-q", "-b", "main"]).status.success());
    std::fs::write(seed.join("README.md"), "x\n").unwrap();
    assert!(git(&seed, &["add", "."]).status.success());
    assert!(git(&seed, &["commit", "-qm", "init"]).status.success());
    let out = b.run(
        "olp-git-hub.py",
        &[
            "init",
            hub.to_str().unwrap(),
            "--board",
            b.board().to_str().unwrap(),
        ],
        None,
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        git(&seed, &["push", "-q", hub.to_str().unwrap(), "main"])
            .status
            .success()
    );

    // The unit's working clone, connected locally.
    let work = b.p("work");
    assert!(
        git(
            b.dir.path(),
            &["clone", "-q", hub.to_str().unwrap(), "work"]
        )
        .status
        .success()
    );
    let out = b
        .cmd("olp-git-hub.py")
        .args([
            "connect-local",
            "--unit",
            "m01",
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

    // Without a local verification record, or to another branch: refused.
    let out = git(&work, &["push", "hub", "task/1"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("缺少本地复验记录"),
        "{}",
        stderr(&out)
    );
    let out = git(&work, &["push", "hub", "task/1:main"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("单元只能推"), "{}", stderr(&out));

    // The verifier's push is accepted and recorded on the board with the
    // unit id the remote was connected with.
    let out = Command::new("bash")
        .arg(scripts().join("olp-task-push.sh"))
        .args(["1", "--verified-by", "m01-ver"])
        .current_dir(&work)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        b.board_text()
            .contains("> 单元(m01)·本地复验通过(#1): task/1 @"),
        "{}",
        b.board_text()
    );

    // Fetching goes the same way.
    assert!(git(&work, &["fetch", "-q", "hub"]).status.success());
    // connect-local refuses a repository that does not exist.
    let out = b
        .cmd("olp-git-hub.py")
        .args([
            "connect-local",
            "--unit",
            "m01",
            "--repo",
            "/nonexistent/hub.git",
        ])
        .current_dir(&work)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// Scenario: hub 回答本机单元的提问、唤醒本机的窗格,不经 ssh
#[test]
fn single_machine_answers_and_wakes_reach_the_local_unit() {
    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed");
        return;
    }
    let b = Box1::new();
    b.register_here();
    let out = b.run("olp-unit.py", &["where", "m01"], None);
    assert_eq!(stdout(&out).trim(), "local", "{}", stderr(&out));
    let out = b.run("olp-unit.py", &["where", "m02"], None);
    assert_eq!(stdout(&out).trim(), "remote");
    let out = b.run("olp-unit.py", &["where", "m09"], None);
    assert_eq!(out.status.code(), Some(2), "unregistered");

    // A waiting question in the unit's mailbox (as the MCP server writes it).
    std::fs::write(
        b.p("outer/mcp/questions/20261114T000000-abcd1234.json"),
        r#"{"id":"abcd1234","ts":"t","question":"用中文还是英文?","context":"c","tried":"t"}"#,
    )
    .unwrap();
    let out = b.run("olp-answer.py", &["list", "m01"], None);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("abcd1234\t等待中"),
        "{}",
        stdout(&out)
    );
    let out = b.run(
        "olp-answer.py",
        &["answer", "m01", "abcd1234"],
        Some("英文"),
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let answer = std::fs::read_to_string(b.p("outer/mcp/answers/abcd1234.json")).unwrap();
    assert!(answer.contains("英文"), "{answer}");
    assert_eq!(b.ssh_calls(), "", "the local unit is answered without ssh");

    // A unit on another machine still goes over ssh.
    let out = b.run("olp-answer.py", &["list", "m02"], None);
    assert_eq!(out.status.code(), Some(75), "{}", stderr(&out));
    assert!(b.ssh_calls().contains("m02 list"), "{}", b.ssh_calls());

    // Dispatch wakes the local unit's pane without --machine; a remote unit's
    // with it.
    for (unit, pane) in [("m01", "w1:p1"), ("m02", "w2:p1")] {
        let out = b.run(
            "olp-dispatch.py",
            &[
                unit,
                "--board",
                b.board().to_str().unwrap(),
                "--title",
                &format!("卡给 {unit}"),
                "--skip-health",
                "--allow-busy",
                "--wake",
                pane,
            ],
            Some("正文\n"),
        );
        assert!(out.status.success(), "{unit}: {}", stderr(&out));
    }
    let log = std::fs::read_to_string(b.p("herdr.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert!(lines[0].starts_with("agent prompt w1:p1 "), "{log}");
    assert!(
        lines[1].starts_with("--machine m02 agent prompt w2:p1 "),
        "{log}"
    );
}

/// Scenario: 心跳任务在单机部署时直接交给本机的 receive
#[test]
fn single_machine_heartbeat_job_hands_the_beat_to_the_local_hub() {
    let b = Box1::new();
    for (tool, file) in [
        ("olp-launchd.py", "org.octos.olp.heartbeat.plist"),
        ("olp-systemd.py", "olp-heartbeat.service"),
    ] {
        let dir = b.p(&format!("jobs-{tool}"));
        let out = b
            .cmd(tool)
            .args([
                "heartbeat",
                "--unit",
                "m01",
                "--hub",
                "local",
                "--dir",
                dir.to_str().unwrap(),
                "--no-load",
            ])
            .env("HOME", b.dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        let job = std::fs::read_to_string(dir.join(file)).unwrap();
        assert!(
            job.contains("olp-unit.py heartbeat --unit m01 | python3 -B")
                && job.contains("olp-unit.py receive --unit m01"),
            "{tool}: {job}"
        );
        assert!(!job.contains("ssh "), "{tool}: {job}");
    }
}
