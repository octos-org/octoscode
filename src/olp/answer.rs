//! `octoscode olp ask list|show|answer|serve` — the way back for a
//! question an inner loop asked with `ask_outer`.
//!
//! A question waits in the unit's mailbox (`<root>/mcp/questions/`); the
//! inner loop waits for `<root>/mcp/answers/<id>.json` for about 50
//! seconds, then goes on without it. On the hub, `list`, `show` and
//! `answer` reach the unit over ssh, where `serve` is the forced command;
//! when the unit is this machine they run `serve` here, with no ssh.
//!
//! Only a question still waiting can be answered: none is left in the
//! mailbox for nobody to read. `serve` reads and writes the mailbox only.
//!
//! Exit codes: 0; 2 usage; 3 the unit refused (no such question, too late,
//! action not allowed); 6 the outer-duty lease is no longer valid; 75 the
//! unit cannot be reached.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde_json::{Value, json};

use super::unit;

const USAGE: &str = "用法: octoscode olp ask <动作>
  hub 上:
    list <单元号> [--to <ssh 目标|local>]            还没回答的提问
    show <单元号> <提问id> [--to …]                  提问的全文
    answer <单元号> <提问id> [--to …]                回答从 stdin 喂
  单元上(ssh 强制命令):
    serve [--root <目录>] [--max-age 秒] [--list-window 秒]
退出码:0 成功;2 用法;3 单元拒绝;6 主审权已失效;75 单元不可达";

const MAX_ANSWER: usize = 64 * 1024;
const UNREACHABLE: i32 = 75;
const REFUSED: i32 = 3;

struct Fail(i32, String);

fn is_id(s: &str) -> bool {
    (4..=64).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// What `serve` printed and how it ended.
struct Served {
    code: i32,
    stdout: String,
    stderr: String,
}

// --- unit side -----------------------------------------------------------

struct Mailbox {
    root: PathBuf,
    max_age: f64,
    list_window: f64,
}

/// Pending questions in file-name order: (id, file, payload).
fn pending(dir: &Path) -> Vec<(String, PathBuf, Value)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    names.sort();
    let mut out: Vec<(String, PathBuf, Value)> = Vec::new();
    for path in names {
        let Some(payload) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            continue;
        };
        let Some(id) = payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|i| is_id(i))
        else {
            continue;
        };
        let id = id.to_string();
        // A later file with the same id replaces the earlier one.
        out.retain(|(seen, _, _)| *seen != id);
        out.push((id, path, payload));
    }
    out
}

