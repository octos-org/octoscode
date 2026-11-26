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
