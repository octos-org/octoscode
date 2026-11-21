//! `octoscode olp board render` — the event log as a Markdown board, for
//! people and agents to read. Read-only: writes go through commands.
//!
//! The view is a board the Markdown tools read to the same conclusions as
//! the log (the tests hold it to that): cards and signed lines appear in
//! event order, as a board that was only ever appended to.

use super::log::{Event, Log};

pub const IMPORT_ACTOR: &str = "导入";

fn text<'a>(event: &'a Event, field: &str) -> &'a str {
    event.data.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

fn number(event: &Event, field: &str) -> usize {
    event.data.get(field).and_then(|v| v.as_u64()).unwrap_or(0) as usize
}

/// The signed line a written (not imported) event reads as.
fn signed_line(event: &Event) -> Option<String> {
    let card = event.card;
    let say = text(event, "text");
    let colon = |s: &str| {
        if s.is_empty() {
            String::new()
        } else {
            format!(": {s}")
        }
    };
    Some(match (event.kind.as_str(), card) {
        ("ack", Some(n)) => format!(
            "> {}·#{n} ACK({}){}",
            event.actor,
            text(event, "result"),
            colon(say)
        ),
        ("accepted", Some(n)) => format!("> {}·采认(#{n}){}", event.actor, colon(say)),
        ("returned", Some(n)) => format!("> {}·打回(#{n}){}", event.actor, colon(say)),
        ("voided", Some(n)) => format!("> {}·改判(作废 #{n}){}", event.actor, colon(say)),
        // A snooze's text already starts with ` 至 YYYY-MM-DD`.
        ("snoozed", Some(n)) => format!("> {}·搁置(#{n}){say}", event.actor),
        ("pushed", Some(n)) => format!("> {}·本地复验通过(#{n}){}", event.actor, colon(say)),
        ("asked", _) => format!(
            "> {}·- MCP(ask_outer) ask: id={} question={}",
            event.actor,
            text(event, "id"),
            text(event, "question")
        ),
        ("answered", _) => format!(
            "> {}·- MCP(ask_outer) answer: id={}",
            event.actor,
            text(event, "id")
        ),
        ("timed_out", _) => format!(
            "> {}·- MCP(ask_outer) timeout: id={}",
            event.actor,
            text(event, "id")
        ),
        _ => return None,
    })
}

/// The Markdown view of `events`.
pub fn render(events: &[Event]) -> String {
    let mut out = String::from(
        "# OctoLoop 黑板\n\n> 本文件由事件日志生成,只读。写入用 `octoscode olp card …`;手工改动不会生效。\n\n## Active\n\n",
    );
    // Source-line ranges covered by imported card texts: a line inside one
    // is already in that card's text and is not repeated.
    let spans: Vec<(usize, usize)> = events
        .iter()
        .filter(|e| e.kind == "dispatched" && e.actor == IMPORT_ACTOR)
        .map(|e| (number(e, "line"), number(e, "line") + number(e, "span")))
        .collect();
    let covered = |line: usize| {
        spans
            .iter()
            .any(|(start, end)| line > *start && line <= *end)
    };
    let mut seen = std::collections::BTreeSet::new();
    for event in events {
        let imported = event.actor == IMPORT_ACTOR;
        if event.kind == "dispatched" {
            let Some(card) = event.card else { continue };
            if !seen.insert(card) {
                continue;
            }
            if !out.ends_with("\n\n") {
                out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
            }
            out.push_str(&format!("### {card}. {}\n", text(event, "title")));
            if !imported {
                if let Some(unit) = &event.unit {
                    out.push_str(&format!("unit: {unit}\n"));
                }
                if !text(event, "impl").is_empty() {
                    out.push_str(&format!("impl: {}\n", text(event, "impl")));
                }
                let review: Vec<&str> = event
                    .data
                    .get("review")
                    .and_then(|v| v.as_array())
                    .map(|names| names.iter().filter_map(|n| n.as_str()).collect())
                    .unwrap_or_default();
                if !review.is_empty() {
                    out.push_str(&format!("review: {}\n", review.join(", ")));
                }
                out.push('\n');
            }
            out.push_str(text(event, "body"));
            if !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        let line = if imported {
            if covered(number(event, "line")) || text(event, "text").is_empty() {
                continue;
            }
            text(event, "text").to_string()
        } else {
            match signed_line(event) {
                Some(line) => line,
                None => continue,
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// `argv` is everything after `olp board render`: the log directory.
pub fn run(argv: &[String]) -> i32 {
    let dir = match argv {
        [dir] if !dir.starts_with('-') => dir,
        [flag] if flag == "-h" || flag == "--help" => {
            println!(
                "用法: octoscode olp board render <日志目录>\n\n把事件日志输出成 Markdown 黑板(只读视图),写到标准输出。"
            );
            return 0;
        }
        _ => {
            eprintln!("用法: octoscode olp board render <日志目录>");
            return 2;
        }
    };
    if !std::path::Path::new(dir).is_dir() {
        eprintln!("olp board render: 日志目录不存在: {dir}");
        return 2;
    }
    match Log::at(dir).read() {
        Ok(events) => {
            print!("{}", render(&events));
            0
        }
        Err(e) => {
            eprintln!("olp board render: {e}");
            1
        }
    }
}
