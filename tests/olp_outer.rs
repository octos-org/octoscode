#![cfg(any(target_os = "linux", target_os = "macos"))]

//! Contract tests for `octoscode olp outer` (SDD spec:
//! `specs/task-olp-outer-autorestart.spec.md`): the outer is restarted in
//! its herdr pane when the duty lock is VACANT, never while it is HELD, and
//! restarts pause after repeated failures. herdr is a fake (`OLP_HERDR`)
//! whose `pane run` really runs the command, so the lock is really held.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .args(["-c", "command -v flock >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// herdr as far as `olp outer` uses it. State lives in files next to it:
/// `workspaces.json`, `busy` (foreground program names, one per line),
/// `calls.log` (every call), `run.pids` (pids of commands `pane run` began).
const FAKE_HERDR: &str = r#"#!/usr/bin/env python3
import json, os, subprocess, sys
here = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
if args[:1] == ["--session"]:
    args = args[2:]
with open(os.path.join(here, "calls.log"), "a") as f:
    f.write(" ".join(args[:2]) + "\n")
ws_file = os.path.join(here, "workspaces.json")
workspaces = json.load(open(ws_file)) if os.path.exists(ws_file) else []
def out(result):
    print(json.dumps({"id": "x", "result": result}))
if args[:2] == ["workspace", "list"]:
    out({"workspaces": workspaces})
elif args[:2] == ["workspace", "create"]:
    label = args[args.index("--label") + 1]
    wid = "w%d" % (len(workspaces) + 1)
    workspaces.append({"workspace_id": wid, "label": label})
    json.dump(workspaces, open(ws_file, "w"))
    out({"root_pane": {"pane_id": wid + ":p1", "workspace_id": wid}})
elif args[:2] == ["pane", "list"]:
    out({"panes": [{"pane_id": w["workspace_id"] + ":p1", "workspace_id": w["workspace_id"]} for w in workspaces]})
elif args[:2] == ["pane", "process-info"]:
    busy = os.path.join(here, "busy")
    names = open(busy).read().split() if os.path.exists(busy) else []
    procs = [{"name": "zsh", "pid": 1}] + [{"name": n, "pid": 2} for n in names]
    out({"process_info": {"foreground_processes": procs}})
elif args[:2] == ["pane", "run"]:
    p = subprocess.Popen(["/bin/sh", "-c", args[3]], start_new_session=True,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    with open(os.path.join(here, "run.pids"), "a") as f:
        f.write("%d\n" % p.pid)
    out({})
else:
    print(json.dumps({"error": {"message": "fake herdr: unsupported " + " ".join(args)}}))
    sys.exit(1)
"#;

struct Sb {
    dir: tempfile::TempDir,
}

impl Sb {
    fn new() -> Self {
        let sb = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for d in ["home", "project", "herdr"] {
            std::fs::create_dir_all(sb.p(d)).unwrap();
        }
        let fake = sb.p("herdr/herdr");
        std::fs::write(&fake, FAKE_HERDR).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(sb.p("board.md"), "# board\n\n## Active\n").unwrap();
        sb
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_octoscode"));
        cmd.env("HOME", self.p("home"))
            .env("OCTOSCODE_OLP_BUNDLE", self.p("bundle"))
            .env("OLP_HERDR", self.p("herdr/herdr"))
            .env("OLP_OUTER_STATE", self.p("state"))
            .env_remove("OLP_DUTY_LEASE");
        cmd
    }

    fn supervise(&self, extra: &[&str], agent: &str) -> Output {
        self.cmd()
            .args(["olp", "outer", "supervise", "--project"])
            .arg(self.p("project"))
            .args(["--signature", "hub-outer", "--duties", "primary-review"])
            .arg("--board")
            .arg(self.p("board.md"))
            .args(extra)
            .args(["--", "/bin/sh", "-c", agent])
            .output()
            .unwrap()
    }

    fn duty(&self) -> String {
        let out = self
            .cmd()
            .args(["outer-duty", "check", "--project"])
            .arg(self.p("project"))
            .output()
            .unwrap();
        stdout(&out).trim().to_string()
    }

    fn calls(&self, prefix: &str) -> usize {
        std::fs::read_to_string(self.p("herdr/calls.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with(prefix))
            .count()
    }

    fn board(&self) -> String {
        std::fs::read_to_string(self.p("board.md")).unwrap()
    }
}

impl Drop for Sb {
    fn drop(&mut self) {
        // Stop every hold the fake started (each in its own session).
        if let Ok(text) = std::fs::read_to_string(self.p("herdr/run.pids")) {
            for pid in text.lines().filter_map(|l| l.trim().parse::<i32>().ok()) {
                kill_group(pid);
            }
        }
    }
}

/// SIGKILL a process group and its leader (kill(2): no `kill` binary needed).
#[allow(unsafe_code)]
fn kill_group(pid: i32) {
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
        libc::kill(pid, libc::SIGKILL);
    }
}

fn wait_for(cond: impl Fn() -> bool) -> bool {
    for _ in 0..200 {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Scenario: 锁空着时在外环窗格里重新 hold;锁被占着时什么都不做
#[test]
fn outer_supervise_restarts_a_vacant_outer_and_leaves_a_held_one_alone() {
    let sb = Sb::new();
    let agent = "while true; do sleep 0.1; done";
    assert_eq!(sb.duty(), "VACANT");
    let out = sb.supervise(&[], agent);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("restarted in pane w1:p1"),
        "{}",
        stdout(&out)
    );
    assert_eq!(sb.duty(), "HELD");
    assert_eq!(
        sb.calls("workspace create"),
        1,
        "the outer's workspace is created once"
    );
    assert_eq!(sb.calls("pane run"), 1);
    if has_flock() {
        assert!(
            sb.board().contains("外环已自动重启(hub-outer)"),
            "{}",
            sb.board()
        );
    }

    // HELD: nothing is started, whatever the interval brings.
    let out = sb.supervise(&[], agent);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out).trim(), "held");
    assert_eq!(sb.calls("pane run"), 1, "never a second outer");

    // The outer dies: the lock is VACANT again and the same pane is reused.
    let pid = std::fs::read_to_string(sb.p("herdr/run.pids")).unwrap();
    kill_group(pid.lines().next().unwrap().trim().parse().unwrap());
    assert!(
        wait_for(|| sb.duty() == "VACANT"),
        "the lock frees when the outer dies"
    );
    let out = sb.supervise(&[], agent);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("w1:p1"));
    assert_eq!(
        sb.calls("workspace create"),
        1,
        "the existing workspace is reused"
    );
    assert_eq!(sb.duty(), "HELD");
}

