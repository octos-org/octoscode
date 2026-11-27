//! `octoscode olp log compact <日志目录>` — move what is over out of the
//! log's way, and `octoscode olp log history <日志目录>` — read all of it.
//!
//! Every command replays the log, so the log's size is every command's
//! cost. Compaction is the checkpoint of an event-sourced store: what is
//! still needed is written to a base, the segments it was read from move
//! to `archive/` byte for byte, and the log goes on in a new segment.
//! Sequence numbers are never reused or renumbered — after a compaction
//! there are gaps where archived events were — so a watcher's "read up to"
//! stays valid.
//!
//! What goes: every event of a card that is settled, whose last event is
//! older than `--keep-days`, that is not among the `--keep` most recently
//! settled, that its unit is not still waiting to claim, and that no card
//! which stays is a re-test of; and events about
//! no card that are that old, unless they ask a question still open. What
//! is left of such a card is its number, its unit, its title and the key
//! it was opened with: the number is not given out again, the key still
//! stands for that card, and a late write about it is refused.
//!
//! Before anything is replaced, the kept events are replayed and compared
//! with the whole log: the same cards in the same states, the same open
//! questions, the same work waiting for and held by every unit. Any
//! difference and nothing is written.
//!
//! Exit codes: 0 (compacted, or nothing to do); 2 usage; 4 the log could
//! not be read or written; 5 the kept events would read differently.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::json;

use super::cards;
use super::inbox::items;
use super::log::{AppendError, Event, Gone, Log, Rebase};

const USAGE: &str = "用法: octoscode olp log compact <日志目录> [--keep-days 14] [--keep 10] [--if-over MiB] [--dry-run] [--json]
      octoscode olp log history <日志目录> [--card N] [--json]

compact:把已了结的卡移出日常要读的日志。已了结、最后一条事件早于 --keep-days 天、
不在最近了结的 --keep 张之内、单元已领取、也不是某张未归档卡的回测对象的卡,全部事件移入归档;
与卡无关的事件过了 --keep-days 天也移入(还没回答的提问除外)。卡号、立卡用的键保留:
卡号不会再发出去,同一个键仍指那张卡,之后再写这张卡会被拒收。
旧分段原样搬到 <日志目录>/archive/,序号不重排。替换前先核对:留下的事件算出的
卡状态、未答提问、各单元的待办与在办,必须与完整日志一致,否则不写。
--if-over:在用的日志不到这么多 MiB、或不到上次压缩后的两倍时什么都不做(定时任务这样用)。
--dry-run:只说会移走什么。
history:按序输出完整历史(归档 + 在用),--card 只看一张卡。
退出码:0;2 用法;4 日志读写失败;5 核对不一致,没有写";

/// What a compaction would keep and what it would move away.
pub struct Plan {
    pub keep: Vec<Event>,
    pub gone: Vec<Gone>,
}

