#![cfg(unix)]

//! Contract tests for the cross-machine ask_outer round trip (SDD spec:
//! `specs/task-olp-ask-roundtrip.spec.md`).
//!
//! The real `octoscode olp-mcp-serve` runs as the unit's MCP server. Its
//! audit hook forwards to the hub's board through `olp-board-remote.py
//! append --sign`; the outer loop sees the question in `olp-inbox.py` and
//! answers with `olp-answer.py`. ssh in both directions is one stand-in
//! script that runs the matching forced command locally.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

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

struct Cluster {
    dir: tempfile::TempDir,
}

impl Cluster {
    fn new() -> Self {
        let c = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(c.board(), "# 板\n\n### 212. 一张卡\nunit: m07\n\n").unwrap();
        for sub in ["mcp/questions", "mcp/answers", "mcp/consumed"] {
            std::fs::create_dir_all(c.unit_root().join(sub)).unwrap();
        }
        // One stand-in for ssh in both directions: the first word of the
        // remote command tells which forced command would have run.
        let ssh = format!(
            "#!/bin/sh\n\
             [ -e '{down}' ] && exit 255\n\
             while [ \"$1\" = \"-o\" ]; do shift 2; done\n\
             shift\n\
             case \"$1\" in\n\
               append|read) SSH_ORIGINAL_COMMAND=\"$*\" exec python3 -B '{remote}' serve --unit m07 --board '{board}' ;;\n\
               *) SSH_ORIGINAL_COMMAND=\"$*\" exec python3 -B '{answer}' serve --root '{root}' --max-age \"$(cat '{max_age}' 2>/dev/null || echo 50)\" ;;\n\
             esac\n",
            down = c.down().display(),
            remote = scripts().join("olp-board-remote.py").display(),
            answer = scripts().join("olp-answer.py").display(),
            board = c.board().display(),
            root = c.unit_root().display(),
            max_age = c.max_age().display(),
        );
        c.write_executable(&c.fake_ssh(), &ssh);
        // The unit's MCP audit hook: forward, signed, to the hub.
        let hook = format!(
            "#!/bin/sh\nexec python3 -B '{}' append --sign\n",
            scripts().join("olp-board-remote.py").display()
        );
        c.write_executable(&c.unit_root().join("board_append.sh"), &hook);
        c
    }

    fn write_executable(&self, path: &std::path::Path, text: &str) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn board(&self) -> PathBuf {
        self.dir.path().join("OUTER_LOOP_REVIEW.md")
    }
    fn unit_root(&self) -> PathBuf {
        self.dir.path().join("unit-outer")
    }
    fn fake_ssh(&self) -> PathBuf {
        self.dir.path().join("fake-ssh")
    }
    /// Seconds the unit's forced command lets a question wait (file content).
    fn max_age(&self) -> PathBuf {
        self.dir.path().join("max-age")
    }
    fn down(&self) -> PathBuf {
        self.dir.path().join("net.down")
    }
    fn board_text(&self) -> String {
        std::fs::read_to_string(self.board()).unwrap()
    }

    fn env(&self, cmd: &mut Command) {
        cmd.env("OLP_SSH", self.fake_ssh())
            .env("OLP_HUB", "olp@hub-01")
            .env("OLP_UNIT_ID", "m07")
            .env("OLP_QUEUE_DIR", self.dir.path().join("queue"))
            .env("OLP_UNIT_CONF", self.dir.path().join("absent.conf"));
    }

    /// The unit's MCP server, as the inner loop would have it.
    fn mcp_server(&self, timeout_secs: f64) -> Child {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_octoscode"));
        cmd.arg("olp-mcp-serve")
            .env("OLP_MCP_OUTER_ROOT", self.unit_root())
            .env("OLP_MCP_TIMEOUT_SECS", timeout_secs.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        self.env(&mut cmd);
        cmd.spawn().unwrap()
    }

    fn python(&self, script: &str, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(scripts().join(script))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.env(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.unwrap_or("").as_bytes());
        child.wait_with_output().unwrap()
    }

    fn inbox(&self) -> Value {
        let out = self.python(
            "olp-inbox.py",
            &[self.board().to_str().unwrap(), "--json"],
            None,
        );
        assert!(out.status.success(), "{}", stderr(&out));
        serde_json::from_str(&stdout(&out)).unwrap()
    }

    /// Poll the hub's board until `needle` shows up.
    fn wait_for_board(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let text = self.board_text();
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "board never showed {needle:?}: {text}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn call(child: &mut Child, name: &str, arguments: Value) -> String {
    let frame = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": arguments}});
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, "{frame}").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    let resp: Value = serde_json::from_str(line.trim()).unwrap();
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text in {resp}"))
        .to_string()
}

/// Scenario: 单元里的提问到达 hub,回答送回单元,内环拿到原文
#[test]
fn ask_outer_round_trip_across_machines() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let c = Cluster::new();
    let mut server = c.mcp_server(30.0);
    let asker = std::thread::spawn(move || {
        let reply = call(
            &mut server,
            "ask_outer",
            json!({"question": "hdc 连不上 d03,换设备还是等?", "context": "#212 跑测", "tried": "重插过 USB"}),
        );
        let _ = server.kill();
        let _ = server.wait();
        reply
    });

    // The question reaches the hub's board, signed by the unit.
    let board = c.wait_for_board("MCP(ask_outer) ask: id=");
    let line = board.lines().find(|l| l.contains("ask: id=")).unwrap();
    assert!(line.starts_with("> 单元(m07)·- "), "{line}");

