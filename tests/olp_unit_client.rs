#![cfg(unix)]

//! Contract tests for the unit's side of the event log (SDD spec:
//! `specs/task-olp-unit-client.spec.md`): `octoscode olp remote ack |
//! progress | mirror` against a hub that keeps an event log.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A hub log with #1 → m07 and #2 → m08, and a unit configuration.
struct Site {
    dir: tempfile::TempDir,
}

impl Site {
    fn new(conf: &str) -> Self {
        let site = Site {
            dir: tempfile::tempdir().unwrap(),
        };
        for (title, unit) in [("给 m07 的卡", "m07"), ("给 m08 的卡", "m08")] {
            let out = site.hub(
                &[
                    "card",
                    "open",
                    "--log",
                    &site.log(),
                    "--title",
                    title,
                    "--unit",
                    unit,
                ],
                "正文\n",
            );
            assert!(out.status.success(), "{}", stderr(&out));
        }
        let conf = conf
            .replace("{log}", &site.log())
            .replace("{mirror}", site.mirror().to_str().unwrap());
        std::fs::write(site.dir.path().join("unit.conf"), conf).unwrap();
        site
    }

    fn log(&self) -> String {
        self.dir
            .path()
            .join("hub/log")
            .to_string_lossy()
            .into_owned()
    }

    fn mirror(&self) -> PathBuf {
        self.dir.path().join("unit/BOARD.md")
    }

    fn run(&self, mut cmd: Command, stdin: &str) -> Output {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn hub(&self, args: &[&str], stdin: &str) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.arg("olp").args(args);
        self.run(cmd, stdin)
    }

    /// A command typed on the unit.
    fn unit(&self, args: &[&str]) -> Output {
        self.unit_env(args, &[])
    }

