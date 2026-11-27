#![cfg(unix)]

//! Contract tests for the outer loop's watcher (SDD spec:
//! `specs/task-olp-outer-watch.spec.md`): events that wait for the outer
//! loop wake its pane with one fixed sentence, once, when it is idle.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
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
        let hub = Hub {
            dir: tempfile::tempdir().unwrap(),
        };
        // A herdr that logs what it is asked; the agent is busy while the
        // file `busy` exists.
        let script = r#"#!/bin/sh
echo "$*" >> 'DIR/herdr.log'
if [ "$1 $2" = 'agent wait' ] && [ -e 'DIR/busy' ]; then exit 1; fi
if [ "$1 $2" = 'workspace list' ]; then
  echo '{"result":{"workspaces":[{"workspace_id":"w4","label":"my-outer"}]}}'
fi
if [ "$1 $2" = 'pane list' ]; then
  echo '{"result":{"panes":[{"pane_id":"w3:p1","workspace_id":"w3"},{"pane_id":"w4:p2","workspace_id":"w4"}]}}'
fi
exit 0
"#
        .replace("DIR", &hub.dir.path().display().to_string());
        std::fs::write(hub.herdr(), script).unwrap();
        std::fs::set_permissions(hub.herdr(), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            hub.dir.path().join("unit.conf"),
            format!(
                "hub = local\nunit = m07\nstore = log\nlog = {}\n",
                hub.log()
            ),
        )
        .unwrap();
        hub
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn herdr(&self) -> std::path::PathBuf {
        self.dir.path().join("fake-herdr")
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.arg("olp")
            .args(args)
            .env("OLP_HERDR", self.herdr())
            .env("OLP_UNITS_DIR", self.dir.path().join("units"))
            .env("OLP_UNIT_CONF", self.dir.path().join("unit.conf"))
            .env("OLP_OUTER_WATCH_STATE", self.dir.path().join("cursor.json"));
        cmd
    }

    fn ok(&self, args: &[&str], stdin: &str) -> String {
        let mut child = self
            .cmd(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    }

    fn card(&self, title: &str) {
        let log = self.log();
        self.ok(
            &[
                "card",
                "dispatch",
                "m07",
                "--log",
                &log,
                "--title",
                title,
                "--wake",
                "w1:p1",
                "--skip-health",
                "--allow-busy",
            ],
            "正文:不该出现在唤醒里的细节\n",
        );
    }

    /// One round of the watcher; `quiet` is large unless a stall is wanted.
    fn watch(&self, quiet: &str) -> String {
        let log = self.log();
        self.ok(
            &[
                "outer",
                "watch",
                &log,
                "--pane",
                "w9:p1",
                "--once",
                "--quiet",
                quiet,
                "--idle-wait",
                "0",
            ],
            "",
        )
    }

    fn prompts(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("herdr.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("agent prompt"))
            .map(str::to_string)
            .collect()
    }
}

/// Scenario: 等外环处理的新事件唤醒外环一次,短句不带内容
#[test]
fn events_for_the_outer_loop_wake_its_pane_once() {
    let hub = Hub::new();
    hub.card("第一张");
    // A dispatch is the outer loop's own doing: nothing waits for it.
    assert_eq!(hub.watch("99999"), "");
    assert!(hub.prompts().is_empty());

    hub.ok(&["remote", "progress", "1", "claimed"], "");
    assert_eq!(hub.watch("99999"), "", "a claim needs no decision");
    hub.ok(
        &["remote", "ack", "1", "done", "commit abc1234,测试通过"],
        "",
    );
    let said = hub.watch("99999");
    assert!(said.contains("1 条新事件"), "{said}");
    let prompts = hub.prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    let log = hub.log();
    assert!(
        prompts[0].contains(&format!("octoscode olp inbox {log}")),
        "{prompts:?}"
    );
    for secret in ["第一张", "正文", "abc1234", "#1"] {
        assert!(
            !prompts[0].contains(secret),
            "the wake leaks {secret}: {prompts:?}"
        );
    }
    // Handed over once: the same event does not wake again.
    assert_eq!(hub.watch("99999"), "");
    assert_eq!(hub.prompts().len(), 1);
}

/// Scenario: 按工作区标签找外环的窗格;没有这个工作区时不提醒,事件留到下一轮
#[test]
fn the_watcher_finds_the_outer_pane_by_its_workspace_label() {
    let hub = Hub::new();
    hub.card("第一张");
    hub.ok(&["remote", "ack", "1", "done", "测试通过"], "");
    let log = hub.log();
    let watch = |label: &str| {
        hub.ok(
            &[
                "outer",
                "watch",
                &log,
                "--label",
                label,
                "--once",
                "--idle-wait",
                "0",
            ],
            "",
        )
    };
    assert_eq!(watch("nobody"), "");
    assert!(hub.prompts().is_empty());
    let said = watch("my-outer");
    assert!(said.contains("w4:p2"), "{said}");
    assert!(
        hub.prompts()[0].starts_with("agent prompt w4:p2 "),
        "{:?}",
        hub.prompts()
    );
    // Both or neither is a usage error.
    for extra in [&["--pane", "w1:p1", "--label", "x"][..], &[][..]] {
        let out = hub
            .cmd(&["outer", "watch", &log, "--once"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{extra:?}");
    }
}

/// Scenario: 已了结的卡、已有结局的提问不再叫醒外环
#[test]
fn what_is_already_over_does_not_wake_the_outer_loop() {
    let hub = Hub::new();
    hub.card("第一张");
    hub.ok(&["remote", "ack", "1", "done", "测试通过"], "");
    let log = hub.log();
    hub.ok(&["card", "accept", "--log", &log, "1", "已复验"], "");
    hub.ok(&["remote", "question", "ask", "q-1234", "用哪个版本"], "");
    hub.ok(&["remote", "question", "timeout", "q-1234"], "");
    // A watcher that has never run meets a log whose history is all settled.
    assert_eq!(hub.watch("99999"), "");
    assert!(hub.prompts().is_empty());
    hub.ok(&["remote", "question", "ask", "q-5678", "第二问"], "");
    assert!(hub.watch("99999").contains("1 条新事件"));
}

/// Scenario: 外环在忙时不打扰,它空闲后补上,忙的时候到的事件不会被跳过
#[test]
fn a_busy_outer_loop_is_told_when_it_is_idle_again() {
    let hub = Hub::new();
    hub.card("第一张");
    hub.card("第二张");
    std::fs::write(hub.dir.path().join("busy"), "").unwrap();
    hub.ok(
        &[
            "remote",
            "progress",
            "1",
            "undelivered",
            "窗格 w1:p1:agent 状态 blocked",
        ],
        "",
    );
    let said = hub.watch("99999");
    assert!(said.contains("下一轮再试"), "{said}");
    assert!(hub.prompts().is_empty());
    // More arrives while it is busy.
    hub.ok(&["remote", "ack", "2", "blocked", "缺少凭据"], "");
    assert!(hub.prompts().is_empty());
    std::fs::remove_file(hub.dir.path().join("busy")).unwrap();
    let said = hub.watch("99999");
    assert!(said.contains("2 条新事件"), "both are handed over: {said}");
    assert_eq!(hub.prompts().len(), 1);
    assert_eq!(hub.watch("99999"), "");
}

/// Scenario: 外环忙着时值守不占用静默的报告次数
#[test]
fn a_busy_outer_pane_does_not_use_up_the_stall_reports() {
    let hub = Hub::new();
    hub.card("没人领的卡");
    let busy = hub.dir.path().join("busy");
    std::fs::write(&busy, "").unwrap();
    // Many rounds against a busy pane: more than the three reports a
    // stall gets.
    for _ in 0..6 {
        let said = hub.watch("0");
        assert!(said.contains("下一轮再试"), "{said}");
    }
    assert!(hub.prompts().is_empty());
    // The pane is free: the stall is still said …
    std::fs::remove_file(&busy).unwrap();
    let said = hub.watch("0");
    assert!(said.contains("1 条静默提醒"), "{said}");
    assert_eq!(hub.prompts().len(), 1);
    // … and the outer loop's own look still names the card: the watcher
    // spent none of its reports.
    let log = hub.log();
    let seen: serde_json::Value =
        serde_json::from_str(&hub.ok(&["board", "stall", &log, "--json", "--quiet", "0"], ""))
            .unwrap();
    assert_eq!(seen["alerts"][0]["number"], 1, "{seen}");
    assert_eq!(seen["alerts"][0]["reports"], 1, "{seen}");
    // The outer loop has looked: the watcher does not say it again.
    assert_eq!(hub.watch("0"), "");
    assert_eq!(hub.prompts().len(), 1);
}

/// Scenario: 整体静默也唤醒外环,同一段沉默不反复唤醒
#[test]
fn a_stall_wakes_the_outer_loop_and_is_not_repeated() {
    let hub = Hub::new();
    hub.card("没人领的卡");
    assert_eq!(hub.watch("99999"), "", "not silent for long enough");
    let said = hub.watch("0");
    assert!(said.contains("1 条静默提醒"), "{said}");
    assert_eq!(hub.prompts().len(), 1);
    assert!(hub.prompts()[0].contains("olp board stall"));
    // The same silence, the next two rounds: nothing.
    assert_eq!(hub.watch("0"), "");
    assert_eq!(hub.watch("0"), "");
    assert_eq!(hub.prompts().len(), 1);

    // Usage.
    let log = hub.log();
    for args in [
        vec!["outer", "watch", log.as_str()],
        vec!["outer", "watch", "--pane", "w9:p1"],
        vec!["outer", "watch", "/no/such/log", "--pane", "w9:p1"],
        vec!["outer", "watch", log.as_str(), "--pane", "bad pane"],
    ] {
        let out = hub.cmd(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}
