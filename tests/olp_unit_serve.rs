#![cfg(unix)]

//! Contract tests for `octoscode olp remote serve-log` (SDD spec:
//! `specs/task-olp-unit-serve.spec.md`): what a unit may do to the event
//! log through its ssh forced command, and what it may see.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

struct Hub {
    dir: tempfile::TempDir,
}

impl Hub {
    /// #1 → m07, #2 → m08, #3 without a unit.
    fn new() -> Self {
        let hub = Hub {
            dir: tempfile::tempdir().unwrap(),
        };
        for (title, unit) in [
            ("给 m07 的卡", Some("m07")),
            ("给 m08 的卡", Some("m08")),
            ("没有单元的卡", None),
        ] {
            let mut args = vec!["--title", title];
            if let Some(unit) = unit {
                args.extend(["--unit", unit]);
            }
            assert!(hub.outer("open", &args, "正文\n").status.success());
        }
        hub
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn run(&self, mut cmd: Command, stdin: &str) -> Output {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // A request refused before stdin is read closes the pipe first.
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    /// The outer loop, on the hub.
    fn outer(&self, action: &str, args: &[&str], stdin: &str) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_octoscode"));
        cmd.args(["olp", "card", action, "--log", &self.log()])
            .args(args);
        self.run(cmd, stdin)
    }

    /// What `ssh hub <request>` does when the unit's key is pinned to `unit`.
    fn unit(&self, unit: &str, request: &str, stdin: &str) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_octoscode"));
        cmd.args([
            "olp",
            "remote",
            "serve-log",
            "--unit",
            unit,
            "--log",
            &self.log(),
        ])
        .env("SSH_ORIGINAL_COMMAND", request);
        self.run(cmd, stdin)
    }

