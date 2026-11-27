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
//! - `ack <N> done|wontdo|blocked [<键>]` (text on stdin) — only about a
//!   card dispatched to this unit; with a key (`<unit>:…`) a retry is
//!   recorded once;
//! - `progress <N> claimed` / `progress <N> review|rework <轮次>` (optional
//!   text on stdin) — a progress node, written once per node and round;
//!   `progress <N> undelivered` is the unit's watcher saying the pane did
//!   not take the card;
//! - `question ask|answer|timeout <id>` (the question on stdin for `ask`) —
//!   a question an inner loop asked with `ask_outer`, and how it ended;
//! - `alert <键> <任务标签>` (one line on stdin) — what the unit's runtime
//!   guard saw (`olp guard forward`), recorded once per key;
//! - `inbox [<窗格>]` — this unit's cards that were dispatched to a pane and
//!   not claimed yet (`progress <N> claimed` claims one);
//! - `events [<之后的序号>]` — the events about this unit's cards, as JSON
//!   Lines. Other units' cards are not shown.
//!
//! Exit codes: 0 done; 3 refused (not this unit's card, unknown request,
//! bad arguments); 4 the log could not be read or written.

use std::io::Read;

use serde_json::json;

use super::cards;
use super::log::{AppendError, Draft, Event, Log};
use super::unit::is_unit;

/// A unit's text travels over ssh on stdin; this is plenty for one line.
const TEXT_LIMIT: u64 = 64 * 1024;
const REFUSED: i32 = 3;
const LOG_FAILED: i32 = 4;

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

/// `Ok(true)`: `key` already stands for this very thing (a retry).
/// `Err`: it stands for something else, which a retry never is.
fn said_before(events: &[Event], key: &str, kind: &str, card: Option<u64>) -> Result<bool, String> {
    match events.iter().find(|e| e.key.as_deref() == Some(key)) {
        None => Ok(false),
        Some(first) if first.kind == kind && first.card == card => Ok(true),
        Some(first) => Err(format!(
            "键 {key} 已用于第 {} 条事件,与这次要写的不是同一件事",
            first.seq
        )),
    }
}

