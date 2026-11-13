#![cfg(unix)]

//! Contract tests for `scripts/olp-dispatch.py` and the occupancy view of
//! `scripts/olp-inbox.py` (SDD spec: `specs/task-olp-dispatch.spec.md`):
//! one command that checks the unit, writes the card and wakes the unit.
//!
//! herdr is replaced by a script that records its arguments and can be made
//! to fail through a flag file.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn scripts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const DATE: &str = "2026-11-13";

struct Hub {
    dir: tempfile::TempDir,
}

impl Hub {
    fn new() -> Self {
        let h = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(h.board(), "# 板\n\n").unwrap();
        let script = format!(
            "#!/bin/sh\n[ -e '{fail}' ] && {{ echo 'pane not found' >&2; exit 1; }}\necho \"$@\" >> '{log}'\n",
            fail = h.herdr_fail().display(),
            log = h.herdr_log().display(),
        );
        std::fs::write(h.fake_herdr(), script).unwrap();
        std::fs::set_permissions(h.fake_herdr(), std::fs::Permissions::from_mode(0o755)).unwrap();
        h
    }

    fn board(&self) -> PathBuf {
        self.dir.path().join("OUTER_LOOP_REVIEW.md")
    }
    fn fake_herdr(&self) -> PathBuf {
        self.dir.path().join("fake-herdr")
    }
    fn herdr_log(&self) -> PathBuf {
        self.dir.path().join("herdr.log")
    }
    fn herdr_fail(&self) -> PathBuf {
        self.dir.path().join("herdr.fail")
    }
    fn board_text(&self) -> String {
        std::fs::read_to_string(self.board()).unwrap()
    }
    fn board_line(&self, line: &str) {
        let mut text = self.board_text();
        text.push_str(line);
        text.push('\n');
        std::fs::write(self.board(), text).unwrap();
    }

    fn python(&self, script: &str) -> Command {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(scripts().join(script))
            .env("OLP_UNITS_DIR", self.dir.path().join("units"))
            .env("OLP_HERDR", self.fake_herdr());
        cmd
    }

