#![cfg(unix)]

//! Contract tests for `octoscode olp log compact|history` (SDD spec:
//! `specs/task-olp-log-compact.spec.md`): cards that are over leave the
//! log every command replays, and nothing that is still decided on it
//! changes.

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
        self.path("log").to_string_lossy().into_owned()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(BIN)
            .arg("olp")
            .args(args)
            .env("OLP_UNITS_DIR", self.path("units"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn ok(&self, args: &[&str], stdin: &str) -> String {
        let out = self.run(args, stdin);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    }

    /// `olp card <action> --log <log> …`.
    fn card(&self, action: &str, args: &[&str]) -> Output {
        let log = self.log();
        let mut all = vec!["card", action, "--log", &log];
        all.extend_from_slice(args);
        self.run(&all, "正文\n")
    }

    /// Open a card for `unit`; its number and whether it is new.
    fn open(&self, title: &str, unit: &str, key: &str) -> (u64, bool) {
        let out = self.card(
            "open",
            &["--title", title, "--unit", unit, "--key", key, "--json"],
        );
        assert!(out.status.success(), "{}", stderr(&out));
        let said: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
        (
            said["number"].as_u64().unwrap(),
            said["created"].as_bool().unwrap(),
        )
    }

    /// The unit's receipt and the outer loop's acceptance.
    fn settle(&self, n: u64, unit: &str) {
        let n = n.to_string();
        assert!(
            self.card("ack", &[&n, "done", "做完了", "--by", unit])
                .status
                .success()
        );
        assert!(self.card("accept", &[&n, "复验通过"]).status.success());
    }

    fn compact(&self, args: &[&str]) -> serde_json::Value {
        let log = self.log();
        let mut all = vec!["log", "compact", &log, "--json"];
        all.extend_from_slice(args);
        serde_json::from_str(self.ok(&all, "").trim()).unwrap()
    }

    /// The inbox's items, by number.
    fn items(&self) -> Vec<serde_json::Value> {
        let log = self.log();
        let report: serde_json::Value =
            serde_json::from_str(&self.ok(&["board", "inbox", &log, "--json", "--all"], ""))
                .unwrap();
        report["items"].as_array().unwrap().clone()
    }

    fn history(&self, args: &[&str]) -> Vec<serde_json::Value> {
        let log = self.log();
        let mut all = vec!["log", "history", &log, "--json"];
        all.extend_from_slice(args);
        self.ok(&all, "")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Make every event written so far `days` old.
    fn age(&self, days: i64) {
        let then = (chrono::Utc::now() - chrono::Duration::days(days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        for entry in std::fs::read_dir(self.log()).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("events-") {
                continue;
            }
            let aged: String = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| {
                    let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
                    event["ts"] = serde_json::json!(then);
                    event.to_string() + "\n"
                })
                .collect();
            std::fs::write(&path, aged).unwrap();
        }
    }
}

/// Scenario: 已了结的卡移入归档,其余的读起来与原来一样(critical)
#[test]
fn settled_cards_move_to_the_archive_and_the_rest_reads_the_same() {
    let hub = Hub::new();
    assert_eq!(hub.open("做完并采认的", "m07", "k1"), (1, true));
    assert_eq!(hub.open("做完还没复验的", "m07", "k2"), (2, true));
    assert_eq!(hub.open("还在做的", "m08", "k3"), (3, true));
    hub.settle(1, "m07");
    assert!(
        hub.card("ack", &["2", "done", "做完了", "--by", "m07"])
            .status
            .success()
    );
    let before = hub.items();
    let whole = hub.history(&[]);
    assert_eq!(whole.len(), 6);

    let done = hub.compact(&["--keep-days", "0", "--keep", "0"]);
    assert_eq!(done["compacted"], true, "{done}");
    assert_eq!(done["cards_archived"], serde_json::json!([1]));
    assert_eq!(
        (done["events_before"].clone(), done["events_after"].clone()),
        (6.into(), 3.into())
    );
    assert_eq!(done["upto"], 6);

    // The cards that stay are in the states they were in.
    let stay: Vec<_> = before
        .iter()
        .filter(|i| i["number"] != 1)
        .cloned()
        .collect();
    assert_eq!(hub.items(), stay);
    // Nothing was rewritten: the segment is in the archive as it was, and
    // the whole history is still there to read, numbered without a gap.
    assert!(hub.path("log/archive/events-000001.jsonl").is_file());
    assert!(!hub.path("log/events-000001.jsonl").exists());
    assert_eq!(hub.history(&[]), whole);
    let kinds: Vec<String> = hub
        .history(&["--card", "1"])
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds, ["dispatched", "ack", "accepted"]);

    // A new card takes the next number, never an archived card's.
    assert_eq!(hub.open("新的一张", "m08", "k4"), (4, true));
    let seqs: Vec<u64> = hub
        .history(&[])
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs, (1..=7).collect::<Vec<_>>());
    // Compacting again right away changes nothing.
    let again = hub.compact(&["--keep-days", "0", "--keep", "0"]);
    assert_eq!(again["compacted"], false, "{again}");
}

