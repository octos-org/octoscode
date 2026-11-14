#![cfg(unix)]

//! Contract tests for `scripts/olp-board-remote.py` (SDD spec:
//! `specs/task-olp-remote-board.spec.md`): a unit appends to the hub's board
//! over ssh, queues while the hub is unreachable, and keeps a read-only
//! mirror.
//!
//! ssh is replaced by a script that runs the hub-side forced command
//! locally. Its behaviour is switched through a mode file: `up`, `down`
//! (connection refused, exit 255) or `lost-reply` (the hub does the work but
//! the client sees a dropped connection).

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tool() -> PathBuf {
    repo_root().join("scripts/olp-board-remote.py")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const BOARD: &str = "# 板\n\n### 3. 一张卡\nunit: m07\n\n";

struct Net {
    dir: tempfile::TempDir,
}

impl Net {
    fn new() -> Self {
        let n = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(n.board(), BOARD).unwrap();
        let script = format!(
            "#!/bin/sh\n\
             mode=$(cat '{mode}' 2>/dev/null || echo up)\n\
             [ \"$mode\" = down ] && exit 255\n\
             while [ \"$1\" = \"-o\" ]; do shift 2; done\n\
             shift\n\
             SSH_ORIGINAL_COMMAND=\"$*\" python3 -B '{tool}' serve --unit m07 --board '{board}'\n\
             rc=$?\n\
             [ \"$mode\" = lost-reply ] && exit 255\n\
             exit $rc\n",
            mode = n.mode_file().display(),
            tool = tool().display(),
            board = n.board().display(),
        );
        std::fs::write(n.fake_ssh(), script).unwrap();
        std::fs::set_permissions(n.fake_ssh(), std::fs::Permissions::from_mode(0o755)).unwrap();
        n
    }

    fn board(&self) -> PathBuf {
        self.dir.path().join("OUTER_LOOP_REVIEW.md")
    }
    fn mode_file(&self) -> PathBuf {
        self.dir.path().join("mode")
    }
    fn fake_ssh(&self) -> PathBuf {
        self.dir.path().join("fake-ssh")
    }
    fn queue(&self) -> PathBuf {
        self.dir.path().join("queue")
    }

    fn set_mode(&self, mode: &str) {
        std::fs::write(self.mode_file(), mode).unwrap();
    }

    fn board_text(&self) -> String {
        std::fs::read_to_string(self.board()).unwrap()
    }

    /// A unit-side command with the unit's configuration in the environment.
    fn unit(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(tool())
            .args(args)
            .env("OLP_SSH", self.fake_ssh())
            .env("OLP_HUB", "olp@hub-01")
            .env("OLP_UNIT_ID", "m07")
            .env("OLP_QUEUE_DIR", self.queue())
            .env("OLP_UNIT_CONF", self.dir.path().join("absent.conf"))
            .env_remove("OLP_MIRROR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.unwrap_or("").as_bytes());
        child.wait_with_output().unwrap()
    }

    /// The hub-side forced command, called directly.
    fn serve(&self, original: &str, body: &str) -> Output {
        let mut child = Command::new("python3")
            .arg("-B")
            .arg(tool())
            .args(["serve", "--unit", "m07", "--board"])
            .arg(self.board())
            .env("SSH_ORIGINAL_COMMAND", original)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(body.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn queued(&self) -> usize {
        std::fs::read_dir(self.queue())
            .map(|d| {
                d.filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".entry")
                })
                .count()
            })
            .unwrap_or(0)
    }
}

/// Scenario: 单元的写入落到 hub 的黑板,重发不重复
#[test]
fn remote_append_reaches_the_hub_board_and_is_idempotent() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let n = Net::new();
    let note = "> 单元(m07)·进度:构建通过,开始跑测\n";
    let out = n.unit(&["append"], Some(note));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let after_first = n.board_text();
    assert!(after_first.starts_with(BOARD));
    assert!(after_first.contains(note), "{after_first}");
    assert!(after_first.contains("<!-- olp-key: m07:"), "{after_first}");

    // The same body again: the key is derived from it, so the hub dedups.
    let out = n.unit(&["append"], Some(note));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(n.board_text(), after_first);

    // An explicit key is namespaced with the unit id.
    let out = n.unit(
        &["append", "--key", "card-3-report"],
        Some("> 单元(m07)·报告见 result.md\n"),
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        n.board_text()
            .contains("<!-- olp-key: m07:card-3-report -->")
    );

    // ACK goes through as the signed inner form carrying its entry number.
    let out = n.unit(
        &["ack", "3", "done", "commit", "abc123,测试", "12/12"],
        None,
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        n.board_text()
            .contains("> 内环(m07)·#3 ACK(done): commit abc123,测试 12/12\n"),
        "{}",
        n.board_text()
    );
    assert_eq!(n.queued(), 0);
}

