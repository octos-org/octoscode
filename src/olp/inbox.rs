//! `octoscode olp board inbox` — the board's entries sorted by who has to
//! act: inbox (the outer loop decides), working, snoozed, settled
//! (specs/task-olp-board-inbox.spec.md). Same arguments, output and exit
//! codes as `scripts/olp-inbox.py`, which the other scripts still load.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;

use super::board::{self, Ack, Entry, Event, EventKind, Scan};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Partition {
    Inbox,
    Working,
    Snoozed,
    Settled,
}

impl Partition {
    const ALL: [Partition; 4] = [
        Partition::Inbox,
        Partition::Working,
        Partition::Snoozed,
        Partition::Settled,
    ];

    fn zh(self) -> &'static str {
        match self {
            Partition::Inbox => "收件箱",
            Partition::Working => "进行中",
            Partition::Snoozed => "搁置",
            Partition::Settled => "已了结",
        }
    }
}

/// The last thing that happened to an entry.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Last {
    Accepted,
    AckBlocked,
    AckDone,
    AckWontdo,
    Pushed,
    Returned,
}

pub struct State {
    pub partition: Partition,
    pub reason: String,
    pub until: Option<String>,
}

fn state(partition: Partition, reason: impl Into<String>) -> State {
    State {
        partition,
        reason: reason.into(),
        until: None,
    }
}

/// `至 YYYY-MM-DD` at the start of a snooze line's text.
fn snooze_until(detail: &str) -> Option<&str> {
    let rest = detail.trim_start().strip_prefix('至')?.trim_start();
    let date = rest.get(..10)?;
    let shape = date.bytes().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    });
    shape.then_some(date)
}

/// Where one entry stands, from the last thing said about it.
pub fn classify(
    number: u64,
    entry: &Entry,
    events: &[Event],
    today: NaiveDate,
    warnings: &mut Vec<String>,
) -> State {
    if events.iter().any(|e| e.kind == EventKind::Voided) {
        return state(Partition::Settled, "已被改判作废");
    }
    let mut timeline: Vec<(usize, Last)> = entry
        .acks
        .iter()
        .map(|(line, kind)| {
            let last = match kind {
                Ack::Done => Last::AckDone,
                Ack::Wontdo => Last::AckWontdo,
                Ack::Blocked => Last::AckBlocked,
            };
            (*line, last)
        })
        .collect();
    timeline.extend(events.iter().filter_map(|e| {
        let last = match e.kind {
            EventKind::Accepted => Last::Accepted,
            EventKind::Returned => Last::Returned,
            EventKind::Pushed(_) => Last::Pushed,
            _ => return None,
        };
        Some((e.line, last))
    }));
    timeline.sort();
    let (last_line, last) = match timeline.last() {
        Some((line, last)) => (*line, Some(*last)),
        None => (0, None),
    };

    // Old archivers also stubbed returned unit cards and outstanding ACKs.
    // Their machine-readable lines survive; they recover the open work. A
    // stub without a unit keeps its historical disposition.
    if let Some(archive_line) = entry.archive_line {
        if last.is_none()
            || last == Some(Last::Accepted)
            || (entry.unit.is_none() && last_line <= archive_line)
        {
            return state(Partition::Settled, "已归档");
        }
    }
    match last {
        Some(Last::Accepted) => return state(Partition::Settled, "已采认"),
        // A returned card goes back to its unit along the dispatch path and
        // is pushed again on the same branch: it still occupies the unit.
        Some(Last::Returned) if entry.unit.is_some() => {
            return state(Partition::Working, "已打回,等单元返工后重新推送");
        }
        Some(Last::Returned) => return state(Partition::Settled, "已打回(整改见后续条目)"),
        _ => {}
    }

    // A snooze counts only when it is the latest thing said about the entry.
    let mut until: Option<NaiveDate> = None;
    for e in events {
        let EventKind::Snooze(detail) = &e.kind else {
            continue;
        };
        if e.line < last_line {
            continue;
        }
        let Some(text) = snooze_until(detail) else {
            warnings.push(format!(
                "第 {} 行:#{number} 的搁置行缺少可解析的日期(应为 至 YYYY-MM-DD),已忽略",
                e.line
            ));
            continue;
        };
        match NaiveDate::parse_from_str(text, "%Y-%m-%d") {
            Ok(date) => until = Some(date),
            Err(_) => warnings.push(format!(
                "第 {} 行:#{number} 的搁置日期 {text} 不是合法日期,已忽略",
                e.line
            )),
        }
    }
    let expired = match until {
        Some(date) if today < date => {
            return State {
                partition: Partition::Snoozed,
                reason: format!("搁置至 {date}"),
                until: Some(date.to_string()),
            };
        }
        Some(date) => format!("(搁置已于 {date} 到期)"),
        None => String::new(),
    };
    let reason = match last {
        None if until.is_some() => "搁置到期,尚未 ACK",
        None => {
            // The unit's watcher said it could not hand the card to its
            // pane, and the card was not claimed since: someone has to look.
            let at = |kind: EventKind| events.iter().rev().find(|e| e.kind == kind).map(|e| e.line);
            return match (at(EventKind::Undelivered), at(EventKind::Claimed)) {
                (Some(missed), claimed) if claimed.is_none_or(|c| c < missed) => state(
                    Partition::Inbox,
                    "送达失败:单元报告窗格没有接住提醒,请看一下该窗格",
                ),
                _ => state(Partition::Working, "已派出,等内环 ACK"),
            };
        }
        Some(Last::AckDone) => "待复验:内环已 ACK(done)",
        Some(Last::AckWontdo) => "待回应:内环 ACK(wontdo),接受或升级",
        Some(Last::AckBlocked) => "待解阻塞:内环 ACK(blocked)",
        Some(Last::Pushed) | Some(Last::Accepted) | Some(Last::Returned) => "待审核:单元已推送",
    };
    state(Partition::Inbox, format!("{reason}{expired}"))
}

