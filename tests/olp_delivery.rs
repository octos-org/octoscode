#![cfg(unix)]

//! Contract tests for delivery and wake on the event log (SDD spec:
//! `specs/task-olp-delivery-wake.spec.md`): a dispatched card waits in its
//! unit's inbox until claimed, and the unit's watcher wakes the pane with
//! one fixed sentence.

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

struct Site {
    dir: tempfile::TempDir,
}

impl Site {
    fn new() -> Self {
        let site = Site {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(
            site.dir.path().join("unit.conf"),
            format!(
                "hub = local\nunit = m07\nstore = log\nlog = {}\n",
                site.log()
            ),
        )
        .unwrap();
        // A herdr that logs what it is asked. The agent is idle unless the
        // file `busy` (working) or `blocked` exists; after a prompt it
        // starts working unless the file `stuck` exists.
        let herdr = r#"#!/bin/sh
echo "$*" >> 'DIR/herdr.log'
if [ "$1 $2" = 'agent get' ]; then
  s=idle
  [ -e 'DIR/busy' ] && s=working
  [ -e 'DIR/blocked' ] && s=blocked
  echo "{\"result\":{\"agent\":{\"agent_status\":\"$s\"}}}"
  exit 0
fi
if [ "$1 $2" = 'agent wait' ]; then
  if [ -e 'DIR/busy' ] || [ -e 'DIR/blocked' ]; then exit 1; fi
fi
if [ "$1 $2" = 'agent prompt' ]; then
  if [ -e 'DIR/blocked' ]; then echo '{"error":{"code":"agent_blocked"}}' >&2; exit 1; fi
  if [ -e 'DIR/stuck' ]; then echo '{"error":{"code":"agent_prompt_stalled"}}' >&2; exit 1; fi
fi
exit 0
"#
        .replace("DIR", &site.dir.path().display().to_string());
        std::fs::write(site.herdr(), herdr).unwrap();
        std::fs::set_permissions(site.herdr(), std::fs::Permissions::from_mode(0o755)).unwrap();
        site
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn herdr(&self) -> PathBuf {
        self.dir.path().join("fake-herdr")
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

    /// The outer loop opens a card; returns the exit code and its number.
    fn open(&self, title: &str, extra: &[&str]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.args([
            "olp",
            "card",
            "open",
            "--log",
            &self.log(),
            "--title",
            title,
        ])
        .args(extra);
        self.run(cmd, "卡的正文:机密细节不该出现在唤醒里\n")
    }

    fn outer(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.args(["olp", "card"])
            .args(args)
            .args(["--log", &self.log()]);
        self.run(cmd, "")
    }

    fn unit_as(&self, unit: &str, args: &[&str]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.args(["olp", "remote"])
            .args(args)
            .env("OLP_UNIT_CONF", self.dir.path().join("unit.conf"))
            .env("OLP_UNIT_ID", unit)
            .env("OLP_HERDR", self.herdr())
            .env("OLP_WATCH_STATE", self.dir.path().join("watch.json"))
            .env("HOME", self.dir.path());
        self.run(cmd, "")
    }

    fn unit(&self, args: &[&str]) -> Output {
        self.unit_as("m07", args)
    }

    fn waiting(&self, unit: &str, args: &[&str]) -> Vec<u64> {
        let mut full = vec!["inbox"];
        full.extend_from_slice(args);
        let out = self.unit_as(unit, &full);
        assert!(out.status.success(), "{}", stderr(&out));
        let report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert_eq!(report["unit"], unit);
        report["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["number"].as_u64().unwrap())
            .collect()
    }

    fn herdr_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("herdr.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn prompts(&self) -> Vec<String> {
        self.herdr_calls()
            .into_iter()
            .filter(|call| call.starts_with("agent prompt"))
            .collect()
    }
}

/// Scenario: 派出的卡在单元的收件箱里等到被领取(critical)
#[test]
fn a_dispatched_card_waits_in_the_units_inbox_until_claimed() {
    let site = Site::new();
    assert!(
        site.open(
            "给 m07 窗格 p1",
            &[
                "--unit",
                "m07",
                "--pane",
                "w1:p1",
                "--impl",
                "octoscode",
                "--review",
                "codex"
            ]
        )
        .status
        .success()
    );
    assert!(
        site.open("给 m07 窗格 p2", &["--unit", "m07", "--pane", "w1:p2"])
            .status
            .success()
    );
    assert!(
        site.open("给 m08", &["--unit", "m08", "--pane", "w1:p1"])
            .status
            .success()
    );
    assert!(
        site.open("没有窗格的卡", &["--unit", "m07"])
            .status
            .success()
    );

    assert_eq!(site.waiting("m07", &[]), [1, 2]);
    assert_eq!(site.waiting("m07", &["w1:p2"]), [2]);
    assert_eq!(site.waiting("m08", &[]), [3]);
    assert!(site.waiting("m09", &[]).is_empty());
    let out = site.unit(&["inbox", "w1:p1"]);
    let report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        report["items"][0],
        serde_json::json!({
            "number": 1,
            "title": "给 m07 窗格 p1",
            "pane": "w1:p1",
            "impl": "octoscode",
            "review": ["codex"],
            "body": "卡的正文:机密细节不该出现在唤醒里",
            "next": {
                "claim": "octoscode olp remote progress 1 claimed",
                "ack": "octoscode olp remote ack 1 done|wontdo|blocked <说明>"
            }
        })
    );
    // The answer says what to do with a card, so a woken agent need not guess.
    assert!(report["how"].as_str().unwrap().contains("next.claim"));
    // The commands it names are real: the claim below is `next.claim`.
    let empty: serde_json::Value =
        serde_json::from_str(&stdout(&site.unit_as("m09", &["inbox"]))).unwrap();
    assert!(empty.get("how").is_none());

    // Claiming is the `claimed` progress node; claiming twice is the same claim.
    for _ in 0..2 {
        assert!(site.unit(&["progress", "1", "claimed"]).status.success());
    }
    assert_eq!(site.waiting("m07", &[]), [2]);
    // A voided card stops waiting.
    assert!(site.outer(&["void", "2"]).status.success());
    assert!(site.waiting("m07", &[]).is_empty());

    assert_eq!(
        site.open("窗格没有单元", &["--pane", "w1:p1"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        site.open("窗格写错", &["--unit", "m07", "--pane", "w1 p1"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(site.unit(&["inbox", "bad pane"]).status.code(), Some(3));
}

/// Scenario: 值守在 agent 空闲时发一句固定短句,不带卡的内容,不重复提醒(critical)
#[test]
fn the_watcher_wakes_an_idle_pane_with_one_fixed_sentence() {
    let site = Site::new();
    // Nothing waits: herdr is not touched.
    assert!(site.unit(&["watch", "w1:p1", "--once"]).status.success());
    assert!(site.herdr_calls().is_empty());

    assert!(
        site.open("第一张", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    assert!(
        site.open("第二张", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    let out = site.unit(&["watch", "w1:p1", "--once"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        site.herdr_calls(),
        [
            "agent wait w1:p1 --until idle --until done --timeout 5000",
            "agent prompt w1:p1 有新派卡,请执行:octoscode olp remote inbox w1:p1,再按输出里的 next 领取、办理、回执 --wait --until working --until blocked --timeout 10000",
        ],
        "one sentence for both cards, and nothing of theirs in it"
    );

    // Already nudged: the next rounds leave the pane alone.
    for _ in 0..2 {
        assert!(site.unit(&["watch", "w1:p1", "--once"]).status.success());
    }
    assert_eq!(site.prompts().len(), 1);
    // A new card is a new reason.
    assert!(
        site.open("第三张", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    assert!(site.unit(&["watch", "w1:p1", "--once"]).status.success());
    assert_eq!(site.prompts().len(), 2);
    // Still unclaimed after the re-nudge time: reminded again.
    assert!(
        site.unit(&["watch", "w1:p1", "--once", "--renudge", "0"])
            .status
            .success()
    );
    assert_eq!(site.prompts().len(), 3);
    // Claimed: nothing left to say, even with re-nudging due.
    for n in ["1", "2", "3"] {
        assert!(site.unit(&["progress", n, "claimed"]).status.success());
    }
    assert!(
        site.unit(&["watch", "w1:p1", "--once", "--renudge", "0"])
            .status
            .success()
    );
    assert_eq!(site.prompts().len(), 3);
}

/// Scenario: agent 正忙时不打扰,空闲后再提醒
#[test]
fn a_busy_agent_is_left_alone_until_it_is_idle() {
    let site = Site::new();
    assert!(
        site.open("忙的时候派来的", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    std::fs::write(site.dir.path().join("busy"), "").unwrap();
    for _ in 0..2 {
        assert!(
            site.unit(&["watch", "w1:p1", "--once", "--idle-wait", "1"])
                .status
                .success()
        );
    }
    assert!(site.prompts().is_empty(), "{:?}", site.herdr_calls());
    std::fs::remove_file(site.dir.path().join("busy")).unwrap();
    assert!(site.unit(&["watch", "w1:p1", "--once"]).status.success());
    assert_eq!(site.prompts().len(), 1);
    // Another pane's cards are not this watcher's business.
    assert!(site.unit(&["watch", "w1:p9", "--once"]).status.success());
    assert_eq!(site.prompts().len(), 1);
    assert_eq!(site.unit(&["watch"]).status.code(), Some(2));
    assert_eq!(
        site.unit(&["watch", "w1:p1", "--interval", "x"])
            .status
            .code(),
        Some(2)
    );
}

/// Scenario: 发了话而 agent 没有开始工作的,不算提醒过
#[test]
fn a_nudge_the_agent_did_not_pick_up_is_tried_again() {
    let site = Site::new();
    assert!(
        site.open("被挡住时派来的", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    // herdr calls the agent idle, takes the prompt, and the agent never starts.
    std::fs::write(site.dir.path().join("stuck"), "").unwrap();
    let out = site.unit(&["watch", "w1:p1", "--once"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(site.prompts().len(), 1);
    assert!(stdout(&out).contains("没有开始工作"), "{}", stdout(&out));
    // Not due yet: the pane is not typed into every round.
    assert!(site.unit(&["watch", "w1:p1", "--once"]).status.success());
    assert_eq!(site.prompts().len(), 1);
    // After the retry time it is said once more, long before the re-nudge time.
    let out = site.unit(&["watch", "w1:p1", "--once", "--retry", "0"]);
    assert_eq!(site.prompts().len(), 2);
    assert!(stdout(&out).contains("不再追着发"), "{}", stdout(&out));
    // Twice in a row: the pane is not taking input. Typing again would only
    // pile sentences up in its prompt, so the retry time no longer applies.
    assert!(
        site.unit(&["watch", "w1:p1", "--once", "--retry", "0"])
            .status
            .success()
    );
    assert_eq!(site.prompts().len(), 2);
    // The dialog is gone: at the re-nudge time it is picked up, and counts.
    std::fs::remove_file(site.dir.path().join("stuck")).unwrap();
    let out = site.unit(&["watch", "w1:p1", "--once", "--renudge", "0"]);
    assert_eq!(site.prompts().len(), 3);
    assert!(stdout(&out).contains("已提醒窗格"), "{}", stdout(&out));
    assert!(
        site.unit(&["watch", "w1:p1", "--once", "--retry", "0"])
            .status
            .success()
    );
    assert_eq!(site.prompts().len(), 3);
}

/// Scenario: 窗格接不了卡时不打字,并让 hub 知道(critical)
#[test]
fn a_pane_that_cannot_take_a_card_is_reported_to_the_hub() {
    let site = Site::new();
    assert!(
        site.open("派给被挡住的窗格", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    let hub_inbox = |what: &str| {
        let mut cmd = Command::new(BIN);
        cmd.args(["olp", "inbox", what]);
        stdout(&site.run(cmd, ""))
    };
    assert!(hub_inbox(&site.log()).contains("已派出,等内环 ACK"));

    // A dialog or an approval covers the agent's prompt: herdr says blocked.
    std::fs::write(site.dir.path().join("blocked"), "").unwrap();
    let out = site.unit(&["watch", "w1:p1", "--once"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        site.prompts().is_empty(),
        "nothing is typed into a blocked pane"
    );
    assert!(
        stdout(&out).contains("agent 状态:blocked"),
        "{}",
        stdout(&out)
    );
    // Not long enough to bother the hub yet.
    assert!(hub_inbox(&site.log()).contains("已派出,等内环 ACK"));

    // Still blocked after the retry time: the hub is told, once.
    for _ in 0..2 {
        let out = site.unit(&["watch", "w1:p1", "--once", "--retry", "0"]);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    assert!(site.prompts().is_empty());
    let seen = hub_inbox(&site.log());
    assert!(seen.contains("送达失败"), "{seen}");
    assert!(
        seen.contains("== 收件箱(1)"),
        "the outer loop has to look:\n{seen}"
    );
    let events = std::fs::read_to_string(format!("{}/events-000001.jsonl", site.log())).unwrap();
    assert_eq!(events.matches("\"undelivered\"").count(), 1, "{events}");
    // The rendered board says the same as the log.
    let board = site.dir.path().join("board.md");
    let mut render = Command::new(BIN);
    render.args(["olp", "board", "render", &site.log()]);
    std::fs::write(&board, site.run(render, "").stdout).unwrap();
    assert_eq!(hub_inbox(&board.to_string_lossy()), seen);

    // Unblocked: the next round wakes it, and a claim clears the report.
    std::fs::remove_file(site.dir.path().join("blocked")).unwrap();
    assert!(site.unit(&["watch", "w1:p1", "--once"]).status.success());
    assert_eq!(site.prompts().len(), 1);
    assert!(site.unit(&["progress", "1", "claimed"]).status.success());
    let seen = hub_inbox(&site.log());
    assert!(!seen.contains("送达失败"), "{seen}");
    assert!(seen.contains("已派出,等内环 ACK"), "{seen}");
}

/// Scenario: agent 每次都开始工作、卡却一直没领取的,也让 hub 知道
#[test]
fn a_card_the_agent_keeps_starting_on_but_never_claims_is_reported() {
    let site = Site::new();
    assert!(
        site.open("领不走的卡", &["--unit", "m07", "--pane", "w1:p1"])
            .status
            .success()
    );
    let hub_inbox = |what: &str| {
        let mut cmd = Command::new(BIN);
        cmd.args(["olp", "inbox", what]);
        stdout(&site.run(cmd, ""))
    };
    // Two nudges the agent started working on; the card stays unclaimed.
    for expected in 1..=2 {
        let out = site.unit(&["watch", "w1:p1", "--once", "--renudge", "0"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(site.prompts().len(), expected);
        assert!(!stdout(&out).contains("仍未领取"), "{}", stdout(&out));
    }
    assert!(hub_inbox(&site.log()).contains("已派出,等内环 ACK"));
    // Not yet time for another nudge: nothing is reported early.
    let out = site.unit(&["watch", "w1:p1", "--once"]);
    assert!(!stdout(&out).contains("仍未领取"), "{}", stdout(&out));
    // Due again and still unclaimed: the hub is told, once.
    for _ in 0..2 {
        let out = site.unit(&["watch", "w1:p1", "--once", "--renudge", "0"]);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    let events = std::fs::read_to_string(format!("{}/events-000001.jsonl", site.log())).unwrap();
    assert_eq!(events.matches("\"undelivered\"").count(), 1, "{events}");
    assert!(events.contains("次开始工作,卡仍未领取"), "{events}");
    let seen = hub_inbox(&site.log());
    assert!(seen.contains("送达失败"), "{seen}");
    // Claimed at last: the report is cleared.
    assert!(site.unit(&["progress", "1", "claimed"]).status.success());
    assert!(!hub_inbox(&site.log()).contains("送达失败"));
}

/// Scenario: `olp remote --help` 列出本程序自己的动作
#[test]
fn remote_help_lists_the_actions_this_binary_has() {
    let out = Command::new(BIN)
        .args(["olp", "remote", "--help"])
        .output()
        .unwrap();
    let text = stdout(&out);
    assert!(out.status.success(), "{}", stderr(&out));
    for action in ["ack", "mirror", "progress", "inbox", "watch", "serve-log"] {
        assert!(
            text.lines()
                .any(|l| l.trim_start().starts_with(&format!("{action} "))),
            "{action} missing from:\n{text}"
        );
    }
    // The actions a script still handles keep their own help below.
    assert!(text.contains("flush"), "{text}");
}