fn at(event: &Event) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&event.ts)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Decide what stays. `keep_days`: how long a settled card stays after
/// its last event; `keep_recent`: how many of the last settled cards stay
/// whatever their age.
pub fn plan(events: &[Event], now: DateTime<Utc>, keep_days: i64, keep_recent: usize) -> Plan {
    let today = now.with_timezone(&chrono::Local).date_naive();
    let cutoff = now - chrono::Duration::days(keep_days);
    // An event without a readable time is never old.
    let old = |event: &Event| at(event).is_some_and(|at| at <= cutoff);
    let settled = cards::settled(events, today);
    // Per card: its opening event and its last one.
    let mut first: BTreeMap<u64, &Event> = BTreeMap::new();
    let mut last: BTreeMap<u64, &Event> = BTreeMap::new();
    for event in events {
        let Some(card) = event.card else { continue };
        if event.kind == "dispatched" {
            first.entry(card).or_insert(event);
        }
        last.insert(card, event);
    }
    let mut by_end: Vec<u64> = settled.iter().copied().collect();
    by_end.sort_by_key(|card| std::cmp::Reverse(last.get(card).map_or(0, |e| e.seq)));
    let recent: BTreeSet<u64> = by_end.into_iter().take(keep_recent).collect();
    // A card its unit was sent and never claimed is still in that unit's
    // delivery inbox, settled or not: it stays where the unit reads it.
    let units: BTreeSet<&str> = events.iter().filter_map(|e| e.unit.as_deref()).collect();
    let undelivered: BTreeSet<u64> = units
        .iter()
        .flat_map(|unit| cards::pending_for(events, unit))
        .map(|pending| pending.number)
        .collect();
    let mut going: BTreeSet<u64> = settled
        .iter()
        .copied()
        .filter(|card| !recent.contains(card) && first.contains_key(card))
        .filter(|card| !undelivered.contains(card))
        .filter(|card| last.get(card).is_some_and(|e| old(e)))
        .collect();
    // A card that stays keeps the card it is a re-test of.
    loop {
        let needed: Vec<u64> = first
            .iter()
            .filter(|(card, _)| !going.contains(card))
            .filter_map(|(_, opened)| opened.data.get("verify_of").and_then(|v| v.as_u64()))
            .filter(|original| going.contains(original))
            .collect();
        if needed.is_empty() {
            break;
        }
        for original in needed {
            going.remove(&original);
        }
    }
    let open_questions: BTreeSet<String> = cards::pending_questions(events)
        .into_iter()
        .map(|question| question.id)
        .collect();
    let stays = |event: &Event| match event.card {
        Some(card) => !going.contains(&card),
        None => {
            let asks_open = event.kind == "asked"
                && event
                    .data
                    .get("id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| open_questions.contains(id));
            asks_open || !old(event)
        }
    };
    let gone = going
        .iter()
        .filter_map(|card| {
            let (opened, ended) = (first.get(card)?, last.get(card)?);
            Some(Gone {
                card: *card,
                unit: opened.unit.clone(),
                title: opened
                    .data
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                key: opened.key.clone(),
                first: opened.seq,
                last: ended.seq,
                ended: ended.ts.clone(),
            })
        })
        .collect();
    Plan {
        keep: events.iter().filter(|e| stays(e)).cloned().collect(),
        gone,
    }
}

/// Do the kept events read as the whole log does, the archived cards
/// apart? `Err` names the first difference.
pub fn same_reading(events: &[Event], plan: &Plan, now: DateTime<Utc>) -> Result<(), String> {
    let today = now.with_timezone(&chrono::Local).date_naive();
    let gone: BTreeSet<u64> = plan.gone.iter().map(|g| g.card).collect();
    let states = |events: &[Event]| -> Vec<serde_json::Value> {
        let mut scan = cards::scan(events);
        items(&mut scan, today, |_| true)
            .into_iter()
            .filter(|item| !gone.contains(&item.number))
            .map(|item| json!(item))
            .collect()
    };
    let whole = cards::settled(events, today);
    if let Some(open) = gone.iter().find(|card| !whole.contains(card)) {
        return Err(format!("#{open} 没有了结,却要归档"));
    }
    let (before, after) = (states(events), states(&plan.keep));
    if before != after {
        let differs = before
            .iter()
            .zip(&after)
            .find(|(a, b)| a != b)
            .map(|(a, _)| a["number"].to_string())
            .unwrap_or_else(|| "数量".to_string());
        return Err(format!("卡的状态不一致(#{differs})"));
    }
    let questions = |events: &[Event]| -> Vec<(String, Option<String>, String, usize)> {
        cards::pending_questions(events)
            .into_iter()
            .map(|q| (q.id, q.unit, q.question, q.line))
            .collect()
    };
    if questions(events) != questions(&plan.keep) {
        return Err("未答的提问不一致".to_string());
    }
    let units: BTreeSet<&str> = events.iter().filter_map(|e| e.unit.as_deref()).collect();
    for unit in units {
        if cards::pending_for(events, unit) != cards::pending_for(&plan.keep, unit) {
            return Err(format!("单元 {unit} 的待办不一致"));
        }
        if cards::held_by(events, unit, now) != cards::held_by(&plan.keep, unit, now) {
            return Err(format!("单元 {unit} 在办的卡不一致"));
        }
    }
    let ever = |events: &[Event]| {
        events
            .iter()
            .filter(|e| e.kind == "dispatched")
            .filter_map(|e| e.card)
            .max()
    };
    if ever(&plan.keep).max(gone.iter().next_back().copied()) != ever(events) {
        return Err("最大卡号不一致".to_string());
    }
    Ok(())
}

/// The outcome of one compaction, for people and for `--json`.
#[derive(serde::Serialize)]
pub struct Outcome {
    pub compacted: bool,
    pub dry_run: bool,
    /// Why nothing was done, when nothing was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    pub events_before: usize,
    pub events_after: usize,
    pub cards_archived: Vec<u64>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// Every event up to this number is in the archive.
    pub upto: u64,
}

pub enum Failed {
    /// Exit 4.
    Log(String),
    /// Exit 5: the kept events would read differently.
    Differs(String),
}

/// Compact the log in `dir`. `if_over`: do nothing unless the log in use
/// is at least this many bytes and at least twice what the last
/// compaction left.
pub fn compact(
    dir: &Path,
    keep_days: i64,
    keep_recent: usize,
    if_over: Option<u64>,
    dry_run: bool,
) -> Result<Outcome, Failed> {
    let log = Log::at(dir);
    let io = |e: super::log::LogError| Failed::Log(e.to_string());
    let (base, segments) = log.bytes().map_err(io)?;
    let mut outcome = Outcome {
        compacted: false,
        dry_run,
        skipped: None,
        events_before: 0,
        events_after: 0,
        cards_archived: Vec::new(),
        bytes_before: base + segments,
        bytes_after: base + segments,
        upto: 0,
    };
    if let Some(least) = if_over {
        if base + segments < least || base + segments < base.saturating_mul(2) {
            outcome.skipped = Some("在用的日志还不够大".to_string());
            return Ok(outcome);
        }
    }
    let now = Utc::now();
    let mut seen = (0usize, 0usize, Vec::new());
    let mut decide = |events: &[Event]| -> Result<Rebase, String> {
        let plan = plan(events, now, keep_days, keep_recent);
        same_reading(events, &plan, now)?;
        seen = (
            events.len(),
            plan.keep.len(),
            plan.gone.iter().map(|g| g.card).collect(),
        );
        Ok(Rebase {
            keep: plan.keep,
            gone: plan.gone,
        })
    };
    if dry_run {
        let events = log.read().map_err(io)?;
        decide(&events).map_err(Failed::Differs)?;
    } else {
        match log.rebase(|events, _| decide(events)) {
            Ok(Some(floor)) => {
                outcome.compacted = true;
                outcome.upto = floor.upto;
            }
            Ok(None) => {}
            Err(AppendError::Refused(why)) => return Err(Failed::Differs(why)),
            Err(AppendError::Log(e)) => return Err(io(e)),
        }
    }
    (
        outcome.events_before,
        outcome.events_after,
        outcome.cards_archived,
    ) = seen;
    if outcome.events_before == outcome.events_after {
        outcome.skipped = Some("没有可以归档的".to_string());
    }
    let (base, segments) = log.bytes().map_err(io)?;
    outcome.bytes_after = base + segments;
    Ok(outcome)
}

/// How large the log in use may grow before the outer loop's watcher
/// compacts it: `OLP_LOG_COMPACT_MIB`, 32 by default, 0 for never.
pub fn auto_threshold() -> Option<u64> {
    let mib = std::env::var("OLP_LOG_COMPACT_MIB")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(32);
    (mib > 0).then_some(mib * 1024 * 1024)
}

/// `argv` is what follows `olp log compact`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp log compact";
    let (mut keep_days, mut keep_recent, mut if_over) = (14i64, 10usize, None);
    let (mut dry_run, mut as_json, mut rest) = (false, false, Vec::new());
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let mut number = || words.next().and_then(|v| v.parse::<u64>().ok());
        let ok = match word.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--dry-run" => {
                dry_run = true;
                true
            }
            "--json" => {
                as_json = true;
                true
            }
            "--keep-days" => number().map(|n| keep_days = n.min(36_500) as i64).is_some(),
            "--keep" => number().map(|n| keep_recent = n as usize).is_some(),
            "--if-over" => number()
                .map(|n| if_over = Some(n.saturating_mul(1024 * 1024)))
                .is_some(),
            other if other.starts_with("--") => false,
            other => {
                rest.push(other.to_string());
                true
            }
        };
        if !ok {
            eprintln!("{prog}: 参数不对: {word}\n{USAGE}");
            return 2;
        }
    }
    let [dir] = rest.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };
    if !Path::new(dir).is_dir() {
        eprintln!("{prog}: 日志目录不存在: {dir}");
        return 2;
    }
    let outcome = match compact(Path::new(dir), keep_days, keep_recent, if_over, dry_run) {
        Ok(outcome) => outcome,
        Err(Failed::Log(why)) => {
            eprintln!("{prog}: {why}");
            return 4;
        }
        Err(Failed::Differs(why)) => {
            eprintln!("{prog}: 留下的事件与完整日志读起来不一致({why}),没有写");
            return 5;
        }
    };
    if as_json {
        println!("{}", json!(outcome));
        return 0;
    }
    let moved = outcome.events_before - outcome.events_after;
    match &outcome.skipped {
        Some(why) => println!("没有压缩:{why}"),
        None => println!(
            "{}:{} 条事件移入归档,留下 {} 条;归档了 {} 张卡;在用的日志 {} → {} 字节",
            if dry_run { "会压缩" } else { "已压缩" },
            moved,
            outcome.events_after,
            outcome.cards_archived.len(),
            outcome.bytes_before,
            outcome.bytes_after,
        ),
    }
    0
}