#[derive(Serialize, Clone)]
pub struct Item {
    pub number: u64,
    pub title: String,
    pub unit: Option<String>,
    #[serde(rename = "impl")]
    pub implementer: Option<String>,
    pub review: Vec<String>,
    pub partition: Partition,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

#[derive(Serialize)]
struct Counts {
    inbox: usize,
    working: usize,
    snoozed: usize,
    settled: usize,
}

#[derive(Serialize)]
struct QuestionOut {
    id: String,
    unit: Option<String>,
    question: String,
    line: usize,
}

#[derive(Serialize)]
struct Report {
    today: String,
    counts: Counts,
    items: Vec<Item>,
    questions: Vec<QuestionOut>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct OpenCard {
    number: u64,
    title: String,
    partition: Partition,
    reason: String,
}

#[derive(Serialize)]
struct UnitLoad {
    unit: String,
    busy: bool,
    open: Vec<OpenCard>,
}

#[derive(Serialize)]
struct UnitsReport {
    today: String,
    units: Vec<UnitLoad>,
    warnings: Vec<String>,
}

/// The entries `wanted` picks, each with its state, in number order. Only
/// those are classified, so only they can add warnings.
pub fn items(scan: &mut Scan, today: NaiveDate, wanted: impl Fn(&Entry) -> bool) -> Vec<Item> {
    let mut out = Vec::new();
    for (number, entry) in scan.entries.iter().filter(|(_, e)| wanted(e)) {
        let events = scan.events.get(number).map(Vec::as_slice).unwrap_or(&[]);
        let state = classify(*number, entry, events, today, &mut scan.warnings);
        out.push(Item {
            number: *number,
            title: entry.title.clone(),
            unit: entry.unit.clone(),
            implementer: entry.implementer.clone(),
            review: entry.review.clone(),
            partition: state.partition,
            reason: state.reason,
            until: state.until,
        });
    }
    out
}

struct Args {
    board: String,
    units: bool,
    json: bool,
    all: bool,
    unit: Option<String>,
    today: Option<String>,
}

const USAGE: &str =
    "usage: olp-inbox.py [-h] [--units] [--json] [--unit UNIT] [--all] [--today TODAY] board";

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut args = Args {
        board: String::new(),
        units: false,
        json: false,
        all: false,
        unit: None,
        today: None,
    };
    let mut boards = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let mut value = |name: &str| {
            words
                .next()
                .cloned()
                .ok_or_else(|| format!("argument {name}: expected one argument"))
        };
        match word.as_str() {
            "-h" | "--help" => return Ok(None),
            "--units" => args.units = true,
            "--json" => args.json = true,
            "--all" => args.all = true,
            "--unit" => args.unit = Some(value("--unit")?),
            "--today" => args.today = Some(value("--today")?),
            other => {
                if let Some(v) = other.strip_prefix("--unit=") {
                    args.unit = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--today=") {
                    args.today = Some(v.to_string());
                } else if other.starts_with('-') && other.len() > 1 {
                    return Err(format!("unrecognized arguments: {other}"));
                } else {
                    boards.push(other.to_string());
                }
            }
        }
    }
    match boards.len() {
        0 => Err("the following arguments are required: board".to_string()),
        1 => {
            args.board = boards.remove(0);
            Ok(Some(args))
        }
        _ => Err(format!("unrecognized arguments: {}", boards[1..].join(" "))),
    }
}

fn print_json<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
    );
}

