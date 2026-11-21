#![cfg(unix)]

//! Contract tests for `octoscode olp card open|ack|accept|return|void|snooze`
//! and `octoscode olp board render` (SDD spec:
//! `specs/task-olp-card-writes.spec.md`): a card's life written as events.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Sandbox {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn command(&self, group: &str, action: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_octoscode"));
        cmd.args(["olp", group, action]);
        if group == "card" {
            cmd.args(["--log", &self.log()]);
        }
        cmd.args(args);
        cmd
    }

    fn card(&self, action: &str, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command("card", action, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn open(&self, title: &str, extra: &[&str]) -> u64 {
        let mut args = vec!["--title", title];
        args.extend_from_slice(extra);
        let out = self.card("open", &args, "正文\n");
        assert!(out.status.success(), "{}", stderr(&out));
        stdout(&out).lines().last().unwrap().parse().unwrap()
    }

    fn inbox(&self, source: &str, args: &[&str]) -> serde_json::Value {
        let out = self
            .command("board", "inbox", &[source, "--json", "--all"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        let mut report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        report.as_object_mut().unwrap().remove("warnings");
        report
    }

    fn state(&self, n: u64, args: &[&str]) -> (String, String) {
        let report = self.inbox(&self.log(), args);
        let item = report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["number"] == n)
            .unwrap_or_else(|| panic!("no #{n}: {report}"))
            .clone();
        (
            item["partition"].as_str().unwrap().to_string(),
            item["reason"].as_str().unwrap().to_string(),
        )
    }

    fn events(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(PathBuf::from(self.log()).join("events-000001.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// Scenario: 一张卡从立卡到采认,每一步都是一条事件,收件箱跟着变(critical)
#[test]
fn a_card_moves_through_its_life_as_events() {
    let sb = Sandbox::new();
    let n = sb.open(
        "修复启动崩溃",
        &[
            "--unit",
            "m07",
            "--impl",
            "octoscode",
            "--review",
            "codex,claude",
        ],
    );
    assert_eq!(n, 1);
    assert_eq!(sb.open("第二张", &[]), 2);
    assert_eq!(sb.state(1, &[]).0, "working");

    let out = sb.card(
        "ack",
        &["1", "done", "commit", "abc123,", "测试通过", "--by", "m07"],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        sb.state(1, &[]),
        ("inbox".into(), "待复验:内环已 ACK(done)".into())
    );

    let out = sb.card("return", &["1", "缺一个回归测试"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    // A returned unit card still occupies its unit.
    assert_eq!(sb.state(1, &[]).0, "working");

    assert!(
        sb.card("ack", &["1", "done", "补了测试", "--by", "m07"], "")
            .status
            .success()
    );
    assert!(sb.card("accept", &["1", "复验记录齐"], "").status.success());
    assert_eq!(sb.state(1, &[]), ("settled".into(), "已采认".into()));

    let out = sb.card("snooze", &["2", "--until", "2026-12-01", "等上游发版"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(sb.state(2, &["--today", "2026-11-25"]).0, "snoozed");
    assert_eq!(sb.state(2, &["--today", "2026-12-02"]).0, "inbox");
    assert!(sb.card("void", &["2"], "").status.success());
    assert_eq!(sb.state(2, &[]), ("settled".into(), "已被改判作废".into()));

    let events = sb.events();
    let kinds: Vec<&str> = events.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "dispatched",
            "dispatched",
            "ack",
            "returned",
            "ack",
            "accepted",
            "snoozed",
            "voided"
        ]
    );
    assert_eq!(events[0]["actor"], "外环(hub)");
    assert_eq!(events[0]["unit"], "m07");
    assert_eq!(
        events[0]["data"]["review"],
        serde_json::json!(["codex", "claude"])
    );
    assert_eq!(events[0]["data"]["body"], "正文\n");
    assert_eq!(events[2]["actor"], "内环(m07)");
    assert_eq!(events[2]["data"]["text"], "commit abc123, 测试通过");
}

/// Scenario: 不合规则的写入当场拒绝,日志不变
#[test]
fn writes_that_break_a_rule_are_refused_and_nothing_is_written() {
    let sb = Sandbox::new();
    sb.open("唯一的卡", &[]);
    assert!(sb.card("void", &["1", "需求取消"], "").status.success());
    let before = sb.events().len();

    let refused: [(&str, &[&str], &str); 4] = [
        ("ack", &["7", "done", "做完了"], "没有 #7 这张卡"),
        ("accept", &["7", "好"], "没有 #7 这张卡"),
        ("ack", &["1", "done", "做完了"], "已被改判作废"),
        ("accept", &["1", "好"], "已被改判作废"),
    ];
    for (action, args, why) in refused {
        let out = sb.card(action, args, "");
        assert_eq!(out.status.code(), Some(1), "{action} {args:?}");
        assert!(stderr(&out).contains(why), "{}", stderr(&out));
    }
    let usage: [(&str, &[&str], &str); 8] = [
        ("ack", &["1", "finished", "x"], ""),
        ("ack", &["1", "done"], ""),
        ("ack", &["x", "done", "y"], ""),
        ("accept", &["1"], ""),
        ("snooze", &["1", "等"], ""),
        ("snooze", &["1", "--until", "2026-13-40", "等"], ""),
        ("open", &["--title", "  "], "正文\n"),
        ("open", &["--title", "t", "--unit", "M07"], "正文\n"),
    ];
    for (action, args, stdin) in usage {
        let out = sb.card(action, args, stdin);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{action} {args:?}: {}",
            stderr(&out)
        );
    }
    assert_eq!(
        sb.card("open", &["--title", "空正文"], "  \n")
            .status
            .code(),
        Some(2)
    );
    assert_eq!(sb.events().len(), before, "nothing was written");
}

/// Scenario: 同一个键只立一张卡;同键不同内容是错误
#[test]
fn the_same_key_opens_one_card() {
    let sb = Sandbox::new();
    let first = sb.open("带键的卡", &["--unit", "m07", "--key", "dispatch:m07:abc"]);
    let out = sb.card(
        "open",
        &[
            "--title",
            "带键的卡",
            "--unit",
            "m07",
            "--key",
            "dispatch:m07:abc",
            "--json",
        ],
        "正文\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let again: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(again["number"], first);
    assert_eq!(again["created"], false);
    assert_eq!(sb.events().len(), 1);

    let out = sb.card(
        "open",
        &[
            "--title",
            "带键的卡",
            "--unit",
            "m08",
            "--key",
            "dispatch:m07:abc",
        ],
        "正文\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("与本次立卡的内容不符"),
        "{}",
        stderr(&out)
    );

    // A verdict with a key is recorded once too.
    for _ in 0..2 {
        assert!(
            sb.card("accept", &["1", "好", "--key", "accept:1"], "")
                .status
                .success()
        );
    }
    assert_eq!(sb.events().len(), 2);
}

/// Scenario: 多个进程同时立卡,卡号不重不漏(critical)
#[test]
fn concurrent_processes_get_distinct_consecutive_numbers() {
    let sb = Sandbox::new();
    let children: Vec<_> = (0..12)
        .map(|i| {
            let mut child = sb
                .command("card", "open", &["--title", &format!("并发 {i}")])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(b"x\n").unwrap();
            child
        })
        .collect();
    let mut numbers: Vec<u64> = children
        .into_iter()
        .map(|child| {
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{}", stderr(&out));
            stdout(&out).lines().last().unwrap().parse().unwrap()
        })
        .collect();
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=12).collect::<Vec<u64>>());
    let seqs: Vec<u64> = sb
        .events()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, (1..=12).collect::<Vec<u64>>());
}

/// Scenario: 外环会话已结束时,外环的写入被拒(退出码 6);内环回执不受影响
#[test]
fn outer_writes_need_a_live_lease() {
    let sb = Sandbox::new();
    sb.open("租约测试", &[]);
    let dead_lease = |action: &str, args: &[&str]| {
        let mut child = sb
            .command("card", action, args)
            .env("OLP_DUTY_LEASE", "lease-of-a-session-that-ended")
            .env("OLP_DUTY_PROJECT", sb.dir.path())
            .env("HOME", sb.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"x\n").unwrap();
        child.wait_with_output().unwrap()
    };
    for (action, args) in [
        ("open", vec!["--title", "不该写入"]),
        ("accept", vec!["1", "好"]),
        ("void", vec!["1"]),
    ] {
        let out = dead_lease(action, &args);
        assert_eq!(out.status.code(), Some(6), "{action}: {}", stderr(&out));
        assert!(stderr(&out).contains("主审权租约无效"), "{}", stderr(&out));
    }
    assert_eq!(sb.events().len(), 1);
    assert!(dead_lease("ack", &["1", "done", "做完"]).status.success());
}

/// Scenario: 渲染出的 Markdown 被收件箱读成与日志相同的结论(critical)
#[test]
fn the_rendered_board_reads_the_same_as_the_log() {
    let sb = Sandbox::new();
    let a = sb.open(
        "带示例的卡",
        &["--unit", "m07", "--impl", "octoscode", "--review", "codex"],
    );
    // Card text that looks like machine lines stays text.
    let out = sb.card(
        "open",
        &["--title", "正文里有定式"],
        "```\n### 99. 示例\nACK(done): 示例\n> 外环(hub)·采认(#1): 示例\n```\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        sb.card(
            "ack",
            &[&a.to_string(), "blocked", "等依赖", "--by", "m07"],
            ""
        )
        .status
        .success()
    );
    assert!(
        sb.card("snooze", &["2", "--until", "2026-12-01", "等上游"], "")
            .status
            .success()
    );
    let third = sb.open("第三张", &["--unit", "m08"]);
    assert!(
        sb.card(
            "ack",
            &[&third.to_string(), "done", "好了", "--by", "m08"],
            ""
        )
        .status
        .success()
    );
    assert!(
        sb.card("return", &[&third.to_string(), "不对"], "")
            .status
            .success()
    );

    let view = sb
        .command("board", "render", &[&sb.log()])
        .output()
        .unwrap();
    assert!(view.status.success(), "{}", stderr(&view));
    let text = stdout(&view);
    assert!(
        text.contains("只读") && text.contains("### 1. 带示例的卡"),
        "{text}"
    );
    assert!(
        text.contains("unit: m07\nimpl: octoscode\nreview: codex\n"),
        "{text}"
    );
    assert!(
        text.contains("> 内环(m07)·#1 ACK(blocked): 等依赖"),
        "{text}"
    );
    assert!(
        text.contains("> 外环(hub)·搁置(#2) 至 2026-12-01:等上游"),
        "{text}"
    );
    assert!(text.contains("> 外环(hub)·打回(#3): 不对"), "{text}");

    let path = sb.dir.path().join("view.md");
    std::fs::write(&path, &text).unwrap();
    for args in [
        vec![],
        vec!["--today", "2026-11-25"],
        vec!["--today", "2026-12-05"],
    ] {
        assert_eq!(
            sb.inbox(path.to_str().unwrap(), &args),
            sb.inbox(&sb.log(), &args),
            "{args:?}"
        );
    }
    assert_eq!(sb.state(1, &[]).0, "inbox");
    assert_eq!(sb.state(3, &[]).0, "working");
    let out = sb
        .command("board", "render", &["/nonexistent/log"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
