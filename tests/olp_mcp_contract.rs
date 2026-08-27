//! Contract tests for the OLP-MCP Rust server (OUTER_LOOP_REVIEW #31),
//! `octoscode olp-mcp-serve`. One test per Scenario selector pinned in
//! `specs/task-req-olp-mcp.spec.md` — driven through a REAL subprocess
//! (newline-delimited JSON-RPC over stdio), with a temp outer root
//! (`OLP_MCP_OUTER_ROOT`) and a compressed clock (`OLP_MCP_TIMEOUT_SECS`).

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
    // Fake board_append.sh recording bodies into board_calls.log.
    let script = dir.join("board_append.sh");
    std::fs::write(
        &script,
        "#!/usr/bin/env bash\ncat >> \"$(dirname \"$1\")/board_calls.log\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

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

    let board = board_text(&root);
    assert!(board.contains("MCP(ask_outer)"), "ask audited: {board}");
}

/// Scenario: 90s 超时降级 — no answer present; compressed clock returns the
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
    assert!(board_text(&root).contains("timeout"), "timeout audited");
}

/// Scenario: 限额拒绝 — the 4th ask on one slice is refused and questions/
/// gains nothing.
#[test]
fn self_test_ask_outer_quota_refusal() {
    let root = temp_outer_root("quota");
    let mut server = ServerProc::spawn(&root, 0.1);
    for i in 0..3 {
        ask_outer(&mut server, &format!("q{i}"), "c", "t");
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
