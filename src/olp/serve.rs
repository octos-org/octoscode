//! `octoscode olp remote serve-log --unit <单元号> --log <日志目录>` — the hub
//! side of a unit's access to the event log, run as an ssh forced command:
//!
//! ```text
//! command="/path/octoscode olp remote serve-log --unit m07 --log /srv/olp/proj/log",no-pty,no-port-forwarding ssh-ed25519 AAAA…
//! ```
//!
//! The unit is whatever that line says, never what the caller claims. What
//! the unit asks for arrives in `SSH_ORIGINAL_COMMAND`:
//!
//! - `ack <N> done|wontdo|blocked` (text on stdin) — only about a card
//!   dispatched to this unit;
//! - `progress <N> claimed` / `progress <N> review|rework <轮次>` (optional
//!   text on stdin) — a progress node, written once per node and round;
//! - `events [<之后的序号>]` — the events about this unit's cards, as JSON
//!   Lines. Other units' cards are not shown.
//!
//! Exit codes: 0 done; 3 refused (not this unit's card, unknown request,
//! bad arguments); 4 the log could not be read or written.

use std::io::Read;

use serde_json::json;

use super::cards;
use super::log::{AppendError, Draft, Event, Log};

/// A unit's text travels over ssh on stdin; this is plenty for one line.
const TEXT_LIMIT: u64 = 64 * 1024;
const REFUSED: i32 = 3;
const LOG_FAILED: i32 = 4;

fn is_unit(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Stdin as one trimmed line. `Err` when it is longer than a line may be
/// or holds a line break.
fn one_line() -> Result<String, String> {
    let mut text = String::new();
    std::io::stdin()
        .take(TEXT_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("读不了说明: {e}"))?;
    if text.len() as u64 > TEXT_LIMIT {
        return Err("说明太长(上限 64 KiB)".to_string());
    }
    let text = text.trim();
    if text.contains(['\n', '\r']) {
        return Err("说明必须是单行".to_string());
    }
    Ok(text.to_string())
}

/// `Err` unless card `n` exists, is not voided and is this unit's.
fn own_card(events: &[Event], n: u64, unit: &str) -> Result<(), String> {
    match cards::unit_of(events, n) {
        None => Err(format!("没有 #{n} 这张卡")),
        Some(owner) if owner.as_deref() != Some(unit) => Err(format!(
            "#{n} 派给的是 {},不是 {unit}",
            owner.as_deref().unwrap_or("未指定单元")
        )),
        Some(_)
            if events
                .iter()
                .any(|e| e.kind == "voided" && e.card == Some(n)) =>
        {
            Err(format!("#{n} 已被改判作废,不再接受写入"))
        }
        Some(_) => Ok(()),
    }
}

fn write(log: &Log, unit: &str, n: u64, draft: Draft) -> i32 {
    let result = log.append_with(|events| {
        own_card(events, n, unit)?;
        if let Some(key) = &draft.key {
            if events.iter().any(|e| e.key.as_ref() == Some(key)) {
                return Ok(Vec::new());
            }
        }
        Ok::<_, String>(vec![draft])
    });
    match result {
        Ok(written) if written.is_empty() => {
            println!("#{n}:已经记录过,没有重复写入");
            0
        }
        Ok(_) => {
            println!("#{n}:已记录");
            0
        }
        Err(AppendError::Refused(why)) => {
            eprintln!("olp-hub: 拒收:{why}");
            REFUSED
        }
        Err(AppendError::Log(e)) => {
            eprintln!("olp-hub: {e}");
            LOG_FAILED
        }
    }
}

/// `argv` is everything after `olp remote serve-log`.
pub fn run(argv: &[String]) -> i32 {
    let (mut unit, mut dir) = (None, None);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--unit" => unit = words.next().cloned(),
            "--log" => dir = words.next().cloned(),
            "-h" | "--help" => {
                println!(
                    "用法(ssh 强制命令): octoscode olp remote serve-log --unit <单元号> --log <日志目录>\n\n单元的请求来自 SSH_ORIGINAL_COMMAND:ack <N> <结果>、progress <N> <节点> [轮次]、events [序号]。"
                );
                return 0;
            }
            other => {
                eprintln!("olp remote serve-log: 不认识的参数: {other}");
                return 2;
            }
        }
    }
    let (Some(unit), Some(dir)) = (unit, dir) else {
        eprintln!("用法: octoscode olp remote serve-log --unit <单元号> --log <日志目录>");
        return 2;
    };
    if !is_unit(&unit) {
        eprintln!("olp remote serve-log: 单元号不合法: {unit}");
        return 2;
    }
    let refuse = |why: &str| {
        eprintln!("olp-hub: 拒收:{why}");
        REFUSED
    };
    let original = std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default();
    let Some(request) = shlex::split(&original) else {
        return refuse("请求无法解析");
    };
    let request: Vec<&str> = request.iter().map(String::as_str).collect();
    let log = Log::at(&dir);
    let card = |word: &str| word.parse::<u64>().ok().filter(|n| *n > 0);
    match request.as_slice() {
        ["ack", n, result] => {
            let Some(n) = card(n) else {
                return refuse("卡号不合法");
            };
            if !matches!(*result, "done" | "wontdo" | "blocked") {
                return refuse("结果只能是 done、wontdo 或 blocked");
            }
            let text = match one_line() {
                Ok(text) if !text.is_empty() => text,
                Ok(_) => return refuse("ACK 需要说明(done 附证据,wontdo 附理由,blocked 附原因)"),
                Err(why) => return refuse(&why),
            };
            let draft = Draft {
                kind: "ack".to_string(),
                card: Some(n),
                actor: format!("内环({unit})"),
                unit: Some(unit.clone()),
                key: None,
                data: json!({ "result": result, "text": text }),
            };
            write(&log, &unit, n, draft)
        }
        ["progress", n, rest @ ..] => {
            let Some(n) = card(n) else {
                return refuse("卡号不合法");
            };
            let (node, round) = match rest {
                ["claimed"] => ("claimed", 0),
                [node @ ("review" | "rework"), round] => {
                    match round.parse::<u32>().ok().filter(|r| (1..=999).contains(r)) {
                        Some(round) => (*node, round),
                        None => return refuse("轮次应为 1–999 的整数"),
                    }
                }
                _ => return refuse("节点只能是 claimed、review <轮次>、rework <轮次>"),
            };
            let text = match one_line() {
                Ok(text) => text,
                Err(why) => return refuse(&why),
            };
            let draft = Draft {
                kind: "progress".to_string(),
                card: Some(n),
                actor: format!("单元({unit})"),
                unit: Some(unit.clone()),
                // A node is reported once per card and round.
                key: Some(format!("{unit}:progress:{n}:{node}:{round}")),
                data: json!({ "node": node, "round": round, "text": text }),
            };
            write(&log, &unit, n, draft)
        }
        ["events"] | ["events", _] => {
            let since = match request.get(1) {
                None => 0,
                Some(word) => match word.parse::<u64>() {
                    Ok(since) => since,
                    Err(_) => return refuse("序号不合法"),
                },
            };
            let events = match log.read() {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("olp-hub: {e}");
                    return LOG_FAILED;
                }
            };
            for event in cards::visible_to(&events, &unit) {
                if event.seq > since {
                    match serde_json::to_string(event) {
                        Ok(line) => println!("{line}"),
                        Err(e) => {
                            eprintln!("olp-hub: {e}");
                            return LOG_FAILED;
                        }
                    }
                }
            }
            0
        }
        _ => refuse("这个密钥只能用于 ack、progress 与 events"),
    }
}
