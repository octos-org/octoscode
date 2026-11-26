#![cfg(any(target_os = "linux", target_os = "macos"))]

//! Contract tests for `octoscode olp outer` (SDD spec:
//! `specs/task-olp-outer-autorestart.spec.md`): the outer is restarted in
//! its herdr pane when the duty lock is VACANT, never while it is HELD, and
//! restarts pause after repeated failures. herdr is a fake (`OLP_HERDR`)
//! whose `pane run` really runs the command, so the lock is really held.

#[path = "support/olp_env.rs"]
mod olp_env;

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
/// `agent_state` ("never": the agent never gets ready), `calls.log` (every
/// call), `prompts.log` (prompts handed over), `run.pids` (pids of commands
/// `pane run` began).
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
elif args[:2] == ["agent", "wait"]:
    state = os.path.join(here, "agent_state")
    state = open(state).read().strip() if os.path.exists(state) else ""
    lost = state == "enter_lost" and "working" in args and not os.path.exists(os.path.join(here, "entered"))
    if state == "never" or lost:
        print(json.dumps({"error": {"message": "timeout"}}))
        sys.exit(1)
    out({"status": "idle"})
elif args[:2] == ["agent", "prompt"]:
    with open(os.path.join(here, "prompts.log"), "a") as f:
        f.write(args[3] + "\n")
    state = os.path.join(here, "agent_state")
    if os.path.exists(state) and open(state).read().strip() == "enter_lost":
        # Typed, but the Enter was lost: the agent never starts on it.
        print(json.dumps({"error": {"message": "timeout"}}))
        sys.exit(1)
    out({})
elif args[:2] == ["pane", "send-keys"]:
    open(os.path.join(here, "entered"), "w").write(args[3])
    out({})
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
        self.supervise_on("board.md", extra, agent)
    }

    fn supervise_on(&self, board: &str, extra: &[&str], agent: &str) -> Output {
        self.cmd()
            .args(["olp", "outer", "supervise", "--project"])
            .arg(self.p("project"))
            .args(["--signature", "hub-outer", "--duties", "primary-review"])
            .arg("--board")
            .arg(self.p(board))
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

/// Scenario: 黑板是事件日志时,install 另注册外环值守
#[test]
fn outer_install_registers_the_watcher_for_an_event_log() {
    let sb = Sb::new();
    std::fs::create_dir_all(sb.p("log")).unwrap();
    let install = |board: &str, jobs: &str| {
        let out = sb
            .cmd()
            .args(["olp", "outer", "install", "--project"])
            .arg(sb.p("project"))
            .args(["--signature", "hub-outer", "--duties", "primary-review"])
            .args(["--label", "my-outer", "--board"])
            .arg(sb.p(board))
            .args(["--no-load", "--dir"])
            .arg(sb.p(jobs))
            .args(["--", "claude", "--continue"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        let mut found: Vec<(String, String)> = std::fs::read_dir(sb.p(jobs))
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| {
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                )
            })
            .collect();
        found.sort();
        found
    };
    let on_log = install("log", "jobs-log");
    assert_eq!(on_log.len(), 2, "{on_log:?}");
    let watch = &on_log
        .iter()
        .find(|(name, _)| name.contains("outer-watch"))
        .expect("a job named outer-watch")
        .1;
    for word in ["watch", "--label", "my-outer", "--once", "OLP_HERDR"] {
        assert!(watch.contains(word), "missing {word}: {watch}");
    }
    assert!(
        watch.contains(&sb.p("log").display().to_string()),
        "{watch}"
    );
    assert!(!watch.contains("supervise"), "{watch}");
    // A Markdown board has its own watch: one job only.
    assert_eq!(install("board.md", "jobs-md").len(), 1);
}

/// Scenario: 重启后等 agent 就绪,把接班提示送进去;没送到在黑板上记一行;--no-prompt 不送
#[test]
fn outer_supervise_hands_the_restarted_outer_the_resume_prompt() {
    let sb = Sb::new();
    let agent = "while true; do sleep 0.1; done";
    let out = sb.supervise(&[], agent);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("prompted pane w1:p1"),
        "{}",
        stdout(&out)
    );
    let log = std::fs::read_to_string(sb.p("herdr/calls.log")).unwrap();
    let wait = log.find("agent wait").expect("waits for the agent");
    let prompt = log.find("agent prompt").expect("hands over the prompt");
    assert!(wait < prompt, "wait first, then prompt: {log}");
    let prompts = std::fs::read_to_string(sb.p("herdr/prompts.log")).unwrap();
    assert!(
        prompts.contains("octoscode olp outer resume --board"),
        "{prompts}"
    );
    assert!(prompts.contains("board.md"), "{prompts}");
}

#[test]
fn outer_supervise_notes_a_prompt_that_was_not_delivered() {
    let sb = Sb::new();
    std::fs::write(sb.p("herdr/agent_state"), "never").unwrap();
    let out = sb.supervise(&["--ready", "1"], "while true; do sleep 0.1; done");
    assert_eq!(out.status.code(), Some(0), "the restart itself succeeded");
    assert!(stderr(&out).contains("接班提示没送到"), "{}", stderr(&out));
    if has_flock() {
        assert!(sb.board().contains("接班提示未送达"), "{}", sb.board());
    }
    assert_eq!(sb.calls("agent prompt"), 0);
}