    fn events(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.dir.path().join("log/events-000001.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn partition(&self, n: u64) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_octoscode"))
            .args(["olp", "board", "inbox", &self.log(), "--json", "--all"])
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["number"] == n)
            .unwrap()["partition"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

/// Scenario: 单元的提问与它的结局各记一次,别的单元不能替它了结
#[test]
fn a_units_question_and_its_end_are_recorded_once() {
    let hub = Hub::new();
    let before = hub.events().len();
    let asked = hub.unit("m07", "question ask q-1234", "接口用哪个版本\n");
    assert!(asked.status.success(), "{}", stderr(&asked));
    let again = hub.unit("m07", "question ask q-1234", "接口用哪个版本\n");
    assert!(stdout(&again).contains("已经记录过"), "{}", stdout(&again));
    for (unit, request, text) in [
        ("m08", "question answer q-1234", ""),
        ("m07", "question answer q-9999", ""),
        ("m07", "question ask q-5678", ""),
        ("m07", "question ask a/b", "问题"),
        ("m07", "question closed q-1234", ""),
    ] {
        let out = hub.unit(unit, request, text);
        assert_eq!(
            out.status.code(),
            Some(3),
            "{unit} {request}: {}",
            stderr(&out)
        );
    }
    assert_eq!(hub.events().len(), before + 1);
    let pending = || {
        let out = Command::new(env!("CARGO_BIN_EXE_octoscode"))
            .args(["olp", "board", "summary", "--board", &hub.log(), "--json"])
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        report["questions"].as_array().unwrap().len()
    };
    assert_eq!(pending(), 1);
    assert!(
        hub.unit("m07", "question timeout q-1234", "")
            .status
            .success()
    );
    let late = hub.unit("m07", "question answer q-1234", "");
    assert!(stdout(&late).contains("已经记录过"), "{}", stdout(&late));
    let events = hub.events();
    assert_eq!(events.len(), before + 2);
    assert_eq!(events[before]["kind"], "asked");
    assert_eq!(events[before]["data"]["question"], "接口用哪个版本");
    assert_eq!(events[before + 1]["kind"], "timed_out");
    assert_eq!(pending(), 0);
}

/// Scenario: 单元只能对派给自己的卡回执(critical)
#[test]
fn a_unit_acks_only_its_own_cards() {
    let hub = Hub::new();
    let out = hub.unit("m07", "ack 1 done", "commit abc,测试通过\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(hub.partition(1), "inbox");
    let ack = hub.events().pop().unwrap();
    assert_eq!(ack["kind"], "ack");
    assert_eq!(ack["actor"], "内环(m07)");
    assert_eq!(ack["unit"], "m07");
    assert_eq!(ack["data"]["text"], "commit abc,测试通过");

    let before = hub.events().len();
    let refused = [
        ("ack 2 done", "派给的是 m08,不是 m07"),
        ("ack 3 done", "派给的是 未指定单元,不是 m07"),
        ("ack 9 done", "没有 #9 这张卡"),
    ];
    for (request, why) in refused {
        let out = hub.unit("m07", request, "做完了\n");
        assert_eq!(out.status.code(), Some(3), "{request}");
        assert!(stderr(&out).contains(why), "{request}: {}", stderr(&out));
    }
    // The unit is the forced command's, whatever the request says.
    let out = hub.unit("m07", "ack 2 done --unit m08", "做完了\n");
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(hub.events().len(), before);
    assert_eq!(hub.partition(2), "working");

    // Nothing more is written about a voided card.
    assert!(hub.outer("void", &["1"], "").status.success());
    let out = hub.unit("m07", "ack 1 done", "再做一次\n");
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("已被改判作废"), "{}", stderr(&out));
}

/// Scenario: 其他请求一律拒收,不写入
#[test]
fn anything_else_is_refused() {
    let hub = Hub::new();
    let before = hub.events().len();
    for (request, stdin) in [
        ("", ""),
        ("append m07:x", "### 9. 立卡\n"),
        ("read", ""),
        ("ack 1", "x\n"),
        ("ack 1 finished", "x\n"),
        ("ack 1 done; rm -rf /", "x\n"),
        ("ack one done", "x\n"),
        ("ack 1 done", ""),
        ("ack 1 done", "两行\n说明\n"),
        ("progress 1", ""),
        ("progress 1 review", ""),
        ("progress 1 review 0", ""),
        ("progress 1 shipped 1", ""),
        ("progress 2 claimed", ""),
        ("events x", ""),
        ("ack 1 'done", "x\n"),
    ] {
        let out = hub.unit("m07", request, stdin);
        assert_eq!(out.status.code(), Some(3), "{request:?}: {}", stderr(&out));
        assert!(stdout(&out).is_empty(), "{request:?}");
    }
    assert_eq!(hub.events().len(), before);
    // A unit id the forced command line could not have is a setup error.
    assert_eq!(hub.unit("M07", "events", "").status.code(), Some(2));
}

/// Scenario: 进度节点每个只记一次,不改变卡的分区
#[test]
fn progress_nodes_are_recorded_once() {
    let hub = Hub::new();
    for (request, text) in [
        ("progress 1 claimed", ""),
        ("progress 1 claimed", ""),
        ("progress 1 review 1", ""),
        ("progress 1 rework 1", "缺回归测试\n"),
        ("progress 1 rework 1", "缺回归测试\n"),
        ("progress 1 review 2", ""),
    ] {
        let out = hub.unit("m07", request, text);
        assert!(out.status.success(), "{request}: {}", stderr(&out));
    }
    let nodes: Vec<String> = hub
        .events()
        .iter()
        .filter(|e| e["kind"] == "progress")
        .map(|e| {
            format!(
                "{} {}",
                e["data"]["node"].as_str().unwrap(),
                e["data"]["round"]
            )
        })
        .collect();
    assert_eq!(nodes, ["claimed 0", "review 1", "rework 1", "review 2"]);
    let rework = hub
        .events()
        .into_iter()
        .find(|e| e["data"]["node"] == "rework")
        .unwrap();
    assert_eq!(rework["actor"], "单元(m07)");
    assert_eq!(rework["data"]["text"], "缺回归测试");
    // Progress is information: the card is still being worked on.
    assert_eq!(hub.partition(1), "working");

    let view = Command::new(env!("CARGO_BIN_EXE_octoscode"))
        .args(["olp", "board", "render", &hub.log()])
        .output()
        .unwrap();
    let text = stdout(&view);
    assert!(text.contains("> 单元(m07)·进度(#1): claimed\n"), "{text}");
    assert!(
        text.contains("> 单元(m07)·进度(#1): rework r1 缺回归测试\n"),
        "{text}"
    );
}

/// Scenario: 单元只看得到自己的卡(critical)
#[test]
fn a_unit_sees_only_its_own_cards() {
    let hub = Hub::new();
    assert!(hub.unit("m07", "progress 1 claimed", "").status.success());
    assert!(
        hub.unit("m08", "ack 2 blocked", "等依赖\n")
            .status
            .success()
    );
    assert!(hub.outer("return", &["2", "换个做法"], "").status.success());

    let seen = |unit: &str, request: &str| -> Vec<serde_json::Value> {
        let out = hub.unit(unit, request, "");
        assert!(out.status.success(), "{}", stderr(&out));
        stdout(&out)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let m07 = seen("m07", "events");
    assert_eq!(
        m07.iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["dispatched", "progress"]
    );
    assert!(m07.iter().all(|e| e["card"] == 1));
    let m08 = seen("m08", "events");
    assert_eq!(
        m08.iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["dispatched", "ack", "returned"]
    );
    assert!(m08.iter().all(|e| e["card"] == 2));
    // Only what is new since the sequence number the unit already has.
    let last = m08[1]["seq"].as_u64().unwrap();
    let newer = seen("m08", &format!("events {last}"));
    assert_eq!(newer.len(), 1);
    assert_eq!(newer[0]["kind"], "returned");
    assert!(
        seen("m09", "events").is_empty(),
        "a unit with no cards sees nothing"
    );
}

/// Scenario: 打回之后同样的回执是新的一次,不当成重发(critical)
#[test]
fn an_ack_after_a_return_is_a_new_answer() {
    let hub = Hub::new();
    let ack = || hub.unit("m07", "ack 1 wontdo m07:ack:1:abc", "做不了:缺权限\n");
    for _ in 0..2 {
        assert!(ack().status.success());
    }
    assert_eq!(hub.events().len(), 4, "the retry was recorded once");
    assert_eq!(hub.partition(1), "inbox");

    assert!(hub.outer("return", &["1", "再试一次"], "").status.success());
    assert_eq!(hub.partition(1), "working");
    // The same result with the same words, after the return.
    let out = ack();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("已记录"), "{}", stdout(&out));
    let last = hub.events().pop().unwrap();
    assert_eq!(last["kind"], "ack");
    assert_eq!(last["key"], "m07:ack:1:abc:r1");
    assert_eq!(hub.partition(1), "inbox", "the card is answered again");
    // A retry of that one is still a retry.
    let before = hub.events().len();
    assert!(stdout(&ack()).contains("没有重复写入"));
    assert_eq!(hub.events().len(), before);
}

/// Scenario: 单元的运行时告警记成事件,一件事一条
#[test]
fn a_units_guard_alert_is_recorded_once() {
    let hub = Hub::new();
    let said = "agent_missing turn=turn-abc pid=4101 state=process_missing\n";
    for _ in 0..2 {
        let out = hub.unit("m07", "alert m07:neg:agent_missing:0a1b '#1'", said);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    let events = hub.events();
    assert_eq!(events.len(), 4, "recorded once");
    let alert = &events[3];
    assert_eq!(alert["kind"], "alert");
    assert_eq!(alert["card"], 1);
    assert_eq!(alert["unit"], "m07");
    assert_eq!(alert["actor"], "单元(m07)·负哨(#1)");
    assert_eq!(alert["data"]["text"], said.trim());

    // A card that is not this unit's is named in the label only.
    let out = hub.unit("m07", "alert m07:neg:x:1 '#2'", said);
    assert!(out.status.success(), "{}", stderr(&out));
    let other = hub.events().pop().unwrap();
    assert!(other.get("card").is_none(), "{other}");
    assert_eq!(other["actor"], "单元(m07)·负哨(#2)");
    // And one without a card at all.
    assert!(
        hub.unit("m07", "alert m07:neg:x:2 unknown", said)
            .status
            .success()
    );

    let before = hub.events().len();
    for (request, text, why) in [
        ("alert m08:neg:x:3 '#1'", said, "键必须以 m07: 开头"),
        ("alert m07:neg:x:3 'bad label'", said, "任务标签不合法"),
        ("alert m07:neg:x:3 '#1'", "\n", "内容为空"),
        ("alert m07:neg:x:3 '#1'", "两行\n告警\n", "单行"),
    ] {
        let out = hub.unit("m07", request, text);
        assert_eq!(out.status.code(), Some(3), "{request}");
        assert!(stderr(&out).contains(why), "{request}: {}", stderr(&out));
    }
    assert_eq!(hub.events().len(), before);

    // The summary shows it with the open card.
    let out = Command::new(env!("CARGO_BIN_EXE_octoscode"))
        .args(["olp", "board", "summary", "--board", &hub.log(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("agent_missing turn=turn-abc"),
        "{}",
        stdout(&out)
    );
}

/// Scenario: 一个键只代表一件事
#[test]
fn a_key_never_stands_for_two_things() {
    let hub = Hub::new();
    assert!(
        hub.unit("m07", "alert m07:one-key '#1'", "turn_error turn=t1\n")
            .status
            .success()
    );
    let before = hub.events().len();
    // The same key on an ACK: not a retry of anything, and not written.
    let out = hub.unit("m07", "ack 1 done m07:one-key", "做完了\n");
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(stderr(&out).contains("不是同一件事"), "{}", stderr(&out));
    assert_eq!(hub.events().len(), before);
    assert_eq!(hub.partition(1), "working");

    // On the hub: a key used for one card is refused for another.
    assert!(
        hub.outer("return", &["1", "--key", "k1", "换个做法"], "")
            .status
            .success()
    );
    assert!(
        hub.outer("return", &["1", "--key", "k1", "换个做法"], "")
            .status
            .success(),
        "the same thing said again is fine"
    );
    let before = hub.events().len();
    let out = hub.outer("return", &["2", "--key", "k1", "换个做法"], "");
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("不是同一件事"), "{}", stderr(&out));
    let out = hub.outer("accept", &["1", "--key", "k1", "通过"], "");
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(hub.events().len(), before);
}