/// Scenario: 归档的卡保留卡号与立卡的键,之后的写入被拒收(critical)
#[test]
fn an_archived_card_keeps_its_number_and_key_and_takes_no_more_writes() {
    let hub = Hub::new();
    hub.open("做完并采认的", "m07", "k1");
    hub.open("还在做的", "m07", "k2");
    hub.settle(1, "m07");
    hub.compact(&["--keep-days", "0", "--keep", "0"]);
    let events = hub.history(&[]).len();

    // The key the card was opened with still stands for that card.
    assert_eq!(hub.open("做完并采认的", "m07", "k1"), (1, false));
    // The outer loop cannot write about it any more …
    let out = hub.card("return", &["1", "再改一下"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("已了结并归档"), "{}", stderr(&out));
    // … nor can its unit, through the hub's forced command.
    let log = hub.log();
    let out = Command::new(BIN)
        .args(["olp", "remote", "serve-log", "--unit", "m07", "--log", &log])
        .env("SSH_ORIGINAL_COMMAND", "ack 1 done")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            let _ = child
                .stdin
                .take()
                .unwrap()
                .write_all("迟到的回执\n".as_bytes());
            child.wait_with_output()
        })
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("已了结并归档"), "{}", stderr(&out));
    assert_eq!(hub.history(&[]).len(), events, "nothing was written");
}

/// Scenario: 最近了结的、还不够久的、被回测的卡都留下
#[test]
fn recent_cards_and_cards_under_retest_stay() {
    let hub = Hub::new();
    hub.open("原卡", "m07", "k1");
    hub.settle(1, "m07");
    // Settled a moment ago: neither old enough nor outside the last ten.
    let done = hub.compact(&[]);
    assert_eq!(done["compacted"], false, "{done}");
    let done = hub.compact(&["--keep-days", "0"]);
    assert_eq!(done["compacted"], false, "{done}");
    let done = hub.compact(&["--keep", "0"]);
    assert_eq!(done["compacted"], false, "{done}");
    assert!(!hub.path("log/base.jsonl").exists());

    // A re-test of #1 on another unit is open: #1 stays.
    let log = hub.log();
    hub.ok(
        &[
            "card",
            "dispatch",
            "m08",
            "--log",
            &log,
            "--title",
            "回测 #1",
            "--verify-of",
            "1",
            "--wake",
            "w1:p1",
            "--skip-health",
            "--allow-busy",
        ],
        "回测正文\n",
    );
    let done = hub.compact(&["--keep-days", "0", "--keep", "0"]);
    assert_eq!(done["cards_archived"], serde_json::json!([]), "{done}");
    // The re-test settled, but its unit never claimed it from its
    // delivery inbox: it is still there to be read, and both stay.
    hub.settle(2, "m08");
    let done = hub.compact(&["--keep-days", "0", "--keep", "0"]);
    assert_eq!(done["cards_archived"], serde_json::json!([]), "{done}");
    // Claimed: both go.
    let out = Command::new(BIN)
        .args(["olp", "remote", "serve-log", "--unit", "m08", "--log", &log])
        .env("SSH_ORIGINAL_COMMAND", "progress 2 claimed")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let done = hub.compact(&["--keep-days", "0", "--keep", "0"]);
    assert_eq!(done["cards_archived"], serde_json::json!([1, 2]), "{done}");
    assert!(hub.items().is_empty());
}

