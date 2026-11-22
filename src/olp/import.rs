//! `octoscode olp board import` — a Markdown board into a new event log.
//! One-time: the board is read by the same rules the inbox uses, and what
//! it holds becomes events in the order of the lines they came from, so the
//! log classifies every card the way the board did.

use std::path::Path;

use serde_json::json;

use super::board::{self, Ack, EventKind};
use super::log::{AppendError, Draft, Log};

/// What an import would write, and what it leaves behind.
pub struct Plan {
    pub drafts: Vec<Draft>,
    pub cards: usize,
    /// Lines the log has no place for (verdicts about cards the board does
    /// not have, duplicate numbers).
    pub left_out: Vec<String>,
}

const ACTOR: &str = "导入";

pub fn plan(text: &str) -> Plan {
    let scan = board::scan(text);
    let bodies = board::entry_bodies(text);
    let raw: Vec<&str> = board::lines(text).map(|(_, line)| line).collect();
    let mut lines: Vec<(usize, Draft)> = Vec::new();
    let draft = |kind: &str, card: Option<u64>, unit: Option<String>, line: usize| Draft {
        kind: kind.to_string(),
        card,
        actor: ACTOR.to_string(),
        unit,
        key: None,
        // Where it came from, and the line as written (a rendered view
        // repeats it when it stood outside every card).
        data: json!({ "line": line, "text": raw.get(line.wrapping_sub(1)).copied().unwrap_or("") }),
    };
    let with = |mut d: Draft, fields: serde_json::Value| {
        if let (Some(data), Some(extra)) = (d.data.as_object_mut(), fields.as_object()) {
            data.extend(extra.clone());
        }
        d
    };
    for (number, entry) in &scan.entries {
        let dispatched = draft("dispatched", Some(*number), entry.unit.clone(), entry.line);
        lines.push((
            entry.line,
            with(
                dispatched,
                json!({
                    "title": entry.title,
                    "span": bodies.get(&entry.line).map_or(0, |b| b.split_inclusive('\n').count()),
                    "body": bodies.get(&entry.line).cloned().unwrap_or_default(),
                    "impl": entry.implementer,
                    "review": entry.review,
                }),
            ),
        ));
        for (line, kind) in &entry.acks {
            let result = match kind {
                Ack::Done => "done",
                Ack::Wontdo => "wontdo",
                Ack::Blocked => "blocked",
            };
            let ack = draft("ack", Some(*number), None, *line);
            lines.push((*line, with(ack, json!({ "result": result }))));
        }
        if let Some(line) = entry.archive_line {
            lines.push((line, draft("archived", Some(*number), None, line)));
        }
    }
    for (number, events) in &scan.events {
        if !scan.entries.contains_key(number) {
            continue; // reported through the scan's warnings below
        }
        for event in events {
            let line = event.line;
            let d = match &event.kind {
                EventKind::Accepted => draft("accepted", Some(*number), None, line),
                EventKind::Returned => draft("returned", Some(*number), None, line),
                EventKind::Voided => draft("voided", Some(*number), None, line),
                EventKind::Snooze(text) => with(
                    draft("snoozed", Some(*number), None, line),
                    json!({ "text": text }),
                ),
                EventKind::Pushed(unit) => draft("pushed", Some(*number), Some(unit.clone()), line),
                EventKind::Dangling | EventKind::Claimed | EventKind::Undelivered => continue,
            };
            lines.push((line, d));
        }
    }
    for q in board::question_lines(text) {
        let kind = match q.closes {
            None => "asked",
            Some("answer") => "answered",
            Some(_) => "timed_out",
        };
        let d = draft(kind, None, q.question.unit.clone(), q.question.line);
        lines.push((
            q.question.line,
            with(
                d,
                json!({ "id": q.question.id, "question": q.question.question }),
            ),
        ));
    }
    lines.sort_by_key(|(line, _)| *line);
    Plan {
        drafts: lines.into_iter().map(|(_, d)| d).collect(),
        cards: scan.entries.len(),
        left_out: scan.warnings,
    }
}

const USAGE: &str = "用法: octoscode olp board import <黑板.md> --to <日志目录> [--dry-run]";

/// `argv` is everything after `olp board import`. Exit codes: 0 imported
/// (or would be); 1 the target log already holds events, or cannot be
/// written; 2 usage; 6 the outer-duty lease is no longer valid.
pub fn run(argv: &[String]) -> i32 {
    let (mut board_path, mut to, mut dry_run) = (None, None, false);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "-h" | "--help" => {
                println!(
                    "{USAGE}\n\n把一块 Markdown 黑板一次性导入成新的事件日志(JSON Lines)。\n日志里已有事件时拒绝;黑板文件不会被改动。"
                );
                return 0;
            }
            "--dry-run" => dry_run = true,
            "--to" => to = words.next().cloned(),
            other if other.starts_with('-') => {
                eprintln!("olp board import: 不认识的参数: {other}\n{USAGE}");
                return 2;
            }
            other if board_path.is_none() => board_path = Some(other.to_string()),
            other => {
                eprintln!("olp board import: 多余的参数: {other}\n{USAGE}");
                return 2;
            }
        }
    }
    let (Some(board_path), Some(to)) = (board_path, to) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let bytes = match std::fs::read(&board_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("olp board import: 读不了黑板 {board_path}: {e}");
            return 2;
        }
    };
    let plan = plan(&String::from_utf8_lossy(&bytes));
    for line in &plan.left_out {
        eprintln!("warning: 未导入:{line}");
    }
    let summary = format!("{} 张卡、{} 条事件", plan.cards, plan.drafts.len());
    if dry_run {
        println!("dry-run:会导入 {summary} 到 {to},未写入任何文件");
        return 0;
    }
    if let Err(code) = super::duty::gate("olp board import") {
        return code;
    }
    let log = Log::at(Path::new(&to));
    let drafts = plan.drafts;
    match log.append_with(|existing| {
        if existing.is_empty() {
            Ok(drafts)
        } else {
            Err(existing.len())
        }
    }) {
        Ok(_) => {
            println!("已导入 {summary} 到 {to}");
            0
        }
        Err(AppendError::Refused(count)) => {
            eprintln!("olp board import: {to} 里已有 {count} 条事件,不重复导入");
            1
        }
        Err(AppendError::Log(e)) => {
            eprintln!("olp board import: {e}");
            1
        }
    }
}