/// Scenario: 外环一起来就挂,重启 N 次后暂停并在黑板上告警一次;--reset 后再试
#[test]
fn outer_supervise_pauses_after_repeated_failures() {
    let sb = Sb::new();
    let fails = |extra: &[&str]| {
        let mut args = vec!["--max-restarts", "2", "--wait", "1"];
        args.extend_from_slice(extra);
        sb.supervise(&args, "exit 1")
    };
    for _ in 0..2 {
        let out = fails(&[]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(stderr(&out).contains("锁仍是空的"), "{}", stderr(&out));
    }
    for _ in 0..2 {
        let out = fails(&[]);
        assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    }
    assert_eq!(sb.calls("pane run"), 2, "no restart while paused");
    if has_flock() {
        assert_eq!(
            sb.board().matches("外环自动重启已暂停").count(),
            1,
            "{}",
            sb.board()
        );
    }
    let out = fails(&["--reset"]);
    assert_eq!(out.status.code(), Some(1), "--reset allows another try");
    assert_eq!(sb.calls("pane run"), 3);
}

/// Scenario: 外环窗格里还有程序在跑而锁空着时不重启
#[test]
fn outer_supervise_leaves_a_busy_pane_alone() {
    let sb = Sb::new();
    std::fs::write(
        sb.p("herdr/workspaces.json"),
        r#"[{"workspace_id":"w1","label":"olp-outer"}]"#,
    )
    .unwrap();
    std::fs::write(sb.p("herdr/busy"), "claude\n").unwrap();
    let out = sb.supervise(&[], "true");
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(stderr(&out).contains("claude"), "{}", stderr(&out));
    assert_eq!(sb.calls("pane run"), 0);
}

/// Scenario: install 注册的常驻任务调用 octoscode 的绝对路径,并记下 herdr 的位置
#[test]
fn outer_install_registers_supervise_as_a_keepalive_job() {
    let sb = Sb::new();
    let jobs = sb.p("jobs");
    let out = sb
        .cmd()
        .args(["olp", "outer", "install", "--project"])
        .arg(sb.p("project"))
        .args(["--signature", "hub-outer", "--duties", "primary-review"])
        .args(["--interval", "60", "--no-load", "--dir"])
        .arg(&jobs)
        .args(["--", "claude", "--continue"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let job: String = std::fs::read_dir(&jobs)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    let bin = env!("CARGO_BIN_EXE_octoscode");
    assert!(job.contains(bin), "{job}");
    for word in [
        "supervise",
        "--signature",
        "hub-outer",
        "--continue",
        "OLP_HERDR",
    ] {
        assert!(job.contains(word), "job lacks {word}: {job}");
    }
    assert!(
        job.contains(&Path::new(&sb.p("herdr/herdr")).display().to_string()),
        "{job}"
    );
}
