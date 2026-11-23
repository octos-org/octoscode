#![cfg(unix)]

//! Contract tests for dispatching to the event log (SDD spec:
//! `specs/task-olp-dispatch-log.spec.md`): `olp card dispatch --log` checks
//! the unit's health, its reviewers and its load, then writes the card.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");

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
    fn new() -> Self {
        Hub {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn olp(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(BIN)
            .arg("olp")
            .args(args)
            .env("OLP_UNITS_DIR", self.dir.path().join("units"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn dispatch(&self, unit: &str, title: &str, extra: &[&str]) -> Output {
        let log = self.log();
        let mut args = vec!["card", "dispatch", unit, "--log", &log, "--title", title];
        args.extend_from_slice(extra);
        self.olp(&args, "背景与验收标准\n")
    }

    /// Register a unit and give it a fresh heartbeat with these reviewers
    /// (kind, installed, logged in) and inner loops.
    fn unit(&self, unit: &str, checks_ok: bool, reviewers: &[(&str, bool, bool)], agents: &[&str]) {
        let host = format!("host-{unit}");
        let mac = format!("aa:bb:cc:dd:ee:{:02x}", unit.as_bytes()[unit.len() - 1]);
        let out = self.olp(
            &["unit", "register", unit, "--hostname", &host, "--mac", &mac],
            "",
        );
        assert!(out.status.success(), "{}", stderr(&out));
        let beat = serde_json::json!({
            "v": 1, "unit": unit, "hostname": host, "macs": [mac],
            "checks": [{"name": "disk", "ok": checks_ok, "detail": "空闲 3%"}],
            "reviewers": reviewers.iter().map(|(kind, installed, logged_in)| {
                serde_json::json!({"kind": kind, "installed": installed, "logged_in": logged_in})
            }).collect::<Vec<_>>(),
            "agents": agents.iter().map(|kind| {
                serde_json::json!({"kind": kind, "status": "idle", "pane": "w1:p1"})
            }).collect::<Vec<_>>(),
        });
        let out = self.olp(&["unit", "receive"], &beat.to_string());
        assert!(out.status.success(), "{}", stderr(&out));
    }

    fn events(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir(self.log()) else {
            return out;
        };
        let mut files: Vec<_> = dir.map(|e| e.unwrap().path()).collect();
        files.sort();
        for file in files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == "jsonl"))
        {
            for line in std::fs::read_to_string(file).unwrap().lines() {
                out.push(serde_json::from_str(line).unwrap());
            }
        }
        out
    }
}

/// Scenario: 健康且空闲的单元接到卡,同一个键重跑不重复派卡
#[test]
fn a_healthy_free_unit_gets_the_card_once() {
    let hub = Hub::new();
    hub.unit("m07", true, &[("codex", true, true)], &["octoscode"]);
    let extra = [
        "--impl",
        "octoscode",
        "--review",
        "codex",
        "--wake",
        "w1:p1",
        "--json",
    ];
    let out = hub.dispatch("m07", "修复解析器", &extra);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let said: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(said["number"], 1);
    assert_eq!(said["created"], true);
    assert_eq!(said["pane"], "w1:p1");
    assert!(said["key"].as_str().unwrap().starts_with("dispatch:m07:"));

    // The same command again: the same card, nothing new, and not "busy".
    let again = hub.dispatch("m07", "修复解析器", &extra);
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    let said: serde_json::Value = serde_json::from_str(&stdout(&again)).unwrap();
    assert_eq!(
        (said["number"].as_u64(), said["created"].as_bool()),
        (Some(1), Some(false))
    );
    let events = hub.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["kind"], "dispatched");
    assert_eq!(events[0]["unit"], "m07");
    assert_eq!(events[0]["data"]["pane"], "w1:p1");
    assert_eq!(events[0]["data"]["review"], serde_json::json!(["codex"]));

    // Usage errors write nothing.
    for bad in [
        vec!["--impl", "octoscode"],
        vec!["--review", "codex,codex"],
        vec!["--wake", "bad pane"],
        vec!["--key", "有空格 的键"],
    ] {
        let out = hub.dispatch("m07", "另一张", &bad);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {}", stderr(&out));
    }
    assert_eq!(hub.events().len(), 1);
}

/// Scenario: 不健康、未登记、正忙的单元不接卡
#[test]
fn an_unhealthy_or_busy_unit_is_refused() {
    let hub = Hub::new();
    hub.unit("m07", false, &[], &["octoscode"]);
    let out = hub.dispatch("m07", "卡", &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("降级") && stderr(&out).contains("disk"),
        "{}",
        stderr(&out)
    );
    let out = hub.dispatch("m99", "卡", &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("未登记"), "{}", stderr(&out));
    assert!(hub.events().is_empty());

    // --skip-health dispatches anyway; the unit then holds a card.
    let out = hub.dispatch("m07", "第一张", &["--skip-health"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = hub.dispatch("m07", "第二张", &["--skip-health"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("正忙") && stderr(&out).contains("#1"),
        "{}",
        stderr(&out)
    );
    let out = hub.dispatch("m07", "第二张", &["--skip-health", "--allow-busy"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(hub.events().len(), 2);
}

/// Scenario: 卡上的复验方必须是单元心跳里可用的
#[test]
fn reviewers_the_unit_cannot_run_are_refused() {
    let hub = Hub::new();
    hub.unit(
        "m07",
        true,
        &[
            ("claude", true, false),
            ("codex", true, true),
            ("gemini", false, false),
        ],
        &["octoscode", "kimi"],
    );
    for (review, why) in [
        ("claude", "claude 未登录"),
        ("gemini", "gemini 未安装"),
        ("qwen", "qwen 不在心跳里"),
        ("codex,claude", "claude 未登录"),
    ] {
        let out = hub.dispatch("m07", "卡", &["--impl", "octoscode", "--review", review]);
        assert_eq!(out.status.code(), Some(1), "{review}");
        assert!(stderr(&out).contains(why), "{review}: {}", stderr(&out));
        assert!(stderr(&out).contains("--allow-reviewers"));
    }
    assert!(hub.events().is_empty());
    // A logged-in CLI and a running inner loop are both fine.
    let out = hub.dispatch(
        "m07",
        "卡",
        &["--impl", "octoscode", "--review", "codex,kimi"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // The operator named it: dispatched as told.
    let out = hub.dispatch(
        "m07",
        "点名的卡",
        &[
            "--impl",
            "octoscode",
            "--review",
            "claude",
            "--allow-reviewers",
            "--allow-busy",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

/// Scenario: 回测卡不派给原作者
#[test]
fn a_verification_card_does_not_go_back_to_the_author() {
    let hub = Hub::new();
    hub.unit("m07", true, &[], &["octoscode"]);
    hub.unit("m08", true, &[], &["octoscode"]);
    assert_eq!(hub.dispatch("m07", "原卡", &[]).status.code(), Some(0));
    let out = hub.dispatch("m07", "回测", &["--verify-of", "1", "--allow-busy"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("另一台单元"), "{}", stderr(&out));
    let out = hub.dispatch("m08", "回测", &["--verify-of", "9"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("没有 #9"), "{}", stderr(&out));
    let out = hub.dispatch("m08", "回测", &["--verify-of", "1"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let rendered = stdout(&hub.olp(&["board", "render", &hub.log()], ""));
    assert!(rendered.contains("verify-of: #1"), "{rendered}");
}

/// Scenario: 卡上写明不许碰什么,单元在收件箱里看得到
#[test]
fn what_a_card_must_not_touch_reaches_the_unit() {
    let hub = Hub::new();
    hub.unit("m07", true, &[], &["octoscode"]);
    let out = hub.dispatch(
        "m07",
        "改解析器",
        &[
            "--wake",
            "w1:p1",
            "--avoid",
            "src/lexer.rs",
            "--avoid",
            "公开的 API 签名",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let events = hub.events();
    assert_eq!(
        events[0]["data"]["avoid"],
        serde_json::json!(["src/lexer.rs", "公开的 API 签名"])
    );
    let rendered = stdout(&hub.olp(&["board", "render", &hub.log()], ""));
    assert!(
        rendered.contains("avoid: src/lexer.rs\navoid: 公开的 API 签名\n"),
        "{rendered}"
    );

    // The unit's inbox, through the unit's client.
    let log = hub.log();
    let conf = hub.dir.path().join("unit.conf");
    std::fs::write(
        &conf,
        format!("hub = local\nunit = m07\nstore = log\nlog = {log}\n"),
    )
    .unwrap();
    let out = Command::new(BIN)
        .args(["olp", "remote", "inbox"])
        .env("OLP_UNIT_CONF", &conf)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let inbox: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        inbox["items"][0]["avoid"],
        serde_json::json!(["src/lexer.rs", "公开的 API 签名"])
    );
    assert!(inbox["how"].as_str().unwrap().contains("avoid"), "{inbox}");

    // A card without the list says nothing about it.
    let out = hub.dispatch("m07", "另一张", &["--wake", "w1:p2", "--allow-busy"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = Command::new(BIN)
        .args(["olp", "remote", "inbox", "w1:p2"])
        .env("OLP_UNIT_CONF", &conf)
        .output()
        .unwrap();
    let inbox: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(inbox["items"][0].get("avoid").is_none(), "{inbox}");
    assert!(!inbox["how"].as_str().unwrap().contains("avoid"));
}