/// `argv` is what follows `olp log history`.
pub fn history(argv: &[String]) -> i32 {
    let prog = "olp log history";
    let (mut card, mut as_json, mut rest) = (None, false, Vec::new());
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let ok = match word.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--json" => {
                as_json = true;
                true
            }
            "--card" => {
                card = words
                    .next()
                    .and_then(|v| v.trim_start_matches('#').parse::<u64>().ok());
                card.is_some()
            }
            other if other.starts_with("--") => false,
            other => {
                rest.push(other.to_string());
                true
            }
        };
        if !ok {
            eprintln!("{prog}: 参数不对: {word}\n{USAGE}");
            return 2;
        }
    }
    let [dir] = rest.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };
    if !Path::new(dir).is_dir() {
        eprintln!("{prog}: 日志目录不存在: {dir}");
        return 2;
    }
    let events = match Log::at(dir).history() {
        Ok(events) => events,
        Err(e) => {
            eprintln!("{prog}: {e}");
            return 4;
        }
    };
    for event in events.iter().filter(|e| card.is_none() || e.card == card) {
        if as_json {
            println!("{}", json!(event));
            continue;
        }
        let said = ["title", "text", "node", "result", "question"]
            .iter()
            .filter_map(|field| event.data.get(field).and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:>7}  {}  {}{}  {}  {}",
            event.seq,
            event.ts,
            event.kind,
            event.card.map(|c| format!(" #{c}")).unwrap_or_default(),
            event.actor,
            said.chars().take(120).collect::<String>(),
        );
    }
    0
}