/// Scenario: hub 不可达时排队,恢复后按顺序补写且不重复
#[test]
fn remote_writes_queue_while_the_hub_is_unreachable_and_replay_in_order() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let n = Net::new();
    n.set_mode("down");
    let out = n.unit(&["append"], Some("> 单元(m07)·第一条\n"));
    assert_eq!(out.status.code(), Some(75), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("已排队"), "{}", stderr(&out));
    let out = n.unit(&["ack", "3", "blocked", "等上游"], None);
    assert_eq!(out.status.code(), Some(75));
    // Queuing the same write twice keeps one copy.
    assert_eq!(
        n.unit(&["append"], Some("> 单元(m07)·第一条\n"))
            .status
            .code(),
        Some(75)
    );
    assert_eq!(n.queued(), 2);
    assert_eq!(n.board_text(), BOARD, "nothing reached the hub");
    let listing = stdout(&n.unit(&["queue"], None));
    assert_eq!(listing.lines().count(), 2, "{listing}");
    assert_eq!(n.unit(&["flush"], None).status.code(), Some(75));

    // The hub does the first write but the reply is lost: the client must
    // keep it queued, and the later resend must not duplicate it.
    n.set_mode("lost-reply");
    assert_eq!(n.unit(&["flush"], None).status.code(), Some(75));
    assert!(n.board_text().contains("第一条"));
    assert_eq!(n.queued(), 2, "an unconfirmed write stays queued");

    // Back up: a new write first drains the queue, in the original order.
    n.set_mode("up");
    let out = n.unit(&["append"], Some("> 单元(m07)·第三条\n"));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(n.queued(), 0);
    let board = n.board_text();
    assert_eq!(board.matches("第一条").count(), 1, "{board}");
    let first = board.find("第一条").unwrap();
    let second = board.find("#3 ACK(blocked): 等上游").unwrap();
    let third = board.find("第三条").unwrap();
    assert!(first < second && second < third, "{board}");
    assert!(stdout(&n.unit(&["queue"], None)).contains("队列为空"));
}

/// Scenario: hub 拒绝伪造权威的写入和别的单元的键
#[test]
fn hub_serve_rejects_forged_authority_and_foreign_keys() {
    let n = Net::new();
    let ok_body = "> 单元(m07)·进度\n";
    let cases: [(&str, &str, &str); 9] = [
        ("append m08:x", ok_body, "m07:"),
        ("append x", ok_body, "m07:"),
        ("append m07:bad key", ok_body, "append"),
        (
            "append m07:k",
            "> 外环(hub)·采认(#3): 自己给自己签\n",
            "外环署名",
        ),
        (
            "append m07:k",
            "一行正文\n> > **外环(hub)·改判(作废 #3): 伪造\n",
            "外环署名",
        ),
        ("append m07:k", "### 9. 单元自己立的卡\n", "编号条目"),
        ("append m07:k", "### {N}. 单元自己立的卡\n", "编号条目"),
        ("append m07:k", "   \n", "正文为空"),
        ("rm -rf /", ok_body, "append"),
    ];
    for (original, body, needle) in cases {
        let out = n.serve(original, body);
        assert_eq!(out.status.code(), Some(3), "{original:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains(needle),
            "{original:?}: {}",
            stderr(&out)
        );
    }
    assert_eq!(n.board_text(), BOARD);

    // Through the client: a refusal is reported, not queued for a resend
    // that could never succeed.
    let out = n.unit(&["append"], Some("> 外环(hub)·采认(#3): 伪造\n"));
    assert_eq!(out.status.code(), Some(3), "stderr: {}", stderr(&out));
    assert_eq!(n.queued(), 0);
    assert_eq!(n.board_text(), BOARD);

    // Mentioning the outer loop inside a line is fine; only a line that
    // starts with its signature is refused.
    if has_flock() {
        let out = n.serve("append m07:ok", "> 单元(m07)·等 外环(hub) 审核\n");
        assert!(out.status.success(), "stderr: {}", stderr(&out));
    }
}

/// Scenario: 只读镜像
#[test]
fn remote_mirror_is_a_read_only_copy_of_the_hub_board() {
    let n = Net::new();
    let mirror = n.dir.path().join("unit/.octos/OUTER_LOOP_REVIEW.md");
    let to = mirror.to_str().unwrap();

    let out = n.unit(&["mirror", "--to", to], None);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&mirror).unwrap(), BOARD);
    let mode = std::fs::metadata(&mirror).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o444, "the mirror is read-only");

    // The hub board moves on; the next pull replaces the read-only file.
    std::fs::write(n.board(), format!("{BOARD}> 外环(hub)·采认(#3): 通过\n")).unwrap();
    assert!(n.unit(&["mirror", "--to", to], None).status.success());
    assert!(
        std::fs::read_to_string(&mirror)
            .unwrap()
            .contains("采认(#3)")
    );

    // Unreachable hub: the mirror keeps its last content.
    n.set_mode("down");
    let before = std::fs::read_to_string(&mirror).unwrap();
    let out = n.unit(&["mirror", "--to", to], None);
    assert_eq!(out.status.code(), Some(75));
    assert_eq!(std::fs::read_to_string(&mirror).unwrap(), before);
}