fn age_of(path: &Path) -> f64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn serve(mailbox: &Mailbox, words: &[String], stdin: &[u8]) -> Served {
    let mut out = Served {
        code: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    let refuse = |why: String| Served {
        code: REFUSED,
        stdout: String::new(),
        stderr: format!("olp-answer: {why}\n"),
    };
    let asks = pending(&mailbox.root.join("mcp/questions"));
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["list"] => {
            let mut hidden = 0;
            for (id, path, payload) in &asks {
                let age = age_of(path);
                if age > mailbox.max_age && age > mailbox.list_window {
                    // Timed out long ago. The file stays in the mailbox;
                    // without a window the listing would only ever grow.
                    hidden += 1;
                    continue;
                }
                let head: String = payload
                    .get("question")
                    .map(|q| {
                        q.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| q.to_string())
                    })
                    .unwrap_or_default()
                    .replace('\n', " ")
                    .chars()
                    .take(120)
                    .collect();
                let state = if age <= mailbox.max_age {
                    "等待中"
                } else {
                    "已超时"
                };
                out.stdout
                    .push_str(&format!("{id}\t{state}\t{}s\t{head}\n", age as i64));
            }
            if hidden > 0 {
                out.stderr = format!(
                    "olp-answer: 另有 {hidden} 条提问早已超时(超过 {} 秒),未列出\n",
                    mailbox.list_window as i64
                );
            }
            out
        }
        [action @ ("show" | "answer"), id] if is_id(id) => {
            let Some((_, path, payload)) = asks.iter().find(|(seen, _, _)| seen == id) else {
                return refuse(format!("没有在等的提问 {id}(可能已回答,或写错了 id)"));
            };
            if *action == "show" {
                // Only the question's own fields, never the file as it is.
                let mut shown = serde_json::Map::new();
                for key in ["id", "ts", "question", "context", "tried"] {
                    shown.insert(
                        key.to_string(),
                        payload.get(key).cloned().unwrap_or_else(|| json!("")),
                    );
                }
                let ordered = ["id", "ts", "question", "context", "tried"]
                    .iter()
                    .map(|k| format!("  {}: {}", json!(k), shown[*k]))
                    .collect::<Vec<_>>()
                    .join(",\n");
                out.stdout = format!("{{\n{ordered}\n}}\n");
                return out;
            }
            let age = age_of(path);
            if age > mailbox.max_age {
                // The file is still there, but the inner loop has gone on:
                // nobody would read the answer.
                return refuse(format!(
                    "提问 {id} 已等了 {} 秒,超过内环的等待时间({} 秒);它已按降级指引继续。请改用黑板条目或 herdr 告诉这台单元",
                    age as i64, mailbox.max_age as i64
                ));
            }
            if stdin.len() > MAX_ANSWER {
                return refuse("回答超过 64 KiB".to_string());
            }
            let Ok(text) = std::str::from_utf8(stdin) else {
                return refuse("回答不是 UTF-8".to_string());
            };
            if text.trim().is_empty() {
                return refuse("回答为空".to_string());
            }
            let answers = mailbox.root.join("mcp/answers");
            let answer = json!({
                "answer": text.trim(),
                "ts": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "via": "olp-answer",
            });
            let written = std::fs::create_dir_all(&answers).and_then(|()| {
                let mut tmp = tempfile::Builder::new()
                    .prefix(".answer-")
                    .tempfile_in(&answers)?;
                tmp.write_all(answer.to_string().as_bytes())?;
                tmp.as_file().sync_all()?;
                tmp.persist(answers.join(format!("{id}.json")))
                    .map_err(|e| e.error)?;
                Ok(())
            });
            match written {
                Ok(()) => out.stdout = format!("answered {id}\n"),
                Err(e) => return refuse(format!("写不进信箱: {e}")),
            }
            out
        }
        _ => refuse("这个密钥只能用于 list、show <id>、answer <id>".to_string()),
    }
}

fn mailbox_from(argv: &[String]) -> Result<Mailbox, Fail> {
    let mut mailbox = Mailbox {
        // The MCP server's own default, so an answer on a single machine
        // reaches the mailbox the inner loop's server reads.
        root: std::env::var_os("OLP_MCP_OUTER_ROOT")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".octos/outer")
            }),
        max_age: 50.0,
        list_window: 86400.0,
    };
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| words.next().cloned())
                .ok_or_else(|| Fail(2, format!("{name} 需要一个值")))
        };
        let seconds = |text: String| {
            text.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| Fail(2, format!("{name} 必须是数字")))
        };
        match name {
            "--root" => mailbox.root = PathBuf::from(value()?),
            "--max-age" => mailbox.max_age = seconds(value()?)?,
            "--list-window" => mailbox.list_window = seconds(value()?)?,
            other => return Err(Fail(2, format!("不认识的参数: {other}"))),
        }
    }
    Ok(mailbox)
}

// --- hub side ------------------------------------------------------------