/// `argv` is everything after `olp board inbox`. Returns the exit code.
pub fn run(argv: &[String]) -> i32 {
    let args = match parse(argv) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!(
                "{USAGE}\n\nOLP 黑板收件箱\n\n  board            黑板文件\n  --units          按单元列出还占着它的卡(占用视图)\n  --json           输出 JSON\n  --unit UNIT      只看派给这个单元的条目\n  --all            连已了结一起输出\n  --today TODAY    把今天当作这一天(YYYY-MM-DD),用于判断搁置是否到期"
            );
            return 0;
        }
        Err(message) => {
            eprintln!("{USAGE}\nolp-inbox.py: error: {message}");
            return 2;
        }
    };
    let source = std::path::Path::new(&args.board);
    // A directory is an event log (JSON Lines); a file is a Markdown board.
    let from_log = source.is_dir();
    if !from_log && !source.is_file() {
        eprintln!("olp-inbox: 板不存在: {}", args.board);
        return 2;
    }
    let today = match &args.today {
        Some(text) => match NaiveDate::parse_from_str(text, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => {
                eprintln!("olp-inbox: --today 不是合法日期: {text}");
                return 2;
            }
        },
        None => chrono::Local::now().date_naive(),
    };
    let (mut scan, pending) = if from_log {
        match super::log::Log::at(source).read() {
            Ok(events) => (
                super::cards::scan(&events),
                super::cards::pending_questions(&events),
            ),
            Err(e) => {
                eprintln!("olp-inbox: {e}");
                return 1;
            }
        }
    } else {
        match std::fs::read(source) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                (board::scan(&text), board::pending_questions(&text))
            }
            Err(e) => {
                eprintln!("olp-inbox: 读不了板 {}: {e}", args.board);
                return 2;
            }
        }
    };

    if args.units {
        let all = items(&mut scan, today, |e| e.unit.is_some());
        // Every unit a card was dispatched to, with the cards it still holds.
        let mut load: BTreeMap<String, Vec<OpenCard>> = BTreeMap::new();
        for item in &all {
            let Some(unit) = &item.unit else { continue };
            let cards = load.entry(unit.clone()).or_default();
            if item.partition != Partition::Settled {
                cards.push(OpenCard {
                    number: item.number,
                    title: item.title.clone(),
                    partition: item.partition,
                    reason: item.reason.clone(),
                });
            }
        }
        let units: Vec<UnitLoad> = load
            .into_iter()
            .filter(|(unit, _)| args.unit.as_ref().is_none_or(|only| only == unit))
            .map(|(unit, open)| UnitLoad {
                unit,
                busy: !open.is_empty(),
                open,
            })
            .collect();
        if args.json {
            print_json(&UnitsReport {
                today: today.to_string(),
                units,
                warnings: scan.warnings,
            });
            return 0;
        }
        if units.is_empty() {
            println!("板上没有派给单元的条目");
        }
        for u in &units {
            if u.open.is_empty() {
                println!("{}\t空闲", u.unit);
                continue;
            }
            let cards: Vec<String> = u
                .open
                .iter()
                .map(|c| format!("#{} {}", c.number, c.reason))
                .collect();
            println!("{}\t忙\t{}", u.unit, cards.join(";"));
        }
        for w in &scan.warnings {
            eprintln!("warning: {w}");
        }
        return 0;
    }

    let shown = items(&mut scan, today, |e| {
        args.unit.is_none() || e.unit == args.unit
    });
    let count = |p: Partition| shown.iter().filter(|i| i.partition == p).count();
    let counts = Counts {
        inbox: count(Partition::Inbox),
        working: count(Partition::Working),
        snoozed: count(Partition::Snoozed),
        settled: count(Partition::Settled),
    };
    let listed = |p: Partition| args.all || p != Partition::Settled;
    let questions: Vec<board::Question> = pending
        .into_iter()
        .filter(|q| args.unit.is_none() || q.unit == args.unit)
        .collect();
    if args.json {
        print_json(&Report {
            today: today.to_string(),
            counts,
            items: shown.into_iter().filter(|i| listed(i.partition)).collect(),
            questions: questions
                .into_iter()
                .map(|q| QuestionOut {
                    id: q.id,
                    unit: q.unit,
                    question: q.question,
                    line: q.line,
                })
                .collect(),
            warnings: scan.warnings,
        });
        return 0;
    }
    if !questions.is_empty() {
        // An inner loop waits only 50 seconds: questions come first.
        println!("== 待回答的提问({})", questions.len());
        for q in &questions {
            match &q.unit {
                Some(unit) => {
                    println!("id={} [{unit}] {}", q.id, q.question);
                    println!(
                        "    olp-answer.py answer {unit} {}  (回答从 stdin 喂)",
                        q.id
                    );
                }
                None => println!("id={} {}", q.id, q.question),
            }
        }
    }
    for partition in Partition::ALL.into_iter().filter(|p| listed(*p)) {
        let rows: Vec<&Item> = shown.iter().filter(|i| i.partition == partition).collect();
        println!("== {}({})", partition.zh(), rows.len());
        for i in rows {
            let unit = i
                .unit
                .as_ref()
                .map(|u| format!(" [{u}]"))
                .unwrap_or_default();
            let roles = if i.implementer.is_some() || !i.review.is_empty() {
                let review = if i.review.is_empty() {
                    "-".to_string()
                } else {
                    i.review.join("、")
                };
                format!(
                    " (实现 {},复验 {review})",
                    i.implementer.as_deref().unwrap_or("-")
                )
            } else {
                String::new()
            };
            println!("#{}{unit}{roles} {}", i.number, i.title);
            println!("    {}", i.reason);
        }
    }
    if !args.all {
        println!("== 已了结({}),--all 查看", counts.settled);
    }
    for w in &scan.warnings {
        eprintln!("warning: {w}");
    }
    0
}