/// Scenario: 提示打进去了但 agent 没有开始办时,在它空闲时再按回车
#[test]
fn outer_supervise_presses_enter_again_when_the_prompt_was_not_taken() {
    let sb = Sb::new();
    std::fs::write(sb.p("herdr/agent_state"), "enter_lost").unwrap();
    let out = sb.supervise(&[], "while true; do sleep 0.1; done");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("prompted pane w1:p1"),
        "{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(sb.calls("agent prompt"), 1, "the prompt is typed once");
    assert_eq!(sb.calls("pane send-keys"), 1);
    let key = std::fs::read_to_string(sb.p("herdr/entered")).unwrap();
    assert_eq!(key, "Enter");
}

/// Scenario: 黑板是事件日志时,监督的留言写成日志里的一条事件
#[test]
fn outer_supervise_writes_its_notes_into_an_event_log() {
    let sb = Sb::new();
    std::fs::create_dir_all(sb.p("log")).unwrap();
    let out = sb.supervise_on("log", &["--no-prompt"], "while true; do sleep 0.1; done");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!stderr(&out).contains("没写进"), "{}", stderr(&out));
    let events = std::fs::read_to_string(sb.p("log/events-000001.jsonl")).unwrap();
    let note: serde_json::Value = serde_json::from_str(events.lines().next().unwrap()).unwrap();
    assert_eq!(note["kind"], "note");
    assert_eq!(note["actor"], "外环监督");
    let text = note["data"]["text"].as_str().unwrap();
    assert!(text.starts_with("外环已自动重启(hub-outer)"), "{text}");

    let view = sb
        .cmd()
        .args(["olp", "board", "render"])
        .arg(sb.p("log"))
        .output()
        .unwrap();
    assert!(
        stdout(&view).contains("> 外环监督: 外环已自动重启"),
        "{}",
        stdout(&view)
    );
    let brief = sb
        .cmd()
        .args(["olp", "outer", "resume", "--project"])
        .arg(sb.p("project"))
        .arg("--board")
        .arg(sb.p("log"))
        .output()
        .unwrap();
    let said = stdout(&brief);
    assert!(said.contains("监督任务的留言"), "{said}");
    assert!(said.contains("olp outer watch"), "{said}");
    assert!(said.contains("keepalive list"), "{said}");
    assert!(!said.contains("board watch"), "{said}");
}

#[test]
fn outer_supervise_sends_nothing_with_no_prompt() {
    let sb = Sb::new();
    let out = sb.supervise(&["--no-prompt"], "while true; do sleep 0.1; done");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(sb.calls("agent "), 0);
}

/// Scenario: 持锁的外环跑 resume 得到收件箱与检查单;不在 hold 里跑退出码 3
#[test]
fn outer_resume_briefs_the_holder_and_warns_anyone_else() {
    let sb = Sb::new();
    std::fs::write(
        sb.p("board.md"),
        "# board\n\n## Active\n\n### 7. 修登录页\nunit: m01\n\n> 内环(m01)·#7 ACK(done): 已改\n",
    )
    .unwrap();
    let brief = sb.p("brief.json");
    let resume = format!(
        "{} olp outer resume --board {} --project {} --signature hub-outer --json > {}; echo $? > {}.code",
        env!("CARGO_BIN_EXE_octoscode"),
        sb.p("board.md").display(),
        sb.p("project").display(),
        brief.display(),
        brief.display(),
    );
    // The outer runs resume from inside its hold.
    let out = sb
        .cmd()
        .args(["outer-duty", "hold", "--project"])
        .arg(sb.p("project"))
        .args([
            "--signature",
            "hub-outer",
            "--duties",
            "d",
            "--",
            "/bin/sh",
            "-c",
            &resume,
        ])
        .env("OLP_UNITS_DIR", sb.p("units"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let code = std::fs::read_to_string(brief.with_extension("json.code")).unwrap();
    assert_eq!(code.trim(), "0", "the holder may resume");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&brief).unwrap()).unwrap();
    assert_eq!(json["holder"], true, "{json}");
    assert_eq!(json["lock"], "HELD", "{json}");
    let inbox: Vec<&serde_json::Value> = json["summary"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["partition"] == "inbox")
        .collect();
    assert_eq!(inbox.len(), 1, "{json}");
    assert_eq!(inbox[0]["number"], 7);
    let checklist = json["checklist"].as_array().unwrap();
    assert!(
        checklist
            .iter()
            .any(|c| c.as_str().unwrap().contains("board watch")),
        "{json}"
    );

    // Not the holder: the brief still prints, with a warning and exit 3.
    let out = sb
        .cmd()
        .args(["olp", "outer", "resume", "--board"])
        .arg(sb.p("board.md"))
        .arg("--project")
        .arg(sb.p("project"))
        .env("OLP_UNITS_DIR", sb.p("units"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(stdout(&out).contains("不要接班"), "{}", stdout(&out));
    assert!(stdout(&out).contains("#7"), "{}", stdout(&out));
}