    fn unit_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.args(["olp", "remote"])
            .args(args)
            .env("OLP_UNIT_CONF", self.dir.path().join("unit.conf"))
            .env("HOME", self.dir.path());
        for (key, value) in envs {
            cmd.env(key, value);
        }
        self.run(cmd, "")
    }

    fn events(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(PathBuf::from(self.log()).join("events-000001.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// An `ssh` that reaches the hub: it drops ssh's own options and the
    /// host, then runs the forced command with the rest as the request.
    fn fake_ssh(&self, reachable: bool) -> PathBuf {
        let path = self.dir.path().join("fake-ssh");
        let script = if reachable {
            format!(
                "#!/bin/sh\nwhile [ \"$1\" = -o ]; do shift 2; done\necho \"$1\" > '{0}/ssh-host'\nshift\nSSH_ORIGINAL_COMMAND=\"$*\" exec '{BIN}' olp remote serve-log --unit m07 --log '{1}'\n",
                self.dir.path().display(),
                self.log()
            )
        } else {
            "#!/bin/sh\necho 'ssh: connect to host hub-01 port 22: Connection refused' >&2\nexit 255\n".to_string()
        };
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

const LOCAL: &str = "hub = local\nunit = m07\nstore = log\nlog = {log}\nmirror = {mirror}\n";
const OVER_SSH: &str = "hub = hub-01\nunit = m07\nstore = log\nmirror = {mirror}\n";

/// Scenario: 单元上报进度与回执,重发不重复(critical)
#[test]
fn a_unit_reports_progress_and_acks_once() {
    let site = Site::new(LOCAL);
    for args in [
        vec!["progress", "1", "claimed"],
        vec!["progress", "1", "review", "1"],
        vec!["progress", "1", "rework", "1", "缺", "回归测试"],
        vec!["ack", "1", "done", "commit", "abc,", "测试通过"],
        // A retry after a lost reply.
        vec!["ack", "1", "done", "commit", "abc,", "测试通过"],
        vec!["progress", "1", "claimed"],
    ] {
        let out = site.unit(&args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    }
    let events = site.events();
    let kinds: Vec<&str> = events[2..]
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["progress", "progress", "progress", "ack"]);
    assert_eq!(events[4]["data"]["text"], "缺 回归测试");
    let ack = &events[5];
    assert_eq!(ack["actor"], "内环(m07)");
    assert_eq!(ack["data"]["text"], "commit abc, 测试通过");
    assert!(
        ack["key"].as_str().unwrap().starts_with("m07:ack:1:"),
        "{ack}"
    );

    // A different ACK is a new event; someone else's card is refused by the hub.
    assert!(
        site.unit(&["ack", "1", "blocked", "又卡住了"])
            .status
            .success()
    );
    assert_eq!(site.events().len(), 7);
    let out = site.unit(&["ack", "2", "done", "不是我的卡"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("派给的是 m08"), "{}", stderr(&out));
    let out = site.unit(&["progress", "2", "claimed"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(site.events().len(), 7);

    for args in [
        vec!["ack", "1", "done"],
        vec!["ack", "1"],
        vec!["progress", "1"],
        vec!["progress", "1", "review"],
    ] {
        assert_eq!(site.unit(&args).status.code(), Some(2), "{args:?}");
    }
}

/// Scenario: hub 的日志换过或压缩过,本机的事件缓存按 hub 现有的内容重建
#[test]
fn a_cache_about_another_log_is_rebuilt_from_the_hub() {
    let site = Site::new(LOCAL);
    assert!(site.unit(&["progress", "1", "claimed"]).status.success());
    let out = site.unit(&["mirror"]);
    assert!(
        stdout(&out).contains("新增 2 条事件,共 2 条"),
        "{}",
        stdout(&out)
    );
    // The cache is about some other log: same numbers, another time.
    let cache = PathBuf::from(format!("{}.events.jsonl", site.mirror().display()));
    let other = std::fs::read_to_string(&cache)
        .unwrap()
        .replace("\"ts\":\"20", "\"ts\":\"19")
        .replace("给 m07 的卡", "别的日志里的卡");
    std::fs::write(&cache, other).unwrap();
    let out = site.unit(&["mirror"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("已按 hub 现有的内容重建") && stdout(&out).contains("共 2 条"),
        "{}",
        stdout(&out)
    );
    let view = std::fs::read_to_string(site.mirror()).unwrap();
    assert!(
        view.contains("给 m07 的卡") && !view.contains("别的日志里的卡"),
        "{view}"
    );
    // And the next run is incremental again.
    let out = site.unit(&["mirror"]);
    assert!(
        stdout(&out).contains("新增 0 条事件,共 2 条"),
        "{}",
        stdout(&out)
    );

    // The hub archives the card: the cache follows the hub.
    assert!(site.unit(&["ack", "1", "done", "做完了"]).status.success());
    let out = site.hub(&["card", "accept", "--log", &site.log(), "1", "好"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(site.unit(&["mirror"]).status.success());
    let out = site.hub(
        &[
            "log",
            "compact",
            &site.log(),
            "--keep-days",
            "0",
            "--keep",
            "0",
        ],
        "",
    );
    assert!(
        stdout(&out).contains("已压缩"),
        "{}{}",
        stdout(&out),
        stderr(&out)
    );
    let out = site.unit(&["mirror"]);
    assert!(
        stdout(&out).contains("重建") && stdout(&out).contains("共 0 条"),
        "{}",
        stdout(&out)
    );
}

/// Scenario: 镜像只有本单元的卡,每次只取新增的事件(critical)
#[test]
fn the_mirror_holds_only_this_units_cards_and_grows_incrementally() {
    let site = Site::new(LOCAL);
    assert!(site.unit(&["progress", "1", "claimed"]).status.success());
    let out = site.unit(&["mirror"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("新增 2 条事件,共 2 条"),
        "{}",
        stdout(&out)
    );
    let view = std::fs::read_to_string(site.mirror()).unwrap();
    assert!(
        view.contains("### 1. 给 m07 的卡") && view.contains("只读"),
        "{view}"
    );
    assert!(view.contains("> 单元(m07)·进度(#1): claimed"), "{view}");
    assert!(
        !view.contains("m08"),
        "another unit's card is not mirrored: {view}"
    );

    // Nothing new: nothing fetched, the mirror stays.
    let out = site.unit(&["mirror"]);
    assert!(
        stdout(&out).contains("新增 0 条事件,共 2 条"),
        "{}",
        stdout(&out)
    );

    // The outer loop returns the card; the next sync brings just that.
    assert!(site.unit(&["ack", "1", "done", "做完了"]).status.success());
    let out = site.hub(&["card", "return", "--log", &site.log(), "1", "缺测试"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    let out = site.unit(&["mirror"]);
    assert!(
        stdout(&out).contains("新增 2 条事件,共 4 条"),
        "{}",
        stdout(&out)
    );
    let view = std::fs::read_to_string(site.mirror()).unwrap();
    assert!(view.contains("> 外环(hub)·打回(#1): 缺测试"), "{view}");

    // The Markdown tools read the mirror: returned, so the unit still has it.
    let out = site.hub(
        &["board", "inbox", site.mirror().to_str().unwrap(), "--units"],
        "",
    );
    assert!(
        stdout(&out).contains("m07\t忙\t#1 已打回"),
        "{}",
        stdout(&out)
    );

    let other = site.dir.path().join("elsewhere/view.md");
    let out = site.unit(&["mirror", "--to", other.to_str().unwrap()]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("新增 4 条事件"),
        "a new mirror starts from the beginning"
    );
}

/// Scenario: 经 ssh 的请求原样到达 hub;连不上时退出码 75,重跑不重复
#[test]
fn requests_travel_over_ssh_and_an_unreachable_hub_is_retryable() {
    let site = Site::new(OVER_SSH);
    let down = site.fake_ssh(false);
    let out = site.unit_env(
        &["ack", "1", "done", "it's", "done; rm -rf /"],
        &[("OLP_SSH", down.to_str().unwrap())],
    );
    assert_eq!(out.status.code(), Some(75), "{}", stderr(&out));
    assert!(stderr(&out).contains("没有连上 hub"), "{}", stderr(&out));
    assert_eq!(
        site.unit_env(&["mirror"], &[("OLP_SSH", down.to_str().unwrap())])
            .status
            .code(),
        Some(75)
    );
    assert_eq!(site.events().len(), 2);

    let up = site.fake_ssh(true);
    let ssh = [("OLP_SSH", up.to_str().unwrap())];
    for _ in 0..2 {
        let out = site.unit_env(&["ack", "1", "done", "it's", "done; rm -rf /"], &ssh);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    assert_eq!(
        std::fs::read_to_string(site.dir.path().join("ssh-host"))
            .unwrap()
            .trim(),
        "hub-01"
    );
    let events = site.events();
    assert_eq!(events.len(), 3, "the retry was recorded once");
    assert_eq!(events[2]["data"]["text"], "it's done; rm -rf /");
    assert!(
        site.unit_env(&["progress", "1", "review", "2"], &ssh)
            .status
            .success()
    );
    let out = site.unit_env(&["mirror"], &ssh);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        std::fs::read_to_string(site.mirror())
            .unwrap()
            .contains("进度(#1): review r2")
    );
}

/// Scenario: hub 还在用 Markdown 黑板的单元,命令照旧走原通道
#[test]
fn a_unit_on_a_markdown_hub_keeps_the_old_channel() {
    let site = Site::new("hub = local\nunit = m07\n");
    let board = site.dir.path().join("board.md");
    std::fs::write(&board, "# 板\n\n### 1. 旧卡\nunit: m07\n").unwrap();
    let out = site.unit_env(
        &["ack", "1", "done", "走旧通道"],
        &[("OLP_BOARD", board.to_str().unwrap())],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(&board).unwrap();
    assert!(
        text.contains("> 内环(m07)·#1 ACK(done): 走旧通道"),
        "{text}"
    );
    assert_eq!(site.events().len(), 2, "the event log was not touched");
    // Progress needs the event log, and says so.
    let out = site.unit(&["progress", "1", "claimed"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("store = log"), "{}", stderr(&out));
}

/// Scenario: 连上以后卡住的 ssh 到时限就结束,算没有连上
#[test]
fn a_connection_that_stalls_ends_at_the_deadline() {
    let site = Site::new(OVER_SSH);
    // An ssh that connects and then says nothing for a long time.
    let stuck = site.dir.path().join("stuck-ssh");
    std::fs::write(&stuck, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o755)).unwrap();
    let envs = [
        ("OLP_SSH", stuck.to_str().unwrap()),
        ("OLP_SSH_TIMEOUT", "1"),
    ];
    let started = std::time::Instant::now();
    let out = site.unit_env(&["ack", "1", "done", "测试通过"], &envs);
    assert_eq!(out.status.code(), Some(75), "{}", stderr(&out));
    assert!(stderr(&out).contains("没有连上 hub"), "{}", stderr(&out));
    assert_eq!(site.unit_env(&["mirror"], &envs).status.code(), Some(75));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "both ended at their deadline, not at the ssh's own end"
    );
    assert_eq!(site.events().len(), 2, "nothing was recorded");
}