    // It is at the top of the outer loop's inbox, with the unit to answer.
    let inbox = c.inbox();
    let questions = inbox["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 1, "{inbox}");
    assert_eq!(questions[0]["unit"], "m07");
    assert!(
        questions[0]["question"]
            .as_str()
            .unwrap()
            .contains("hdc 连不上 d03")
    );
    let id = questions[0]["id"].as_str().unwrap().to_string();

    // The full question is fetched from the unit, not from the board line.
    let out = c.python("olp-answer.py", &["list", "m07"], None);
    assert!(
        out.status.success() && stdout(&out).contains(&id),
        "{}",
        stderr(&out)
    );
    let out = c.python("olp-answer.py", &["show", "m07", &id], None);
    assert!(out.status.success(), "{}", stderr(&out));
    let shown: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(shown["tried"], "重插过 USB");
    assert_eq!(shown["context"], "#212 跑测");

    // The outer loop answers; the inner loop receives the text verbatim.
    let answer = "换 d05,d03 标记离线。";
    let out = c.python("olp-answer.py", &["answer", "m07", &id], Some(answer));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(asker.join().unwrap(), answer);

    // The unit's server audits the answer, and that line reaches the hub
    // too, so the question leaves the inbox.
    c.wait_for_board(&format!("MCP(ask_outer) answer: id={id}"));
    assert_eq!(c.inbox()["questions"].as_array().unwrap().len(), 0);

    // The question is no longer waiting: a second answer is refused and
    // leaves nothing in the mailbox.
    let out = c.python("olp-answer.py", &["answer", "m07", &id], Some("重复的回答"));
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let leftover = std::fs::read_dir(c.unit_root().join("mcp/answers"))
        .unwrap()
        .count();
    assert_eq!(leftover, 0, "no stray answer file");
}

/// Scenario: 没人回答时提问超时,板上的超时行让它离开收件箱
#[test]
fn ask_outer_timeout_leaves_the_inbox_and_refuses_a_late_answer() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let c = Cluster::new();
    let mut server = c.mcp_server(0.5);
    let reply = call(
        &mut server,
        "ask_outer",
        json!({"question": "没人回答的问题", "context": "x", "tried": "自查过"}),
    );
    assert!(reply.contains("超时降级"), "{reply}");
    // report_blocked goes straight to the board as well.
    let blocked = call(
        &mut server,
        "report_blocked",
        json!({"reason": "hdc 连不上设备", "needs": "换设备"}),
    );
    assert!(blocked.contains("落板"), "{blocked}");
    let _ = server.kill();
    let _ = server.wait();

    let board = c.wait_for_board("MCP(ask_outer) blocked: reason=hdc 连不上设备");
    assert!(board.contains("> 单元(m07)·- "), "{board}");
    let inbox = c.inbox();
    assert_eq!(inbox["questions"].as_array().unwrap().len(), 0, "{inbox}");

    // The inner loop has moved on, but its question file is still in the
    // mailbox. An answer now would never be read, so the unit refuses it
    // and tells the outer loop to use another channel.
    let id = board
        .lines()
        .find_map(|l| l.split("timeout: id=").nth(1))
        .map(|rest| rest.split_whitespace().next().unwrap().to_string())
        .expect("timeout line with an id");
    std::fs::write(c.max_age(), "0").unwrap();
    let out = c.python("olp-answer.py", &["list", "m07"], None);
    assert!(stdout(&out).contains("已超时"), "{}", stdout(&out));
    let out = c.python("olp-answer.py", &["answer", "m07", &id], Some("迟到的回答"));
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(stderr(&out).contains("降级指引"), "{}", stderr(&out));
    let leftover = std::fs::read_dir(c.unit_root().join("mcp/answers"))
        .unwrap()
        .count();
    assert_eq!(leftover, 0, "no stray answer file");
    // Reading an expired question is still allowed.
    assert!(
        c.python("olp-answer.py", &["show", "m07", &id], None)
            .status
            .success()
    );
}

/// Scenario: 回答命令的输入检查与单元不可达
#[test]
fn answer_validates_input_and_reports_an_unreachable_unit() {
    let c = Cluster::new();
    let cases: [(&[&str], Option<&str>); 5] = [
        (&["answer", "M07", "abcd1234"], Some("x")),
        (&["answer", "m07", "../../etc"], Some("x")),
        (&["answer", "m07", "abcd1234"], Some("   ")),
        (&["show", "m07", "a b"], None),
        (
            &["answer", "m07", "abcd1234", "--to", "-oProxyCommand=x"],
            Some("x"),
        ),
    ];
    for (args, stdin) in cases {
        let out = c.python("olp-answer.py", args, stdin);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    // A question that is not waiting.
    let out = c.python("olp-answer.py", &["answer", "m07", "abcd1234"], Some("x"));
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let out = c.python("olp-answer.py", &["show", "m07", "abcd1234"], None);
    assert_eq!(out.status.code(), Some(3));

    // The unit's forced command allows nothing else.
    for original in ["", "cat /etc/passwd", "answer", "answer a b", "show ../x"] {
        let out = Command::new("python3")
            .arg("-B")
            .arg(scripts().join("olp-answer.py"))
            .args(["serve", "--root"])
            .arg(c.unit_root())
            .env("SSH_ORIGINAL_COMMAND", original)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(3), "{original:?}: {}", stderr(&out));
    }

    std::fs::write(c.down(), "").unwrap();
    let out = c.python("olp-answer.py", &["list", "m07"], None);
    assert_eq!(out.status.code(), Some(75), "{}", stderr(&out));
}
