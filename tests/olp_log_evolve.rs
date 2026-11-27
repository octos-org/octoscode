#![cfg(unix)]

//! Contract tests for `octoscode olp log symptoms` and `octoscode olp tune`
//! (SDD spec: `specs/task-olp-log-evolve.spec.md`): where delivery got
//! worse is said with the cards that show it, the thresholds live in the
//! log, and an automatic change is tried one at a time and undone when its
//! hedge gets worse.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");
/// Day 28 after the first synthetic event: the windows are days 14–20 and 21–27.
const NOW: &str = "2026-11-30T00:00:00Z";
const DAY: i64 = 24 * 60;

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

struct Hub {
    dir: tempfile::TempDir,
    card: u64,
}

impl Hub {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("log")).unwrap();
        Hub { dir, card: 0 }
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("log")
    }

    fn dir(&self) -> String {
        self.log().to_string_lossy().into_owned()
    }

    /// Append one event at `ts`, numbered after whatever the log holds.
    fn write(&self, ts: &str, kind: &str, card: Option<u64>, unit: Option<&str>, data: Value) {
        let path = self.log().join("events-000001.jsonl");
        let seq = std::fs::read_to_string(&path).map_or(0, |t| t.lines().count()) + 1;
        let mut event = json!({"seq": seq, "ts": ts, "kind": kind, "actor": "测试", "data": data});
        if let Some(card) = card {
            event["card"] = json!(card);
        }
        if let Some(unit) = unit {
            event["unit"] = json!(unit);
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{event}").unwrap();
    }

    /// `minute` minutes after `origin`.
    fn at(origin: &str, minute: i64) -> String {
        (chrono::DateTime::parse_from_rfc3339(origin).unwrap() + chrono::Duration::minutes(minute))
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// Three cards dispatched one after another from `start`, each answered
    /// five minutes later and all accepted `wait` minutes after the last
    /// receipt: the third goes out while two wait for the outer loop.
    fn batch(&mut self, origin: &str, start: i64, wait: i64) -> Vec<u64> {
        let first = self.card + 1;
        for i in 0..3 {
            self.card += 1;
            let c = Some(self.card);
            let t = start + i * 6;
            self.write(
                &Self::at(origin, t),
                "dispatched",
                c,
                Some("m07"),
                json!({"title": format!("卡 {}", self.card), "pane": "w1:1"}),
            );
            self.write(
                &Self::at(origin, t + 1),
                "progress",
                c,
                Some("m07"),
                json!({"node": "claimed"}),
            );
            self.write(
                &Self::at(origin, t + 5),
                "ack",
                c,
                Some("m07"),
                json!({"result": "done"}),
            );
        }
        for card in first..=self.card {
            self.write(
                &Self::at(origin, start + 17 + wait),
                "accepted",
                Some(card),
                None,
                json!({}),
            );
        }
        (first..=self.card).collect()
    }

    /// A history whose two latest windows wait far longer for the outer
    /// loop than the baseline did.
    fn slowed() -> Self {
        let mut hub = Hub::new();
        let origin = "2026-11-02T00:00:00Z";
        hub.batch(origin, 0, 2);
        hub.batch(origin, DAY, 2);
        hub.batch(origin, 15 * DAY, 60);
        hub.batch(origin, 22 * DAY, 90);
        hub
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .arg("olp")
            .args(args)
            .env("OLP_UNITS_DIR", self.dir.path().join("units"))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(out.status.success(), "{args:?}: {}", said(&out));
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", said(&out)))
    }

    fn knob(&self, name: &str) -> Value {
        let list = self.json(&["tune", "list", &self.dir(), "--json"]);
        list["knobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["name"] == name)
            .unwrap()
            .clone()
    }

    fn kinds(&self, kind: &str) -> Vec<Value> {
        let out = self.run(&["log", "history", &self.dir(), "--json"]);
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|e| e["kind"] == kind)
            .collect()
    }
}

/// Scenario: 连续两个窗口都变差才算症状,带着说明它的卡,记进日志只记一次(critical)
#[test]
fn a_symptom_names_its_cards_and_is_recorded_once() {
    let hub = Hub::slowed();
    let dir = hub.dir();
    let found = hub.json(&["log", "symptoms", &dir, "--now", NOW, "--json"]);
    let wait = found["symptoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["indicator"] == "outer_wait")
        .unwrap_or_else(|| panic!("{found}"));
    assert_eq!(wait["window_start"], "2026-11-23T00:00:00.000Z");
    // The latest batch waited longest; within a batch the first card did.
    assert_eq!(wait["cards"], json!([10, 11, 12, 7, 8]));
    assert!(
        wait["latest"].as_f64() > wait["earlier"].as_f64()
            && wait["earlier"].as_f64() > wait["baseline"].as_f64()
    );
    assert!(hub.kinds("symptom").is_empty(), "looking writes nothing");

    // One window back, only one window is worse: not a symptom.
    let week_ago = hub.json(&[
        "log",
        "symptoms",
        &dir,
        "--now",
        "2026-11-23T00:00:00Z",
        "--json",
    ]);
    assert_eq!(week_ago["symptoms"], json!([]), "{week_ago}");
    // More cards asked for than the two windows hold.
    let strict = hub.json(&[
        "log",
        "symptoms",
        &dir,
        "--now",
        NOW,
        "--min-cards",
        "7",
        "--json",
    ]);
    assert_eq!(strict["symptoms"], json!([]));

    let first = hub.json(&["log", "symptoms", &dir, "--now", NOW, "--record", "--json"]);
    let recorded = first["recorded"].as_array().unwrap().len();
    assert!(
        recorded >= 1
            && first["recorded"]
                .as_array()
                .unwrap()
                .contains(&json!("symptom:outer_wait:2026-11-23")),
        "{first}"
    );
    let again = hub.json(&["log", "symptoms", &dir, "--now", NOW, "--record", "--json"]);
    assert_eq!(again["recorded"], json!([]));
    let events = hub.kinds("symptom");
    assert_eq!(events.len(), recorded);
    assert!(
        events
            .iter()
            .any(|e| e["data"]["cards"] == json!([10, 11, 12, 7, 8]))
    );
}

/// Scenario: 旋钮的值是日志里最后一次改动,有界、要写原因、可撤回(critical)
#[test]
fn a_knob_is_set_with_a_reason_within_bounds_and_reverted() {
    let hub = Hub::new();
    let dir = hub.dir();
    assert_eq!(hub.knob("follow_up_after")["value"], 900);
    assert_eq!(hub.knob("follow_up_after")["changed"], Value::Null);

    let out = hub.run(&[
        "tune",
        "set",
        &dir,
        "follow_up_after",
        "300",
        "--reason",
        "主持停得久",
    ]);
    assert!(out.status.success(), "{}", said(&out));
    let knob = hub.knob("follow_up_after");
    assert_eq!(knob["value"], 300);
    assert_eq!(knob["changed"]["reason"], "主持停得久");
    assert_eq!(knob["changed"]["source"], "manual");

    // The same value again writes nothing.
    let out = hub.run(&[
        "tune",
        "set",
        &dir,
        "follow_up_after",
        "300",
        "--reason",
        "再来一次",
    ]);
    assert!(out.status.success());
    assert_eq!(hub.kinds("tuned").len(), 1);

    for (args, code) in [
        (
            vec![
                "tune",
                "set",
                &dir,
                "follow_up_after",
                "5",
                "--reason",
                "太小",
            ],
            1,
        ),
        (
            vec!["tune", "set", &dir, "no_such_knob", "5", "--reason", "x"],
            1,
        ),
        (vec!["tune", "set", &dir, "follow_up_after", "600"], 2),
        (vec!["tune", "revert", &dir, "review_backlog_max"], 1),
        (vec!["tune", "frobnicate", &dir], 2),
    ] {
        let out = hub.run(&args);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {}", said(&out));
    }
    assert_eq!(
        hub.kinds("tuned").len(),
        1,
        "a refused change writes nothing"
    );

    let out = hub.run(&["tune", "revert", &dir, "follow_up_after"]);
    assert!(out.status.success(), "{}", said(&out));
    assert_eq!(hub.knob("follow_up_after")["value"], 900);
    assert_eq!(hub.kinds("tuned")[1]["data"]["reverts"], 1);

    // The unit's watcher is told the hub's value with its inbox.
    hub.run(&[
        "tune",
        "set",
        &dir,
        "follow_up_after",
        "240",
        "--reason",
        "试",
    ]);
    let out = Command::new(BIN)
        .args(["olp", "remote", "serve-log", "--unit", "m07", "--log", &dir])
        .env("SSH_ORIGINAL_COMMAND", "inbox")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let inbox: Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{}", said(&out)));
    assert_eq!(inbox["tuning"]["follow_up_after"], 240);
}

/// Scenario: 等外环裁决的卡到了上限就不再派新卡(critical)
#[test]
fn dispatch_stops_at_the_backlog_limit() {
    let hub = Hub::new();
    let dir = hub.dir();
    let dispatch = |unit: &str, title: &str, more: &[&str]| {
        let mut child = Command::new(BIN)
            .args([
                "olp",
                "card",
                "dispatch",
                unit,
                "--log",
                &dir,
                "--title",
                title,
                "--skip-health",
            ])
            .args(more)
            .env("OLP_UNITS_DIR", hub.dir.path().join("units"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all("正文\n".as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    assert!(dispatch("m07", "第一张", &[]).status.success());
    let out = hub.run(&[
        "card",
        "ack",
        "--log",
        &dir,
        "1",
        "done",
        "做完了",
        "--by",
        "m07",
    ]);
    assert!(out.status.success(), "{}", said(&out));
    // No limit by default.
    assert!(dispatch("m08", "第二张", &[]).status.success());

    let out = hub.run(&[
        "tune",
        "set",
        &dir,
        "review_backlog_max",
        "1",
        "--reason",
        "外环忙不过来",
    ]);
    assert!(out.status.success(), "{}", said(&out));
    let out = dispatch("m09", "第三张", &[]);
    assert_eq!(out.status.code(), Some(1), "{}", said(&out));
    assert!(
        said(&out).contains("review_backlog_max = 1") && said(&out).contains("--allow-backlog"),
        "{}",
        said(&out)
    );
    assert_eq!(hub.kinds("dispatched").len(), 2);

    assert!(
        dispatch("m09", "第三张", &["--allow-backlog"])
            .status
            .success()
    );
    // The outer loop rules: there is room again.
    let out = hub.run(&["card", "accept", "--log", &dir, "1", "复验通过"]);
    assert!(out.status.success(), "{}", said(&out));
    assert!(dispatch("m10", "第四张", &[]).status.success());

    // What the limit would have done to this history.
    let replay = hub.json(&["tune", "replay", &dir, "review_backlog_max", "1", "--json"]);
    assert_eq!(replay["dispatches"], 4);
    assert_eq!(replay["cards"], json!([2, 3]), "{replay}");
}

/// Scenario: 自动调参一次只试一处,对冲指标没变差就保留(critical)
#[test]
fn an_automatic_change_is_tried_alone_and_kept_when_its_hedge_holds() {
    let mut hub = Hub::slowed();
    let dir = hub.dir();
    // Looking decides and writes nothing.
    let plan = hub.json(&["tune", "auto", &dir, "--now", NOW, "--json"]);
    assert_eq!(plan["decision"]["action"], "try", "{plan}");
    assert_eq!(plan["decision"]["change"]["name"], "review_backlog_max");
    // Cards went out into a backlog of two: start one under that.
    assert_eq!(plan["decision"]["change"]["value"], 1);
    assert!(hub.kinds("tuned").is_empty());
    assert_eq!(hub.knob("review_backlog_max")["value"], 0);

    let done = hub.json(&["tune", "auto", &dir, "--now", NOW, "--apply", "--json"]);
    assert_eq!(done["decision"]["action"], "try");
    let knob = hub.knob("review_backlog_max");
    assert_eq!(
        (
            knob["value"].as_u64(),
            knob["changed"]["on_trial"].as_bool()
        ),
        (Some(1), Some(true))
    );
    let trial = &hub.kinds("tuned")[0];
    assert_eq!(trial["data"]["symptom"], "symptom:outer_wait:2026-11-23");
    assert_eq!(trial["data"]["hedge"], "total");

    // On trial: nothing else is tried, and too few cards to judge.
    let wait = hub.json(&["tune", "auto", &dir, "--now", NOW, "--apply", "--json"]);
    assert_eq!(wait["decision"]["action"], "waiting", "{wait}");
    assert_eq!(hub.kinds("tuned").len(), 1);

    // Six cards after the change, no slower end to end than before it.
    let later = "2099-01-01T00:00:00Z";
    hub.batch(later, 0, 2);
    hub.batch(later, 60, 2);
    let judged = hub.json(&[
        "tune",
        "auto",
        &dir,
        "--now",
        "2099-01-02T00:00:00Z",
        "--apply",
        "--json",
    ]);
    assert_eq!(judged["decision"]["action"], "keep", "{judged}");
    let knob = hub.knob("review_backlog_max");
    assert_eq!(
        (
            knob["value"].as_u64(),
            knob["changed"]["on_trial"].as_bool()
        ),
        (Some(1), Some(false))
    );
    assert_eq!(hub.kinds("tuned")[1]["data"]["verdict"], "kept");
}

/// Scenario: 对冲指标变差,自动改动自动撤回(critical)
#[test]
fn an_automatic_change_is_undone_when_its_hedge_gets_worse() {
    let mut hub = Hub::slowed();
    let dir = hub.dir();
    hub.json(&["tune", "auto", &dir, "--now", NOW, "--apply", "--json"]);
    assert_eq!(hub.knob("review_backlog_max")["value"], 1);
    // After the change every card takes several times as long end to end.
    let later = "2099-01-01T00:00:00Z";
    hub.batch(later, 0, 600);
    hub.batch(later, DAY, 600);
    let judged = hub.json(&[
        "tune",
        "auto",
        &dir,
        "--now",
        "2099-01-03T00:00:00Z",
        "--apply",
        "--json",
    ]);
    assert_eq!(judged["decision"]["action"], "revert", "{judged}");
    assert_eq!(hub.knob("review_backlog_max")["value"], 0);
    let events = hub.kinds("tuned");
    assert_eq!(events[1]["data"]["verdict"], "reverted");
    assert_eq!(events[1]["data"]["judges"], events[0]["seq"]);

    // A person's change of the same knob ends a trial too.
    let mut hub = Hub::slowed();
    let dir = hub.dir();
    hub.json(&["tune", "auto", &dir, "--now", NOW, "--apply", "--json"]);
    let out = hub.run(&[
        "tune",
        "set",
        &dir,
        "review_backlog_max",
        "4",
        "--reason",
        "我来定",
    ]);
    assert!(out.status.success(), "{}", said(&out));
    hub.batch(later, 0, 600);
    hub.batch(later, DAY, 600);
    assert_eq!(hub.knob("review_backlog_max")["changed"]["on_trial"], false);
    assert_eq!(hub.knob("review_backlog_max")["value"], 4);
}

/// Scenario: 压缩不丢旋钮的值
#[test]
fn a_compaction_keeps_the_knobs() {
    let mut hub = Hub::new();
    let dir = hub.dir();
    let origin = "2026-10-01T00:00:00Z";
    hub.write(&Hub::at(origin, 0), "tuned", None, None, json!({"name": "follow_up_after", "value": 420, "previous": 900, "source": "manual", "reason": "旧的改动"}));
    hub.batch(origin, 10, 2);
    assert_eq!(hub.knob("follow_up_after")["value"], 420);
    let done = hub.json(&[
        "log",
        "compact",
        &dir,
        "--keep-days",
        "1",
        "--keep",
        "0",
        "--json",
    ]);
    assert_eq!(done["compacted"], true, "{done}");
    assert_eq!(done["cards_archived"], json!([1, 2, 3]));
    // Read as every command reads the log: from the base.
    let out = Command::new(BIN)
        .args(["olp", "remote", "serve-log", "--unit", "m07", "--log", &dir])
        .env("SSH_ORIGINAL_COMMAND", "inbox")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let inbox: Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{}", said(&out)));
    assert_eq!(inbox["tuning"]["follow_up_after"], 420);
}
