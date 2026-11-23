#![cfg(unix)]

//! Contract tests for `olp board stall` (SDD spec:
//! `specs/task-olp-stall.spec.md`): the cluster is stalled when cards are
//! open, nobody works on them and the log is silent; each silence is
//! reported with its card, cause and event, and not over and over.

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

    fn ok(&self, args: &[&str], stdin: &str) -> String {
        let out = self.olp(args, stdin);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    }

    /// Register `unit` and give it a heartbeat whose one inner loop is in `status`.
    fn beat(&self, unit: &str, status: &str) {
        let host = format!("host-{unit}");
        let mac = format!("aa:bb:cc:dd:ee:{:02x}", unit.as_bytes()[unit.len() - 1]);
        self.ok(
            &["unit", "register", unit, "--hostname", &host, "--mac", &mac],
            "",
        );
        let beat = serde_json::json!({
            "v": 1, "unit": unit, "hostname": host, "macs": [mac], "checks": [],
            "agents": [{"kind": "octoscode", "status": status, "pane": "w1:p1"}],
        });
        self.ok(&["unit", "receive"], &beat.to_string());
    }

    fn card(&self, unit: &str, title: &str) {
        let log = self.log();
        self.ok(
            &[
                "card",
                "dispatch",
                unit,
                "--log",
                &log,
                "--title",
                title,
                "--wake",
                "w1:p1",
                "--allow-busy",
            ],
            "正文\n",
        );
    }

    fn stall(&self, extra: &[&str]) -> serde_json::Value {
        let log = self.log();
        let mut args = vec!["board", "stall", &log, "--json"];
        args.extend_from_slice(extra);
        serde_json::from_str(&self.ok(&args, "")).unwrap()
    }
}

/// Scenario: 有人在工作、或日志刚有动静,都不算静默
#[test]
fn working_units_and_a_fresh_log_are_not_a_stall() {
    let hub = Hub::new();
    hub.beat("m07", "working");
    hub.beat("m08", "idle");
    std::fs::create_dir_all(hub.log()).unwrap();
    // No open card at all.
    let seen = hub.stall(&["--quiet", "0"]);
    assert_eq!(
        (seen["stalled"].as_bool(), seen["open"].as_u64()),
        (Some(false), Some(0))
    );

    hub.card("m07", "长时间构建");
    hub.card("m08", "等着的卡");
    // One holder is working: a long build is normal.
    let seen = hub.stall(&["--quiet", "0"]);
    assert_eq!(seen["stalled"], false, "{seen}");
    assert_eq!(seen["working_units"], serde_json::json!(["m07"]));
    assert!(seen["alerts"].as_array().unwrap().is_empty());

    // Nobody works now, but the log moved a moment ago.
    hub.beat("m07", "idle");
    let seen = hub.stall(&[]);
    assert_eq!(seen["stalled"], false, "{seen}");
    let text = hub.ok(&["board", "stall", &hub.log()], "");
    assert!(text.contains("还没到静默的时间"), "{text}");
}

/// Scenario: 整体静默时逐张卡说明原因、起点和谁能推动
#[test]
fn a_stall_names_each_card_its_cause_and_who_can_move_it() {
    let hub = Hub::new();
    hub.beat("m07", "idle");
    hub.beat("m08", "blocked");
    hub.card("m07", "没领取的卡");
    hub.card("m08", "领了没回执的卡");
    hub.card("m08", "等外环复验的卡");
    let log = hub.log();
    let conf = hub.dir.path().join("unit.conf");
    std::fs::write(
        &conf,
        format!("hub = local\nunit = m08\nstore = log\nlog = {log}\n"),
    )
    .unwrap();
    let unit = |args: &[&str]| {
        let out = Command::new(BIN)
            .args(["olp", "remote"])
            .args(args)
            .env("OLP_UNIT_CONF", &conf)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    };
    unit(&["progress", "2", "claimed"]);
    unit(&["progress", "3", "claimed"]);
    unit(&["ack", "3", "done", "commit abc1234,测试通过"]);

    let seen = hub.stall(&["--quiet", "0"]);
    assert_eq!(seen["stalled"], true, "{seen}");
    let alerts = seen["alerts"].as_array().unwrap();
    assert_eq!(alerts.len(), 3, "{seen}");
    let of = |n: u64| alerts.iter().find(|a| a["number"] == n).unwrap();
    assert_eq!(of(1)["waits_on"], "unit");
    assert!(of(1)["cause"].as_str().unwrap().contains("没有领取"));
    assert_eq!(
        (of(1)["unit"].as_str(), of(1)["seq"].as_u64()),
        (Some("m07"), Some(1))
    );
    assert!(of(2)["cause"].as_str().unwrap().contains("领取后没有回执"));
    assert_eq!(of(3)["waits_on"], "outer");
    assert!(
        of(3)["cause"].as_str().unwrap().contains("待复验"),
        "{seen}"
    );
    assert!(!of(3)["since"].as_str().unwrap().is_empty());

    // A unit the hub cannot see is said to be unseen, not assumed busy.
    hub.ok(
        &[
            "card",
            "dispatch",
            "m09",
            "--log",
            &log,
            "--title",
            "未登记单元的卡",
            "--skip-health",
        ],
        "正文\n",
    );
    let seen = hub.stall(&["--quiet", "0", "--state", "/dev/null"]);
    assert_eq!(seen["unknown_units"], serde_json::json!(["m09"]));
    assert_eq!(seen["stalled"], true);
}

/// Scenario: 同一段沉默不连着报,变了才再报,最多三次
#[test]
fn the_same_silence_is_not_reported_over_and_over() {
    let hub = Hub::new();
    hub.beat("m07", "idle");
    hub.card("m07", "卡一");
    hub.card("m07", "卡二");
    let count = |seen: &serde_json::Value| seen["alerts"].as_array().unwrap().len();
    let run = || hub.stall(&["--quiet", "0"]);
    assert_eq!(count(&run()), 2); // run 1: both, first report
    let again = run(); // run 2
    assert_eq!((count(&again), again["held_back"].as_u64()), (0, Some(2)));
    assert_eq!(again["stalled"], true);
    assert_eq!(count(&run()), 0); // run 3

    // Something happened to card 2: it is reported afresh, card 1 is not.
    let log = hub.log();
    hub.ok(
        &[
            "card",
            "snooze",
            "--log",
            &log,
            "2",
            "--until",
            "2020-01-01",
            "等过了",
        ],
        "",
    );
    let seen = run(); // run 4: card 1 is due again as well (3 runs later)
    let numbers: Vec<u64> = seen["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1, 2], "{seen}");
    assert_eq!(seen["alerts"][0]["reports"], 2);
    assert_eq!(seen["alerts"][1]["reports"], 1);

    // Card 1 gets its third report three runs later, and then no more.
    let mut reports_of_1 = 2;
    for _ in 0..12 {
        let seen = run();
        if let Some(a) = seen["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["number"] == 1)
        {
            reports_of_1 = a["reports"].as_u64().unwrap();
        }
    }
    assert_eq!(reports_of_1, 3);
    assert_eq!(count(&run()), 0);

    // Usage.
    let out = hub.olp(&["board", "stall"], "");
    assert_eq!(out.status.code(), Some(2));
    let out = hub.olp(&["board", "stall", "/no/such/dir"], "");
    assert_eq!(out.status.code(), Some(2));
}
