//! Contract tests for the OLP-MCP Rust server (OUTER_LOOP_REVIEW #31),
//! `octoscode olp-mcp-serve`. One test per Scenario selector pinned in
//! `specs/task-req-olp-mcp.spec.md` — driven through a REAL subprocess
//! (newline-delimited JSON-RPC over stdio), with a temp outer root
//! (`OLP_MCP_OUTER_ROOT`) and a compressed clock (`OLP_MCP_TIMEOUT_SECS`).

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn server_binary() -> std::path::PathBuf {
    // OLP_MCP_SERVER_BIN overrides the binary path (CI / custom installs);
    // the default is cargo's own integration-test compile of the bin target —
    // correct under any CARGO_TARGET_DIR (no fragile target/debug probing).
    if let Ok(path) = std::env::var("OLP_MCP_SERVER_BIN") {
        return std::path::PathBuf::from(path);
    }
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_octoscode"))
}

struct ServerProc {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl ServerProc {
    fn spawn(root: &std::path::Path, timeout_secs: f64) -> Self {
        let mut child = Command::new(server_binary())
            .arg("olp-mcp-serve")
            .env("OLP_MCP_OUTER_ROOT", root)
            .env("OLP_MCP_TIMEOUT_SECS", timeout_secs.to_string())
            // The unit's configuration is the test's own, never this machine's.
            .env("OLP_UNIT_CONF", root.join("unit.conf"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn octoscode olp-mcp-serve");
        let stdout = child.stdout.take().expect("stdout piped");
        Self {
            child,
            reader: BufReader::new(stdout),
        }
    }

    fn call(&mut self, method: &str, params: Value, id: i64) -> Value {
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().expect("stdin piped");
        writeln!(stdin, "{}", serde_json::to_string(&frame).unwrap()).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read response");
        serde_json::from_str(line.trim()).unwrap_or_else(|_| panic!("bad frame: {line:?}"))
    }
}

impl Drop for ServerProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn temp_outer_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "olp-mcp-rust-test-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    for sub in ["mcp/questions", "mcp/answers", "mcp/consumed"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        // Fake board_append.sh recording bodies into board_calls.log.
        let script = dir.join("board_append.sh");
        std::fs::write(
            &script,
            "#!/usr/bin/env bash\ncat >> \"$(dirname \"$1\")/board_calls.log\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

#[cfg(unix)]
fn board_text(root: &std::path::Path) -> String {
    std::fs::read_to_string(root.join("board_calls.log")).unwrap_or_default()
}

/// Scenario: initialize 握手 — legal capabilities (tools non-empty), protocol
/// version matches the octos mcp handshake.
#[test]
fn self_test_initialize_handshake() {
    let root = temp_outer_root("init");
    let mut server = ServerProc::spawn(&root, 5.0);
    let resp = server.call(
        "initialize",
        json!({"protocolVersion": "2024-11-05", "capabilities": {},
               "clientInfo": {"name": "t", "version": "0"}}),
        1,
    );
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert!(resp["result"]["capabilities"].get("tools").is_some());
    assert_eq!(resp["result"]["serverInfo"]["name"], "olp-mcp");
}

/// Scenario: tools/list 仅二件 — exactly ask_outer + report_blocked, each
/// with a JSON Schema input declaration.
#[test]
fn self_test_tools_list_exactly_two() {
    let root = temp_outer_root("tools");
    let mut server = ServerProc::spawn(&root, 5.0);
    let resp = server.call("tools/list", json!({}), 2);
    let tools = resp["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, vec!["ask_outer", "report_blocked"]);
    for tool in tools {
        assert!(tool["inputSchema"]["required"].is_array());
    }
}

fn ask_outer(server: &mut ServerProc, question: &str, context: &str, tried: &str) -> String {
    let resp = server.call(
        "tools/call",
        json!({"name": "ask_outer", "arguments": {
            "question": question, "context": context, "tried": tried}}),
        42,
    );
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Scenario: ask_outer 正常往返 — question file lands with all fields, the
/// answer returns verbatim, consumed/ archives the pair, board audits.
#[test]
fn self_test_ask_outer_roundtrip() {
    let root = temp_outer_root("roundtrip");
    // Pre-seed the answer via a watcher thread: wait for the question file,
    // then write the answer with its id.
    let watcher_root = root.clone();
    let watcher = std::thread::spawn(move || {
        let questions = watcher_root.join("mcp/questions");
        let answers = watcher_root.join("mcp/answers");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(entries) = std::fs::read_dir(&questions) {
                for entry in entries.flatten() {
                    if let Ok(text) = std::fs::read_to_string(entry.path()) {
                        if let Ok(payload) = serde_json::from_str::<Value>(&text) {
                            let id = payload["id"].as_str().unwrap().to_string();
                            std::fs::write(
                                answers.join(format!("{id}.json")),
                                json!({"answer": "外环答:走候选 b"}).to_string(),
                            )
                            .unwrap();
                            return;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    let mut server = ServerProc::spawn(&root, 10.0);
    let answer = ask_outer(
        &mut server,
        "驱逐释放选 a 还是 b?",
        "#20 评审两方向",
        "读过两处源码",
    );
    watcher.join().unwrap();
    assert_eq!(answer, "外环答:走候选 b");

    // questions/ drained; consumed/ holds the archived pair with tried.
    let questions: Vec<_> = std::fs::read_dir(root.join("mcp/questions"))
        .unwrap()
        .filter(|e| e.is_ok())
        .collect();
    assert!(
        questions.is_empty(),
        "consumed pair drained from questions/"
    );
    let consumed: Vec<_> = std::fs::read_dir(root.join("mcp/consumed"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(consumed.len(), 1);
    let archive: Value =
        serde_json::from_str(&std::fs::read_to_string(consumed[0].path()).unwrap()).unwrap();
    assert_eq!(archive["question"]["tried"], "读过两处源码");
    assert_eq!(archive["answer"]["answer"], "外环答:走候选 b");

    #[cfg(unix)]
    {
        let board = board_text(&root);
        assert!(board.contains("MCP(ask_outer)"), "ask audited: {board}");
    }
}

/// Scenario: 超时降级 — no answer present; compressed clock returns the
/// degraded guidance, never blocks, timeout audited.
#[test]
fn self_test_ask_outer_timeout_degrades() {
    let root = temp_outer_root("timeout");
    let mut server = ServerProc::spawn(&root, 0.4);
    let start = Instant::now();
    let answer = ask_outer(&mut server, "无人作答的问题", "x", "已自查");
    let elapsed = start.elapsed();
    assert!(answer.contains("超时降级"), "{answer}");
    assert!(answer.contains("ACK(blocked)"), "{answer}");
    assert!(
        elapsed < Duration::from_secs(3),
        "compressed clock: {elapsed:?}"
    );
    #[cfg(unix)]
    assert!(board_text(&root).contains("timeout"), "timeout audited");
}

/// Scenario: hub 用事件日志时,提问与它的结局记进 hub 的日志
#[test]
fn ask_outer_on_a_log_hub_reports_the_question_and_how_it_ended() {
    let root = temp_outer_root("loghub");
    let log = root.join("log");
    std::fs::create_dir_all(&log).unwrap();
    std::fs::write(
        root.join("unit.conf"),
        format!(
            "hub = local\nunit = m07\nstore = log\nlog = {}\n",
            log.display()
        ),
    )
    .unwrap();
    let mut server = ServerProc::spawn(&root, 0.4);
    let answer = ask_outer(&mut server, "接口用哪个版本\n第二行", "x", "已自查");
    assert!(answer.contains("超时降级"), "{answer}");
    // The end is reported on its own time, after the reply.
    let read = || -> Vec<Value> {
        std::fs::read_to_string(log.join("events-000001.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while read().len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let events = read();
    let kinds: Vec<&str> = events.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["asked", "timed_out"], "{events:?}");
    assert_eq!(events[0]["unit"], "m07");
    assert_eq!(events[0]["data"]["question"], "接口用哪个版本 第二行");
    assert_eq!(events[0]["data"]["id"], events[1]["data"]["id"]);
    assert!(answer.contains(events[0]["data"]["id"].as_str().unwrap()));
}

/// Scenario: 答案已经到了就立刻返回,不等 hub 收完上报
#[cfg(unix)]
#[test]
fn ask_outer_returns_the_answer_without_waiting_for_a_slow_hub() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = temp_outer_root("slowhub");
    std::fs::write(
        root.join("unit.conf"),
        "hub = hub-01\nunit = m07\nstore = log\n",
    )
    .unwrap();
    // The hub takes eight seconds over every report.
    let ssh = root.join("slow-ssh");
    let seen = root.join("ssh-calls");
    std::fs::write(
        &ssh,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nsleep 8\nexit 0\n",
            seen.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let watcher_root = root.clone();
    let watcher = std::thread::spawn(move || {
        let questions = watcher_root.join("mcp/questions");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for entry in std::fs::read_dir(&questions)
                .into_iter()
                .flatten()
                .flatten()
            {
                let payload = std::fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok());
                if let Some(payload) = payload {
                    let id = payload["id"].as_str().unwrap().to_string();
                    std::fs::write(
                        watcher_root.join(format!("mcp/answers/{id}.json")),
                        json!({"answer": "用 v2"}).to_string(),
                    )
                    .unwrap();
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let mut child = Command::new(server_binary())
        .arg("olp-mcp-serve")
        .env("OLP_MCP_OUTER_ROOT", &root)
        .env("OLP_MCP_TIMEOUT_SECS", "30")
        .env("OLP_UNIT_CONF", root.join("unit.conf"))
        .env("OLP_SSH", &ssh)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn octoscode olp-mcp-serve");
    let stdout = child.stdout.take().expect("stdout piped");
    let mut server = ServerProc {
        child,
        reader: BufReader::new(stdout),
    };
    let started = Instant::now();
    let answer = ask_outer(&mut server, "接口用哪个版本", "x", "已自查");
    let took = started.elapsed();
    watcher.join().unwrap();
    assert_eq!(answer, "用 v2");
    assert!(
        took < Duration::from_secs(5),
        "the answer waited for the hub: {took:?}"
    );
    // The question itself is on its way to the hub all the same.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !seen.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let calls = std::fs::read_to_string(&seen).unwrap_or_default();
    assert!(calls.contains("question ask"), "{calls}");
}

/// Scenario: 限额拒绝 — the 4th ask on one slice is refused and questions/
/// gains nothing.
#[test]
fn self_test_ask_outer_quota_refusal() {
    let root = temp_outer_root("quota");
    let mut server = ServerProc::spawn(&root, 0.1);
    let questions = |root: &std::path::Path| {
        std::fs::read_dir(root.join("mcp/questions"))
            .unwrap()
            .count()
    };
    // All three asks of the quota really reach the mailbox (retrospective
    // #11: generating an id used to consume quota too, so the third was
    // already refused).
    for i in 0..3 {
        let reply = ask_outer(&mut server, &format!("q{i}"), "c", "t");
        assert!(!reply.contains("上限"), "ask {i} must be accepted: {reply}");
        assert_eq!(questions(&root), i + 1, "ask {i} reaches the mailbox");
    }
    let before = std::fs::read_dir(root.join("mcp/questions"))
        .unwrap()
        .count();
    let refusal = ask_outer(&mut server, "q4", "c", "t");
    let after = std::fs::read_dir(root.join("mcp/questions"))
        .unwrap()
        .count();
    assert!(refusal.contains("已达 3 次上限"), "{refusal}");
    assert_eq!(before, after, "refused ask never enters the mailbox");
}

/// Scenario: 投递失败不计入配额,立刻报告而不是等到超时(复盘第 11 条)
#[cfg(unix)]
#[test]
fn self_test_ask_outer_failed_delivery_costs_no_quota() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = temp_outer_root("delivery");
    // Two seconds per unanswered ask: long enough to tell an immediate
    // report from waiting out the timeout.
    let mut server = ServerProc::spawn(&root, 2.0);
    let mailbox = root.join("mcp/questions");
    let set_mode =
        |mode| std::fs::set_permissions(&mailbox, std::fs::Permissions::from_mode(mode)).unwrap();
    set_mode(0o555);
    let started = std::time::Instant::now();
    let reply = ask_outer(&mut server, "q", "c", "t");
    set_mode(0o755);
    assert!(
        reply.contains("投递失败") && reply.contains("不计入配额"),
        "{reply}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1500),
        "a failed delivery is reported at once, not after the timeout"
    );
    // The same server still has its whole quota.
    for i in 0..3 {
        let reply = ask_outer(&mut server, &format!("q{i}"), "c", "t");
        assert!(
            !reply.contains("上限"),
            "ask {i} after a failed delivery: {reply}"
        );
    }
    let refusal = ask_outer(&mut server, "q4", "c", "t");
    assert!(refusal.contains("已达 3 次上限"), "{refusal}");
}

/// Scenario: tried 必填 — empty tried is refused without a mailbox write.
#[test]
fn self_test_ask_outer_requires_tried() {
    let root = temp_outer_root("tried");
    let mut server = ServerProc::spawn(&root, 0.1);
    let before = std::fs::read_dir(root.join("mcp/questions"))
        .unwrap()
        .count();
    let refusal = ask_outer(&mut server, "q", "c", "  ");
    let after = std::fs::read_dir(root.join("mcp/questions"))
        .unwrap()
        .count();
    assert!(
        refusal.contains("tried") && refusal.contains("拒绝"),
        "{refusal}"
    );
    assert_eq!(before, after);
}

/// Scenario: report_blocked 直通落板 — no mailbox round-trip, board audit
/// signed MCP(ask_outer).
#[test]
#[cfg(unix)]
fn self_test_report_blocked_board_only() {
    let root = temp_outer_root("blocked");
    let mut server = ServerProc::spawn(&root, 1.0);
    let resp = server.call(
        "tools/call",
        json!({"name": "report_blocked", "arguments": {
            "reason": "cargo 窗口被占", "needs": "外环代跑验证"}}),
        99,
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("直通落板"), "{text}");
    assert_eq!(
        std::fs::read_dir(root.join("mcp/questions"))
            .unwrap()
            .count(),
        0,
        "no mailbox file for report_blocked"
    );
    let board = board_text(&root);
    assert!(
        board.contains("blocked") && board.contains("cargo 窗口被占"),
        "{board}"
    );
    assert!(board.contains("MCP(ask_outer)"));
}
