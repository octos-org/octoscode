#![cfg(unix)]

//! Contract tests for `octoscode olp unit host` (SDD spec:
//! `specs/task-olp-unit-host.spec.md`): the unit's host agent is started
//! again in its herdr pane when the pane has no program in it, and is left
//! alone otherwise. herdr is a fake (`OLP_HERDR`).

#[path = "support/olp_env.rs"]
mod olp_env;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

/// herdr as far as `olp unit host` uses it. Files next to it: `busy` (the
/// pane's foreground programs besides the shell), `dies` (a started host
/// exits at once), `run.log` (commands `pane run` was given), `prompts.log`.
const FAKE_HERDR: &str = r#"#!/bin/sh
here=$(dirname "$0")
if [ "$1" = "--session" ]; then shift 2; fi
case "$1 $2" in
  "workspace list")
    if [ -e "$here/workspace" ]; then
      echo '{"result":{"workspaces":[{"workspace_id":"w7","label":"unit-host"}]}}'
    else
      echo '{"result":{"workspaces":[]}}'
    fi ;;
  "workspace create")
    : > "$here/workspace"
    echo '{"result":{"root_pane":{"pane_id":"w7:p1","workspace_id":"w7"}}}' ;;
  "pane list")
    echo '{"result":{"panes":[{"pane_id":"w7:p1","workspace_id":"w7"}]}}' ;;
  "pane process-info")
    procs='{"name":"zsh","pid":1}'
    for name in $(cat "$here/busy" 2>/dev/null); do procs="$procs,{\"name\":\"$name\",\"pid\":2}"; done
    echo "{\"result\":{\"process_info\":{\"foreground_processes\":[$procs]}}}" ;;
  "pane run")
    echo "$3 | $4" >> "$here/run.log"
    [ -e "$here/dies" ] || echo claude > "$here/busy"
    echo '{"result":{}}' ;;
  "pane get")
    # A program is an agent to herdr only a moment after it started.
    if [ -e "$here/seen" ]; then
      echo '{"result":{"pane":{"pane_id":"'"$3"'","agent":"claude"}}}'
    else
      : > "$here/seen"
      echo '{"result":{"pane":{"pane_id":"'"$3"'","agent":null}}}'
    fi ;;
  "agent wait")
    [ -e "$here/seen" ] || { echo '{"error":{"message":"agent target not found"}}'; exit 1; }
    echo '{"result":{"status":"idle"}}' ;;
  "agent prompt")
    echo "$3 | $4" >> "$here/prompts.log"
    echo '{"result":{}}' ;;
  *) echo '{"error":{"message":"fake herdr: unsupported"}}'; exit 1 ;;
esac
"#;

struct Unit {
    dir: tempfile::TempDir,
}

impl Unit {
    fn new() -> Self {
        let unit = Unit {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(unit.p("herdr"), FAKE_HERDR).unwrap();
        std::fs::set_permissions(unit.p("herdr"), std::fs::Permissions::from_mode(0o755)).unwrap();
        unit
    }

    fn p(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn host(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_octoscode"))
            .args(["olp", "unit", "host"])
            .args(args)
            .env("HOME", self.dir.path())
            .env("OLP_HERDR", self.p("herdr"))
            .env("OLP_UNIT_HOST_STATE", self.p("state"))
            .output()
            .unwrap()
    }

    fn lines(&self, name: &str) -> Vec<String> {
        std::fs::read_to_string(self.p(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Scenario: 窗格里没有程序时在原窗格重新启动主持并送接班提示;已在跑就不动
#[test]
fn a_host_whose_program_is_gone_is_started_again_and_told_to_take_over() {
    let unit = Unit::new();
    let command = ["--pane", "w3:p2", "--", "claude", "--continue", "two words"];
    let out = unit.host(&command);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(
        said(&out).contains("restarted in pane w3:p2"),
        "{}",
        said(&out)
    );
    assert_eq!(
        unit.lines("run.log"),
        ["w3:p2 | claude --continue 'two words'"]
    );
    let prompts = unit.lines("prompts.log");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(
        prompts[0].contains("octoscode olp remote inbox w3:p2"),
        "{prompts:?}"
    );

    // The next run of the job finds the host there.
    let out = unit.host(&command);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(
        said(&out).contains("running in pane w3:p2"),
        "{}",
        said(&out)
    );
    assert_eq!(unit.lines("run.log").len(), 1);
    assert_eq!(unit.lines("prompts.log").len(), 1);
}

/// Scenario: 窗格里有别的程序在跑时不启动、不发话
#[test]
fn a_pane_with_a_program_in_it_is_left_alone() {
    let unit = Unit::new();
    std::fs::write(unit.p("busy"), "kimi-code\n").unwrap();
    let out = unit.host(&["--pane", "w3:p2", "--", "kimi", "-y"]);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert!(said(&out).contains("kimi-code"), "{}", said(&out));
    assert!(unit.lines("run.log").is_empty());
    assert!(unit.lines("prompts.log").is_empty());
}

/// Scenario: 按标签找窗格,工作区不存在时新建;--no-prompt 不发话
#[test]
fn the_host_pane_is_found_or_created_by_its_workspace_label() {
    let unit = Unit::new();
    let out = unit.host(&["--label", "unit-host", "--no-prompt", "--", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert_eq!(unit.lines("run.log"), ["w7:p1 | claude"]);
    assert!(unit.lines("prompts.log").is_empty());
    assert!(unit.p("workspace").exists());
}

/// Scenario: 主持一起来就退出时,重启到上限后暂停;--reset 后恢复
#[test]
fn restarts_pause_when_the_host_keeps_dying() {
    let unit = Unit::new();
    std::fs::write(unit.p("dies"), "").unwrap();
    let args = [
        "--pane",
        "w3:p2",
        "--ready",
        "1",
        "--max-restarts",
        "2",
        "--",
        "claude",
    ];
    for _ in 0..2 {
        let out = unit.host(&args);
        assert_eq!(out.status.code(), Some(1), "{}", said(&out));
        assert!(said(&out).contains("没有程序在跑"), "{}", said(&out));
    }
    let out = unit.host(&args);
    assert_eq!(out.status.code(), Some(4), "{}", said(&out));
    assert!(said(&out).contains("已暂停"), "{}", said(&out));
    assert_eq!(unit.lines("run.log").len(), 2, "no third start");
    assert!(unit.lines("prompts.log").is_empty());

    std::fs::remove_file(unit.p("dies")).unwrap();
    let mut again = vec!["--reset"];
    again.extend(args);
    let out = unit.host(&again);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert_eq!(unit.lines("run.log").len(), 3);
}

/// Scenario: 参数不全时退出码 2,不碰 herdr
#[test]
fn a_host_without_a_pane_or_a_command_is_a_usage_error() {
    let unit = Unit::new();
    for args in [
        vec!["--", "claude"],
        vec!["--pane", "w3:p2", "--label", "x", "--", "claude"],
        vec!["--pane", "w3:p2"],
        vec!["--pane", "bad pane", "--", "claude"],
    ] {
        let out = unit.host(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", said(&out));
    }
    assert!(unit.lines("run.log").is_empty());
    // The command's own -h is not this command's help.
    let out = unit.host(&["--pane", "w3:p2", "--no-prompt", "--", "claude", "-h"]);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    assert_eq!(unit.lines("run.log"), ["w3:p2 | claude -h"]);
}
