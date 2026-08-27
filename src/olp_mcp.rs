//! OLP-MCP outer-loop server (OUTER_LOOP_REVIEW #31): the Rust port of the
//! campaign-night Python prototype (`scripts/olp-mcp-server.py`, now archived
//! under `scripts/reference/`). Behavior is pinned by the seven Scenarios in
//! `specs/task-req-olp-mcp.spec.md` and exercised end-to-end by
//! `tests/olp_mcp_contract.rs` (real subprocess stdio).
//!
//! Transport: newline-delimited JSON-RPC over stdio — the framing rmcp's
//! TokioChildProcess (the `octos mcp` client) speaks.
//!
//! Pure stdlib-Rust (no new crates): stdin lines in, stdout lines out, file
//! mailbox under `$OLP_MCP_OUTER_ROOT` (default `~/.octos/outer`), audit via
//! `board_append.sh`.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const SERVER_NAME: &str = "olp-mcp";
pub const SERVER_VERSION: &str = "0.2.0";
pub const SIGNATURE: &str = "MCP(ask_outer)";
pub const ASK_TIMEOUT_SECS: f64 = 90.0;
pub const ASK_POLL_INTERVAL_SECS: f64 = 0.5;
pub const ASK_QUOTA_PER_SLICE: usize = 3;
const BOARD_RELATIVE: &str = "OUTER_LOOP_MCP.md";

const DEGRADED_GUIDANCE: &str = "外环 90s 未应答(超时降级):按黑板既有指导继续推进;若确实无法推进,以 ACK(blocked) 收场并注明本次问询 id,外环会在线信箱补答。";
const QUOTA_REFUSAL: &str = "拒绝:本切片 ask_outer 问询已达 3 次上限(防思考外包)。请按黑板既有指导自行推进,或以 report_blocked 直通落板请求外环介入。";
const TRIED_REFUSAL: &str =
    "拒绝:tried 字段为空。防思考外包纪律要求先自行尝试——把已试过的路径写进 tried 再问。";

/// Mailbox root — overridable for tests; runtime default `~/.octos/outer`.
pub fn outer_root() -> PathBuf {
    if let Ok(override_root) = std::env::var("OLP_MCP_OUTER_ROOT") {
        return PathBuf::from(override_root);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".octos").join("outer")
}

/// Serve newline-delimited JSON-RPC over the given reader/writer. Returns the
/// number of ask_outer dispatches (for self-report).
pub fn serve(
    reader: impl BufRead,
    mut writer: impl Write,
    timeout_secs: f64,
) -> std::io::Result<usize> {
    let root = outer_root();
    let server = OlpMcpServer::new(&root, timeout_secs, ASK_POLL_INTERVAL_SECS);
    let mut asks = 0usize;
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                let _ = writeln!(
                    writer,
                    "{}",
                    json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": {"code": -32700, "message": "parse error"},
                    })
                );
                let _ = writer.flush();
                continue;
            }
        };
        if request.get("method").and_then(Value::as_str) == Some("ask_counter") {
            // internal self-test hook: report the dispatch counter (never a
            // real MCP method; rmcp never sends it)
            let _ = writeln!(writer, "{}", json!({"asks": asks}));
            let _ = writer.flush();
            continue;
        }
        let response = server.handle_request(&request, &mut asks);
        if let Some(response) = response {
            let _ = writeln!(
                writer,
                "{}",
                serde_json::to_string(&response).unwrap_or_default()
            );
            let _ = writer.flush();
        }
    }
    Ok(asks)
}

pub struct OlpMcpServer<'a> {
    root: &'a std::path::Path,
    questions_dir: PathBuf,
    answers_dir: PathBuf,
    consumed_dir: PathBuf,
    timeout_secs: f64,
    poll_interval: f64,
}

impl<'a> OlpMcpServer<'a> {
    pub fn new(root: &'a std::path::Path, timeout_secs: f64, poll_interval: f64) -> Self {
        let questions_dir = root.join("mcp").join("questions");
        let answers_dir = root.join("mcp").join("answers");
        let consumed_dir = root.join("mcp").join("consumed");
        for dir in [&questions_dir, &answers_dir, &consumed_dir] {
            let _ = std::fs::create_dir_all(dir);
        }
        Self {
            root,
            questions_dir,
            answers_dir,
            consumed_dir,
            timeout_secs,
            poll_interval,
        }
    }