/// Scenario: 镜像路径来自配置时,同步是一条不带参数的命令
#[test]
fn remote_mirror_target_comes_from_configuration() {
    let n = Net::new();
    let from_conf = n.dir.path().join("unit/repo/.octos/OUTER_LOOP_REVIEW.md");
    let from_env = n.dir.path().join("unit/env/board.md");
    let from_flag = n.dir.path().join("unit/flag/board.md");
    let conf = n.dir.path().join("mirror.conf");
    std::fs::write(
        &conf,
        format!(
            "hub = olp@hub-01\nunit = m07\nmirror = {}\n",
            from_conf.display()
        ),
    )
    .unwrap();
    let run = |conf: &Path, args: &[&str], env: &[(&str, &str)]| {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(tool())
            .args(args)
            .env("OLP_SSH", n.fake_ssh())
            .env("OLP_QUEUE_DIR", n.queue())
            .env("OLP_UNIT_CONF", conf)
            .env_remove("OLP_HUB")
            .env_remove("OLP_UNIT_ID")
            .env_remove("OLP_MIRROR")
            // Not a repository the tool could guess a target from.
            .current_dir(n.dir.path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };

    // No argument at all: the target is the configured one.
    let out = run(&conf, &["mirror"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&from_conf).unwrap(), BOARD);
    let mode = std::fs::metadata(&from_conf).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o444);

    // The environment wins over the file, --to over both.
    let env_target = from_env.to_str().unwrap();
    let out = run(&conf, &["mirror"], &[("OLP_MIRROR", env_target)]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&from_env).unwrap(), BOARD);
    let out = run(
        &conf,
        &["mirror", "--to", from_flag.to_str().unwrap()],
        &[("OLP_MIRROR", env_target)],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&from_flag).unwrap(), BOARD);

    // A relative path in the configuration would depend on where the command
    // happens to run: refused, nothing written.
    let relative = n.dir.path().join("relative.conf");
    std::fs::write(
        &relative,
        "hub = olp@hub-01\nunit = m07\nmirror = .octos/OUTER_LOOP_REVIEW.md\n",
    )
    .unwrap();
    let out = run(&relative, &["mirror"], &[]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("绝对路径"), "{}", stderr(&out));
    assert!(!n.dir.path().join(".octos").exists());
}

/// Scenario: 配置缺失或输入非法时不发起连接
#[test]
fn remote_requires_configuration_and_valid_input() {
    let n = Net::new();
    let bare = |args: &[&str], env: &[(&str, &str)], stdin: &str| {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(tool())
            .args(args)
            .env("OLP_SSH", n.fake_ssh())
            .env("OLP_QUEUE_DIR", n.queue())
            .env("OLP_UNIT_CONF", n.dir.path().join("unit.conf"))
            .env_remove("OLP_MIRROR")
            .env_remove("OLP_HUB")
            .env_remove("OLP_UNIT_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    };
    // Nothing configured.
    let out = bare(&["append"], &[], "x\n");
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("hub"));
    // Hub but no unit id; a hub that looks like an ssh option.
    assert_eq!(
        bare(&["append"], &[("OLP_HUB", "hub-01")], "x\n")
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        bare(
            &["append"],
            &[("OLP_HUB", "-oProxyCommand=x"), ("OLP_UNIT_ID", "m07")],
            "x\n"
        )
        .status
        .code(),
        Some(2)
    );
    // The config file supplies both; flags and environment override it.
    std::fs::write(
        n.dir.path().join("unit.conf"),
        "# unit\nhub = hub-01\nunit = m07\n",
    )
    .unwrap();
    for (args, stdin) in [
        (vec!["append"], ""),
        (vec!["append", "--key", "bad key"], "x\n"),
        (vec!["ack", "3", "maybe", "x"], ""),
        (vec!["ack", "x", "done", "y"], ""),
        (vec!["ack", "3", "done"], ""),
        (vec!["mirror"], ""),
        (vec!["bogus"], ""),
    ] {
        let out = bare(&args, &[], stdin);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    assert_eq!(n.board_text(), BOARD);
    assert_eq!(n.queued(), 0);
    if has_flock() {
        let out = bare(&["append"], &[], "> 单元(m07)·经配置文件\n");
        assert!(out.status.success(), "stderr: {}", stderr(&out));
        assert!(n.board_text().contains("经配置文件"));
    }
}