/// Run the unit's side: here when the unit is this machine, else over ssh.
fn relay(unit_name: &str, to: Option<&str>, request: &[&str], stdin: &[u8]) -> Result<i32, Fail> {
    if !unit::is_unit(unit_name) {
        return Err(Fail(2, format!("单元号不合法: {unit_name}")));
    }
    let local = unit::is_local(unit_name) == Some(true);
    let target = to.unwrap_or(if local { "local" } else { unit_name });
    if !unit::is_target(target) {
        return Err(Fail(2, format!("--to 不是合法的 ssh 目标: {target}")));
    }
    let served = if target == "local" {
        let words: Vec<String> = request.iter().map(|w| w.to_string()).collect();
        serve(&mailbox_from(&[])?, &words, stdin)
    } else {
        let ssh = std::env::var("OLP_SSH")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "ssh".to_string());
        let unreachable = |why: String| Served {
            code: 255,
            stdout: String::new(),
            stderr: why,
        };
        let mut cmd = Command::new(ssh);
        cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", target])
            .args(request);
        // The whole request has a deadline, not only the connection.
        match super::proc::run(cmd, Some(stdin), super::proc::ssh_limit()) {
            Err(super::proc::RunError::Spawn(e)) => unreachable(format!("无法运行 ssh: {e}")),
            Err(super::proc::RunError::Timeout) => unreachable(format!(
                "{} 秒内没有答复",
                super::proc::ssh_limit().as_secs()
            )),
            Ok(done) => Served {
                code: done.code.unwrap_or(255),
                stdout: done.stdout_text(),
                stderr: done.stderr_text(),
            },
        }
    };
    if served.code == 255 {
        eprintln!(
            "olp-answer: 单元 {unit_name} 不可达:{}",
            served.stderr.trim()
        );
        return Ok(UNREACHABLE);
    }
    print!("{}", served.stdout);
    if !served.stderr.trim().is_empty() {
        eprintln!("{}", served.stderr.trim());
    }
    Ok(served.code)
}

fn hub(action: &str, argv: &[String]) -> Result<i32, Fail> {
    let mut to = None;
    let mut rest = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        if word == "--to" {
            to = Some(
                words
                    .next()
                    .cloned()
                    .ok_or_else(|| Fail(2, "--to 需要一个值".to_string()))?,
            );
        } else if let Some(value) = word.strip_prefix("--to=") {
            to = Some(value.to_string());
        } else if word.starts_with("--") {
            return Err(Fail(2, format!("不认识的参数: {word}")));
        } else {
            rest.push(word.as_str());
        }
    }
    let wanted = if action == "list" { 1 } else { 2 };
    if rest.len() != wanted {
        return Err(Fail(2, format!("参数个数不对\n{USAGE}")));
    }
    if wanted == 2 && !is_id(rest[1]) {
        return Err(Fail(2, format!("提问 id 不合法: {}", rest[1])));
    }
    let mut body = Vec::new();
    if action == "answer" {
        super::duty::gate("olp-answer").map_err(|code| Fail(code, String::new()))?;
        let _ = std::io::stdin()
            .take(MAX_ANSWER as u64 + 1)
            .read_to_end(&mut body);
        if body.len() > MAX_ANSWER {
            return Err(Fail(2, "回答超过 64 KiB".to_string()));
        }
        if body.iter().all(u8::is_ascii_whitespace) {
            return Err(Fail(2, "回答为空(从 stdin 喂)".to_string()));
        }
    }
    let mut request = vec![action];
    request.extend(&rest[1..]);
    relay(rest[0], to.as_deref(), &request, &body)
}

/// `argv` is what follows `olp ask` (the `mcp` action is handled elsewhere).
pub fn run(argv: &[String]) -> i32 {
    let (action, rest) = match argv.split_first() {
        Some((a, rest)) if !matches!(a.as_str(), "-h" | "--help") => (a.as_str(), rest),
        Some(_) => {
            println!("{USAGE}");
            return 0;
        }
        None => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    if rest.iter().any(|w| w == "-h" || w == "--help") {
        println!("{USAGE}");
        return 0;
    }
    let result = match action {
        "list" | "show" | "answer" => hub(action, rest),
        "serve" => mailbox_from(rest).map(|mailbox| {
            let words = shlex::split(&std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default())
                .unwrap_or_default();
            let mut stdin = Vec::new();
            if words.first().map(String::as_str) == Some("answer") {
                let _ = std::io::stdin()
                    .take(MAX_ANSWER as u64 + 1)
                    .read_to_end(&mut stdin);
            }
            let served = serve(&mailbox, &words, &stdin);
            print!("{}", served.stdout);
            eprint!("{}", served.stderr);
            served.code
        }),
        other => Err(Fail(2, format!("没有这个动作: {other}\n{USAGE}"))),
    };
    match result {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            if !why.is_empty() {
                eprintln!("olp-answer: {why}");
            }
            code
        }
    }
}