    /// Register a unit and feed the hub one healthy heartbeat for it.
    fn healthy_unit(&self, unit: &str, n: u8) {
        let mac = format!("aa:bb:cc:dd:ee:{n:02x}");
        let out = self
            .python("olp-unit.py")
            .args(["register", unit, "--hostname", unit, "--mac", &mac])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        let beat = format!(
            "{{\"v\":1,\"unit\":\"{unit}\",\"hostname\":\"{unit}\",\"macs\":[\"{mac}\"],\"checks\":[{{\"name\":\"disk\",\"ok\":true}}]}}"
        );
        let mut child = self
            .python("olp-unit.py")
            .arg("receive")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(beat.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }

    fn dispatch(&self, unit: &str, title: &str, extra: &[&str], body: &str) -> Output {
        let mut cmd = self.python("olp-dispatch.py");
        cmd.arg(unit)
            .arg("--board")
            .arg(self.board())
            .args(["--title", title, "--date", DATE])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let _ = child.stdin.take().unwrap().write_all(body.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn units(&self) -> serde_json::Value {
        let out = self
            .python("olp-inbox.py")
            .arg(self.board())
            .args(["--units", "--json", "--today", DATE])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        serde_json::from_str(&stdout(&out)).unwrap()
    }

    fn busy(&self, unit: &str) -> Option<bool> {
        self.units()["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["unit"] == unit)
            .map(|u| u["busy"].as_bool().unwrap())
    }
}

/// Scenario: 一条命令查单元、写板并唤醒
#[test]
fn dispatch_writes_a_numbered_card_and_wakes_the_unit() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let h = Hub::new();
    h.healthy_unit("m07", 7);
    let out = h.dispatch(
        "m07",
        "修复启动崩溃",
        &["--wake", "w1:p2"],
        "背景:xxx\n分支 task/{N},只推 hub 本地仓库。\n",
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).lines().last(), Some("1"), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("本地复验通过(#1)"),
        "{}",
        stdout(&out)
    );

    let board = h.board_text();
    assert!(
        board.contains(
            "### 1. 修复启动崩溃(2026-11-13,外环(hub))\nunit: m07\n\n背景:xxx\n分支 task/1,只推 hub 本地仓库。\n<!-- olp-key: dispatch:m07:"
        ),
        "{board}"
    );
    let log = std::fs::read_to_string(h.herdr_log()).unwrap();
    assert!(
        log.starts_with("--machine m07 agent prompt w1:p2 "),
        "{log}"
    );
    assert!(log.contains("#1"), "{log}");
    assert_eq!(h.busy("m07"), Some(true));
}

/// Scenario: 不健康或正忙的单元不派卡
#[test]
fn dispatch_refuses_unhealthy_and_busy_units() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let h = Hub::new();
    let before = h.board_text();

    // Not registered at all, then registered but never heard from.
    let out = h.dispatch("m07", "卡一", &[], "正文\n");
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    h.python("olp-unit.py")
        .args([
            "register",
            "m07",
            "--hostname",
            "m07",
            "--mac",
            "aa:bb:cc:dd:ee:07",
        ])
        .output()
        .unwrap();
    let out = h.dispatch("m07", "卡一", &[], "正文\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("不能接卡"), "{}", stderr(&out));
    assert_eq!(h.board_text(), before, "a refusal writes nothing");

    h.healthy_unit("m07", 7);
    assert!(h.dispatch("m07", "卡一", &[], "正文\n").status.success());

    // One card at a time.
    let out = h.dispatch("m07", "卡二", &[], "正文二\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("正忙") && stderr(&out).contains("#1"),
        "{}",
        stderr(&out)
    );
    assert_eq!(h.board_text().matches("\n### ").count(), 1);

    // Pushed and returned: the unit is still on the card.
    h.board_line("> 单元(m07)·本地复验通过(#1): task/1 @ aaaaaaaaaaaa 复验 m07-ver");
    h.board_line("> 外环(hub)·打回(#1): 缺测试");
    assert_eq!(h.busy("m07"), Some(true));
    assert_eq!(
        h.dispatch("m07", "卡二", &[], "正文二\n").status.code(),
        Some(1)
    );

    // Accepted: free again.
    h.board_line("> 外环(hub)·采认(#1): 通过");
    assert_eq!(h.busy("m07"), Some(false));
    let out = h.dispatch("m07", "卡二", &[], "正文二\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).lines().last(), Some("2"));

    // The two escape hatches.
    let out = h.dispatch("m07", "加塞", &["--allow-busy"], "正文三\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let out = h.dispatch("m09", "没登记的单元", &["--skip-health"], "正文四\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

/// Scenario: 唤醒失败后原样重跑只补做唤醒
#[test]
fn dispatch_retry_after_a_failed_wake_does_not_write_a_second_card() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let h = Hub::new();
    h.healthy_unit("m07", 7);
    std::fs::write(h.herdr_fail(), "").unwrap();

    let args = ["--wake", "w1:p2"];
    let out = h.dispatch("m07", "卡一", &args, "正文\n");
    assert_eq!(out.status.code(), Some(5), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("#1 已写板"), "{}", stderr(&out));
    let after_first = h.board_text();
    assert!(after_first.contains("### 1. 卡一"));

    // Same command again, herdr working: no health or occupancy refusal even
    // though the unit is now busy with this very card.
    std::fs::remove_file(h.herdr_fail()).unwrap();
    let out = h.dispatch("m07", "卡一", &args, "正文\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("没有重复派卡"), "{}", stdout(&out));
    assert_eq!(stdout(&out).lines().last(), Some("1"));
    assert_eq!(h.board_text(), after_first, "the board is unchanged");
    let log = std::fs::read_to_string(h.herdr_log()).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");

    // --json reports what happened.
    let out = h.dispatch("m07", "卡一", &["--json"], "正文\n");
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["number"], 1);
    assert_eq!(v["created"], false);
    assert_eq!(v["watch_token"], "本地复验通过(#1)");
}

/// Scenario: 回测卡不派给做这张卡的单元
#[test]
fn dispatch_verification_card_avoids_the_author_unit() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let h = Hub::new();
    h.healthy_unit("m07", 7);
    h.healthy_unit("m08", 8);
    assert!(h.dispatch("m07", "实现卡", &[], "正文\n").status.success());
    h.board_line("> 外环(hub)·采认(#1): 通过");

    let out = h.dispatch("m07", "R1 冒烟:#1", &["--verify-of", "1"], "跑冒烟集\n");
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("另一台单元"), "{}", stderr(&out));
    let out = h.dispatch("m08", "R1 冒烟:#9", &["--verify-of", "9"], "跑冒烟集\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("没有 #9"), "{}", stderr(&out));

    let out = h.dispatch("m08", "R1 冒烟:#1", &["--verify-of", "1"], "跑冒烟集\n");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        h.board_text().contains(
            "### 2. R1 冒烟:#1(2026-11-13,外环(hub))\nunit: m08\nverify-of: #1\n\n跑冒烟集\n"
        ),
        "{}",
        h.board_text()
    );
    // The verification card occupies m08, not the author.
    assert_eq!(h.busy("m07"), Some(false));
    assert_eq!(h.busy("m08"), Some(true));
}

/// Scenario: 用法错误不写板
#[test]
fn dispatch_rejects_bad_usage() {
    let h = Hub::new();
    let before = h.board_text();
    let cases: [(&str, &str, &[&str], &str); 6] = [
        ("M07", "t", &["--skip-health"], "x\n"),
        ("m07", "   ", &["--skip-health"], "x\n"),
        ("m07", "t", &["--skip-health"], "  \n"),
        ("m07", "t", &["--skip-health", "--key", "bad key"], "x\n"),
        ("m07", "t", &["--skip-health", "--date", "tomorrow"], "x\n"),
        ("m07", "t", &["--skip-health", "--verify-of", "x"], "x\n"),
    ];
    for (unit, title, extra, body) in cases {
        let out = h.dispatch(unit, title, extra, body);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{unit} {title:?} {extra:?}: {}",
            stderr(&out)
        );
    }
    assert_eq!(h.board_text(), before);
    // The occupancy view of a board with no dispatched cards.
    assert_eq!(h.units()["units"].as_array().unwrap().len(), 0);
}