/// Scenario: 试运行与体积不够时什么都不写
#[test]
fn a_dry_run_and_a_small_log_write_nothing() {
    let hub = Hub::new();
    hub.open("做完并采认的", "m07", "k1");
    hub.settle(1, "m07");
    let done = hub.compact(&["--keep-days", "0", "--keep", "0", "--dry-run"]);
    assert_eq!(done["cards_archived"], serde_json::json!([1]), "{done}");
    assert_eq!(
        (done["compacted"].clone(), done["dry_run"].clone()),
        (false.into(), true.into())
    );
    let done = hub.compact(&["--keep-days", "0", "--keep", "0", "--if-over", "1"]);
    assert_eq!(done["compacted"], false, "{done}");
    assert!(done["skipped"].as_str().is_some(), "{done}");
    assert!(!hub.path("log/base.jsonl").exists());
    assert!(!hub.path("log/archive").exists());
    assert_eq!(hub.items().len(), 1);
}

/// Scenario: 没回答的提问不随旧事件归档
#[test]
fn an_open_question_is_not_archived_with_old_events() {
    let hub = Hub::new();
    hub.open("一张卡", "m07", "k1");
    let log = hub.log();
    let ask = |id: &str, request: &str| {
        let out = Command::new(BIN)
            .args(["olp", "remote", "serve-log", "--unit", "m07", "--log", &log])
            .env("SSH_ORIGINAL_COMMAND", format!("question {request} {id}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                let _ = child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all("用哪个版本\n".as_bytes());
                child.wait_with_output()
            })
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
    };
    ask("q-1111", "ask");
    ask("q-2222", "ask");
    ask("q-2222", "answer");
    let questions = || {
        let report: serde_json::Value =
            serde_json::from_str(&hub.ok(&["board", "inbox", &log, "--json", "--all"], ""))
                .unwrap();
        report["questions"].clone()
    };
    let open = questions();
    assert_eq!(open.as_array().unwrap().len(), 1, "{open}");

    let done = hub.compact(&["--keep-days", "0", "--keep", "0"]);
    // The answered question and its answer go; the open one and the card stay.
    assert_eq!(
        (done["events_before"].clone(), done["events_after"].clone()),
        (4.into(), 2.into())
    );
    assert_eq!(questions(), open);
}

/// Scenario: 外环值守在日志长大后自己压缩,不重复尝试
#[test]
fn the_outer_watcher_compacts_a_log_that_grew_large() {
    let hub = Hub::new();
    for n in 1..=12u64 {
        hub.open(&format!("第 {n} 张"), "m07", &format!("k{n}"));
        hub.settle(n, "m07");
    }
    hub.open("还在做的", "m08", "k13");
    hub.age(30);
    // Over a mebibyte: one more old event about no card.
    let segment = hub.path("log/events-000001.jsonl");
    let then = (chrono::Utc::now() - chrono::Duration::days(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let big = serde_json::json!({
        "seq": 38, "ts": then, "kind": "note", "actor": "外环(hub)",
        "data": { "text": "x".repeat(1_100_000) },
    });
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&segment)
        .unwrap();
    writeln!(file, "{big}").unwrap();
    drop(file);

    let herdr = hub.path("fake-herdr");
    std::fs::write(&herdr, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&herdr, std::fs::Permissions::from_mode(0o755)).unwrap();
    let log = hub.log();
    let watch = |mib: &str| {
        let out = Command::new(BIN)
            .args([
                "olp",
                "outer",
                "watch",
                &log,
                "--pane",
                "w9:p1",
                "--once",
                "--quiet",
                "999999",
                "--idle-wait",
                "0",
            ])
            .env("OLP_HERDR", &herdr)
            .env("OLP_UNITS_DIR", hub.path("units"))
            .env("OLP_OUTER_WATCH_STATE", hub.path("cursor.json"))
            .env("OLP_LOG_COMPACT_MIB", mib)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        stdout(&out)
    };
    // Turned off: the log is left alone.
    assert!(!watch("0").contains("已压缩日志"));
    assert!(!hub.path("log/base.jsonl").exists());

    let said = watch("1");
    assert!(said.contains("已压缩日志:归档 2 张卡"), "{said}");
    // The ten most recently settled and the open card are still there.
    let left: Vec<u64> = hub
        .items()
        .iter()
        .map(|i| i["number"].as_u64().unwrap())
        .collect();
    assert_eq!(left, (3..=13).collect::<Vec<_>>());
    assert_eq!(hub.history(&[]).len(), 38);
    // Small again: the next round does not try.
    assert!(!watch("1").contains("已压缩日志"));
}