/// Append `draft` about the unit's card `n`. `per_round`: the key counts
/// within one round of the card — after the outer loop returned the card,
/// the same words are a new answer, not a retry of the old one.
fn write(log: &Log, unit: &str, n: u64, mut draft: Draft, per_round: bool) -> i32 {
    let result = log.append_with(|events| {
        own_card(events, n, unit)?;
        if per_round {
            let returns = events
                .iter()
                .filter(|e| e.card == Some(n) && e.kind == "returned")
                .count();
            if returns > 0 {
                draft.key = draft.key.take().map(|key| format!("{key}:r{returns}"));
            }
        }
        if let Some(key) = &draft.key {
            if said_before(events, key, &draft.kind, draft.card)? {
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
                    "用法(ssh 强制命令): octoscode olp remote serve-log --unit <单元号> --log <日志目录>\n\n单元的请求来自 SSH_ORIGINAL_COMMAND:ack <N> <结果>、progress <N> <节点> [轮次]、question <ask|answer|timeout> <提问id>、alert <键> <任务标签>、inbox [窗格]、events [序号]。"
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
        ["ack", n, result, key @ ..] if key.len() <= 1 => {
            // A key lets a retried ACK be recorded once. It is the unit's
            // own: `<unit>:…`.
            let key = match key {
                [] => None,
                [key]
                    if key.strip_prefix(&format!("{unit}:")).is_some_and(|rest| {
                        (1..=180).contains(&rest.len())
                            && rest
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || "._:/@+=-".contains(c))
                    }) =>
                {
                    Some(key.to_string())
                }
                _ => return refuse(&format!("键必须以 {unit}: 开头且只含 [A-Za-z0-9._:/@+=-]")),
            };
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
                key,
                data: json!({ "result": result, "text": text }),
            };
            write(&log, &unit, n, draft, true)
        }
        ["progress", n, rest @ ..] => {
            let Some(n) = card(n) else {
                return refuse("卡号不合法");
            };
            let (node, round) = match rest {
                ["claimed"] => ("claimed", 0),
                // The unit's watcher: the pane did not take the card.
                ["undelivered"] => ("undelivered", 0),
                [node @ ("review" | "rework"), round] => {
                    match round.parse::<u32>().ok().filter(|r| (1..=999).contains(r)) {
                        Some(round) => (*node, round),
                        None => return refuse("轮次应为 1–999 的整数"),
                    }
                }
                _ => {
                    return refuse("节点只能是 claimed、undelivered、review <轮次>、rework <轮次>");
                }
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
            write(&log, &unit, n, draft, false)
        }
        ["alert", key, label] => {
            // What the unit's runtime guard saw (`olp guard forward`). It
            // is the unit's own word about itself; a card it names is
            // linked only when that card is this unit's.
            let own_key = key.strip_prefix(&format!("{unit}:")).is_some_and(|rest| {
                (1..=180).contains(&rest.len())
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._:/@+=-".contains(c))
            });
            if !own_key {
                return refuse(&format!("键必须以 {unit}: 开头且只含 [A-Za-z0-9._:/@+=-]"));
            }
            let plain = (1..=41).contains(&label.len())
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "#._:/@+=-".contains(c));
            if !plain {
                return refuse("告警的任务标签不合法");
            }
            let text = match one_line() {
                Ok(text) if !text.is_empty() => text,
                Ok(_) => return refuse("告警的内容为空"),
                Err(why) => return refuse(&why),
            };
            let named = label.strip_prefix('#').and_then(card);
            let result = log.append_with(|events| {
                let card = named.filter(|n| own_card(events, *n, &unit).is_ok());
                if said_before(events, key, "alert", card)? {
                    return Ok(Vec::new());
                }
                Ok::<_, String>(vec![Draft {
                    kind: "alert".to_string(),
                    card,
                    actor: format!("单元({unit})·负哨({label})"),
                    unit: Some(unit.clone()),
                    key: Some(key.to_string()),
                    data: json!({ "text": text }),
                }])
            });
            match result {
                Ok(written) if written.is_empty() => {
                    println!("告警:已经记录过,没有重复写入");
                    0
                }
                Ok(_) => {
                    println!("告警:已记录");
                    0
                }
                Err(AppendError::Refused(why)) => refuse(&why),
                Err(AppendError::Log(e)) => {
                    eprintln!("olp-hub: {e}");
                    LOG_FAILED
                }
            }
        }
        ["question", kind @ ("ask" | "answer" | "timeout"), id] => {
            // A question an inner loop asked with `ask_outer`, and how it
            // ended. It is the unit's own and belongs to no card.
            let valid = (4..=64).contains(&id.len())
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid {
                return refuse("提问 id 不合法");
            }
            let question = match one_line() {
                Ok(text) => text,
                Err(why) => return refuse(&why),
            };
            let (event_kind, data) = match *kind {
                "ask" if question.is_empty() => return refuse("提问的内容为空"),
                "ask" => ("asked", json!({ "id": id, "question": question })),
                "answer" => ("answered", json!({ "id": id })),
                _ => ("timed_out", json!({ "id": id })),
            };
            let draft = Draft {
                kind: event_kind.to_string(),
                card: None,
                actor: format!("单元({unit})"),
                unit: Some(unit.clone()),
                key: Some(format!("{unit}:question:{id}:{kind}")),
                data,
            };
            let result = log.append_with(|events| {
                let about = |e: &&Event| {
                    e.unit.as_deref() == Some(unit.as_str())
                        && e.data.get("id").and_then(|v| v.as_str()) == Some(*id)
                };
                if let Some(key) = &draft.key {
                    if said_before(events, key, &draft.kind, None)? {
                        return Ok(Vec::new());
                    }
                }
                if *kind != "ask" {
                    if !events.iter().filter(about).any(|e| e.kind == "asked") {
                        return Err(format!("没有 {id} 这个提问"));
                    }
                    if events
                        .iter()
                        .filter(about)
                        .any(|e| e.kind == "answered" || e.kind == "timed_out")
                    {
                        return Ok(Vec::new());
                    }
                }
                Ok::<_, String>(vec![draft.clone()])
            });
            match result {
                Ok(written) if written.is_empty() => {
                    println!("提问 {id}:已经记录过,没有重复写入");
                    0
                }
                Ok(_) => {
                    println!("提问 {id}:已记录");
                    0
                }
                Err(AppendError::Refused(why)) => refuse(&why),
                Err(AppendError::Log(e)) => {
                    eprintln!("olp-hub: {e}");
                    LOG_FAILED
                }
            }
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
        ["inbox"] | ["inbox", _] => {
            let pane = request.get(1).copied();
            if pane.is_some_and(|p| !super::card::is_pane(p)) {
                return refuse("窗格标识不合法");
            }
            let events = match log.read() {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("olp-hub: {e}");
                    return LOG_FAILED;
                }
            };
            let items: Vec<cards::Pending> = cards::pending_for(&events, &unit)
                .into_iter()
                .filter(|item| pane.is_none_or(|p| p == item.pane))
                .collect();
            let held: Vec<cards::Held> = cards::held_by(&events, &unit, chrono::Utc::now())
                .into_iter()
                .filter(|card| pane.is_none_or(|p| p == card.pane))
                .collect();
            println!("{}", json!({ "unit": unit, "items": items, "held": held }));
            0
        }
        _ => refuse("这个密钥只能用于 ack、progress、question、alert、events 与 inbox"),
    }
}
