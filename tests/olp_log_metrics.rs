#![cfg(unix)]

//! Contract tests for `octoscode olp log metrics` (SDD spec:
//! `specs/task-olp-log-metrics.spec.md`): the delivery numbers are read
//! from the whole event log, one row per card, and nothing is written.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");

struct Hub {
    dir: tempfile::TempDir,
    seq: u64,
}

impl Hub {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("log")).unwrap();
        Hub { dir, seq: 0 }
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("log")
    }

    /// Append one event, `minute` minutes after 2026-11-02T00:00Z.
    fn event(&mut self, minute: i64, kind: &str, card: Option<u64>, unit: &str, data: Value) {
        self.seq += 1;
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap()
            + chrono::Duration::minutes(minute);
        let mut event = json!({
            "seq": self.seq,
            "ts": at.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "kind": kind,
            "actor": "测试",
            "unit": unit,
            "data": data,
        });
        if let Some(card) = card {
            event["card"] = json!(card);
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log().join("events-000001.jsonl"))
            .unwrap();
        writeln!(file, "{event}").unwrap();
    }

    /// A card for `unit`: dispatched at `start`, claimed 1 minute later,
    /// receipt after `work` more minutes, accepted 2 minutes after that.
    fn delivered(&mut self, card: u64, unit: &str, start: i64, work: i64) {
        let c = Some(card);
        self.event(
            start,
            "dispatched",
            c,
            unit,
            json!({"title": format!("卡 {card}"), "impl": "octoscode", "pane": "w1:1"}),
        );
        self.event(start + 1, "progress", c, unit, json!({"node": "claimed"}));
        self.event(start + 1 + work, "ack", c, unit, json!({"result": "done"}));
        self.event(start + 3 + work, "accepted", c, unit, json!({}));
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .arg("olp")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn metrics(&self, args: &[&str]) -> Value {
        let log = self.log();
        let mut all = vec!["log", "metrics", log.to_str().unwrap(), "--json"];
        all.extend_from_slice(args);
        let out = self.run(&all);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

/// Scenario: 每张卡一行,汇总的每个数都来自这些行(critical)
#[test]
fn each_card_is_one_row_and_the_summary_comes_from_the_rows() {
    let mut hub = Hub::new();
    hub.delivered(1, "m07", 0, 10);
    // #2: returned once, then accepted.
    let c = Some(2);
    hub.event(0, "dispatched", c, "m08", json!({"title": "卡 2"}));
    hub.event(5, "progress", c, "m08", json!({"node": "claimed"}));
    hub.event(25, "ack", c, "m08", json!({"result": "done"}));
    hub.event(30, "returned", c, "m08", json!({}));
    hub.event(
        40,
        "progress",
        c,
        "m08",
        json!({"node": "review", "round": 2}),
    );
    hub.event(50, "ack", c, "m08", json!({"result": "done"}));
    hub.event(60, "accepted", c, "m08", json!({}));
    // #3: voided; #4: still open, blocked once, with a guard alert.
    hub.event(0, "dispatched", Some(3), "m07", json!({"title": "卡 3"}));
    hub.event(9, "voided", Some(3), "m07", json!({}));
    hub.event(0, "dispatched", Some(4), "m07", json!({"title": "卡 4"}));
    hub.event(3, "ack", Some(4), "m07", json!({"result": "blocked"}));
    hub.event(4, "alert", Some(4), "m07", json!({"text": "agent_missing"}));
    hub.event(
        5,
        "asked",
        None,
        "m07",
        json!({"id": "q1", "question": "?"}),
    );
    hub.event(6, "answered", None, "m07", json!({"id": "q1"}));
    hub.event(
        7,
        "asked",
        None,
        "m08",
        json!({"id": "q2", "question": "?"}),
    );
    hub.event(90, "timed_out", None, "m08", json!({"id": "q2"}));

    let report = hub.metrics(&["--cards"]);
    let s = &report["summary"];
    assert_eq!(
        (
            s["cards"].as_u64(),
            s["accepted"].as_u64(),
            s["voided"].as_u64(),
            s["open"].as_u64()
        ),
        (Some(4), Some(2), Some(1), Some(1)),
        "{s}"
    );
    assert_eq!(s["first_pass"], 1, "#2 was returned");
    assert_eq!(
        (
            s["returns"].as_u64(),
            s["blocked"].as_u64(),
            s["alerts"].as_u64()
        ),
        (Some(1), Some(1), Some(1))
    );
    assert_eq!(
        report["questions"],
        json!({"asked": 2, "answered": 1, "timed_out": 1})
    );
    // Totals of the two accepted cards: 13 and 60 minutes.
    assert_eq!(
        s["total"],
        json!({"cards": 2, "p50": 13 * 60, "p90": 60 * 60})
    );
    let cards = report["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 4);
    let two = &cards[1];
    assert_eq!(two["to_claim"], 5 * 60);
    // 5→25 and 30→50.
    assert_eq!(two["work"], 40 * 60);
    // 25→30 and 50→60.
    assert_eq!(two["outer_wait"], 15 * 60);
    assert_eq!(
        (two["returns"].as_u64(), two["review_rounds"].as_u64()),
        (Some(1), Some(2))
    );
    assert_eq!(two["first_pass"], false);
    assert_eq!(cards[2]["outcome"], "voided");
    assert_eq!(cards[2]["total"], Value::Null);
    // The voided card's time is in no spread.
    assert_eq!(s["to_claim"]["cards"], 2);
}

/// Scenario: 时间窗与分组只改变算哪些卡(critical)
#[test]
fn a_window_and_a_grouping_narrow_what_is_counted() {
    let mut hub = Hub::new();
    hub.delivered(1, "m07", 0, 10);
    hub.delivered(2, "m08", 60, 20);
    // Eight days later: the next ISO week.
    hub.delivered(3, "m08", 8 * 24 * 60, 30);

    let all = hub.metrics(&["--by", "unit"]);
    assert_eq!(all["summary"]["cards"], 3);
    assert_eq!(all["groups"]["m07"]["cards"], 1);
    assert_eq!(all["groups"]["m08"]["cards"], 2);
    assert_eq!(all["groups"]["m08"]["work"]["p90"], 30 * 60);

    let weeks = hub.metrics(&["--by", "week"]);
    assert_eq!(weeks["groups"]["2026-W45"]["cards"], 2, "{weeks}");
    assert_eq!(weeks["groups"]["2026-W46"]["cards"], 1);

    let late = hub.metrics(&["--since", "2026-11-03"]);
    assert_eq!(late["summary"]["cards"], 1);
    let early = hub.metrics(&["--until", "2026-11-02T00:30:00Z"]);
    assert_eq!(early["summary"]["cards"], 1);
    assert_eq!(early["summary"]["work"]["p50"], 10 * 60);

    let by_impl = hub.metrics(&["--by", "impl"]);
    assert_eq!(by_impl["groups"]["octoscode"]["cards"], 3);
}

/// Scenario: 压缩过的日志,指标与压缩前一样(critical)
#[test]
fn metrics_read_the_archive_too() {
    let mut hub = Hub::new();
    for card in 1..=6 {
        hub.delivered(card, "m07", card as i64 * 100, card as i64);
    }
    let before = hub.metrics(&["--cards"]);
    let log = hub.log();
    let out = hub.run(&[
        "log",
        "compact",
        log.to_str().unwrap(),
        "--keep-days",
        "1",
        "--keep",
        "1",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        log.join("archive").is_dir(),
        "the compaction archived something"
    );
    let after = hub.metrics(&["--cards"]);
    assert_eq!(before, after);
    assert_eq!(after["summary"]["accepted"], 6);
}

/// Scenario: 只读,用法错与读不了各有退出码
#[test]
fn metrics_write_nothing_and_say_how_they_were_misused() {
    let mut hub = Hub::new();
    hub.delivered(1, "m07", 0, 10);
    let log = hub.log();
    let listing = || {
        let mut names: Vec<_> = std::fs::read_dir(&log)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "events.lock")
            .collect();
        names.sort();
        names
    };
    let before = (
        listing(),
        std::fs::read(log.join("events-000001.jsonl")).unwrap(),
    );
    let out = hub.run(&[
        "log",
        "metrics",
        log.to_str().unwrap(),
        "--by",
        "unit",
        "--cards",
    ]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("1 张卡") && text.contains("#1"), "{text}");
    assert_eq!(
        before,
        (
            listing(),
            std::fs::read(log.join("events-000001.jsonl")).unwrap()
        )
    );

    for bad in [
        vec!["log", "metrics"],
        vec!["log", "metrics", log.to_str().unwrap(), "--by", "colour"],
        vec!["log", "metrics", log.to_str().unwrap(), "--since", "昨天"],
        vec!["log", "metrics", "/nonexistent/olp-log"],
    ] {
        assert_eq!(hub.run(&bad).status.code(), Some(2), "{bad:?}");
    }
    std::fs::write(log.join("events-000001.jsonl"), "{\"seq\":1,\"ts\":\"x\",\"kind\":\"a\",\"actor\":\"b\",\"data\":{}}\nnot json\n{\"seq\":3,\"ts\":\"x\",\"kind\":\"a\",\"actor\":\"b\",\"data\":{}}\n").unwrap();
    assert_eq!(
        hub.run(&["log", "metrics", log.to_str().unwrap()])
            .status
            .code(),
        Some(4)
    );
}