    fn board_append(&self, text: &str) {
        // Audit must never crash the server: missing script / failed spawn
        // degrade to a silent skip (the Python port logged; stderr is owned
        // by the MCP client transport, so we stay quiet).
        let script = self.root.join("board_append.sh");
        let board = self.root.join(BOARD_RELATIVE);
        if !script.exists() {
            return;
        }
        let _ = Command::new("bash")
            .arg(&script)
            .arg(&board)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            });
    }

    fn audit(&self, kind: &str, detail: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // RFC3339-ish local timestamp (UTC) without a chrono dependency.
        let days = now / 86_400;
        let secs_of_day = now % 86_400;
        let (year, month, day) = civil_from_days(days as i64);
        let entry = format!(
            "\n- {:04}-{:02}-{:02} {:02}:{:02}:{:02} {} {}: {}\n",
            year,
            month,
            day,
            secs_of_day / 3600,
            (secs_of_day % 3600) / 60,
            secs_of_day % 60,
            SIGNATURE,
            kind,
            detail
        );
        self.board_append(&entry);
    }

    pub fn ask_outer(&self, args: &Value) -> String {
        let get = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let question = get("question");
        let context = get("context");
        let tried = get("tried");

        if tried.is_empty() {
            self.audit("refusal", "ask_outer rejected: empty tried");
            return TRIED_REFUSAL.into();
        }
        if ASK_STATE.with(|state| state.get()) >= ASK_QUOTA_PER_SLICE {
            self.audit(
                "refusal",
                &format!("ask_outer rejected: quota {ASK_QUOTA_PER_SLICE} exhausted"),
            );
            return QUOTA_REFUSAL.into();
        }
        if question.is_empty() {
            return "拒绝:question 为空。".into();
        }

        ASK_STATE.with(|state| state.set(state.get() + 1));
        let ask_id = short_uuid();
        let ts = timestamp_compact();
        let payload = json!({
            "id": ask_id, "ts": ts,
            "question": question, "context": context, "tried": tried,
        });
        let question_path = self.questions_dir.join(format!("{ts}-{ask_id}.json"));
        let _ = std::fs::write(
            &question_path,
            serde_json::to_string_pretty(&payload).unwrap_or_default(),
        );
        let detail_head: String = question.chars().take(200).collect();
        self.audit("ask", &format!("id={ask_id} question={detail_head}"));

        let answer_path = self.answers_dir.join(format!("{ask_id}.json"));
        let deadline = Instant::now() + Duration::from_secs_f64(self.timeout_secs);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&answer_path) {
                if let Ok(answer) = serde_json::from_str::<Value>(&text) {
                    let reply = answer
                        .get("answer")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.audit(
                        "answer",
                        &format!("id={ask_id} answered ({} chars)", reply.chars().count()),
                    );
                    // Archive the consumed pair (S3): questions/ holds only
                    // unanswered asks; the audit trail keeps full history.
                    let archive = self.consumed_dir.join(format!("{ts}-{ask_id}.json"));
                    let _ = std::fs::write(
                        &archive,
                        serde_json::to_string_pretty(&json!({
                            "question": payload, "answer": answer,
                        }))
                        .unwrap_or_default(),
                    );
                    let _ = std::fs::remove_file(&question_path);
                    let _ = std::fs::remove_file(&answer_path);
                    return reply;
                }
            }
            std::thread::sleep(Duration::from_secs_f64(self.poll_interval));
        }

        self.audit(
            "timeout",
            &format!("id={ask_id} no answer in {:.0}s", self.timeout_secs),
        );
        format!("{DEGRADED_GUIDANCE}\n(问询 id: {ask_id})")
    }

    pub fn report_blocked(&self, args: &Value) -> String {
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let needs = args
            .get("needs")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if reason.is_empty() {
            return "拒绝:reason 为空。".into();
        }
        let head: String = reason.chars().take(200).collect();
        let needs_head: String = needs.chars().take(200).collect();
        self.audit("blocked", &format!("reason={head} needs={needs_head}"));
        "已直通落板(署名 MCP(ask_outer)),外环会在黑板看到。".into()
    }

    pub fn handle_request(&self, request: &Value, asks: &mut usize) -> Option<Value> {
        let method = request.get("method").and_then(Value::as_str)?;
        let id = request.get("id").cloned();

        let result = match method {
            "initialize" => json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
            }),
            "initialized" | "notifications/initialized" => return None,
            "tools/list" => json!({"tools": tools_schema()}),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let text = match name {
                    "ask_outer" => self.ask_outer(&args),
                    "report_blocked" => self.report_blocked(&args),
                    other => {
                        return Some(json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32602,
                                      "message": format!("unknown tool: {other:?} (surface is exactly two tools)")},
                        }));
                    }
                };
                ASK_STATE.with(|state| *asks = state.get());
                return Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {"content": [{"type": "text", "text": text}], "isError": false},
                }));
            }
            "ping" => json!({}),
            _ if id.is_none() => return None,
            _ => {
                return Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("method not found: {method}")},
                }));
            }
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}

thread_local! {
    static ASK_STATE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn tools_schema() -> Value {
    json!([
        {
            "name": "ask_outer",
            "description": "Ask the outer loop a question and wait (<=90s) for the answer via the mailbox. Quota: 3 per slice; `tried` is mandatory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "The question for the outer loop"},
                    "context": {"type": "string", "description": "Where you are stuck / relevant state"},
                    "tried": {"type": "string", "description": "What you already tried (must be non-empty)"},
                },
                "required": ["question", "context", "tried"],
            },
        },
        {
            "name": "report_blocked",
            "description": "Report a blocker straight to the outer-loop board (no mailbox round-trip).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reason": {"type": "string", "description": "Why you are blocked"},
                    "needs": {"type": "string", "description": "What you need to unblock"},
                },
                "required": ["reason", "needs"],
            },
        },
    ])
}

fn short_uuid() -> String {
    // 12 hex chars from OS entropy — no uuid crate.
    let bytes = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mixed = bytes.wrapping_mul(0x9E3779B97F4A7C15) ^ pid.rotate_left(32);
    let counter = ASK_STATE.with(|s| {
        let v = s.get() + 1;
        s.set(v);
        v as u64
    });
    let value = mixed ^ counter.wrapping_mul(0xBF58476D1CE4E5B9);
    format!("{value:012x}")
}

fn timestamp_compact() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = now / 86_400;
    let secs = now % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Days-since-epoch → (y, m, d). Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
