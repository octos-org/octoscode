//! `octoscode olp log metrics <日志目录>` — what the event log says about
//! how the cluster delivers: how long a card takes and where the time
//! goes, how often work comes back, and what interrupted it.
//!
//! Everything is computed from the whole history (archive and the log in
//! use); nothing is collected elsewhere and nothing is written. Each card
//! becomes one row, and every number in the summary is taken from those
//! rows, so an odd number can be traced to the cards behind it (`--cards`).
//!
//! A card's time is split at its events:
//!
//! | span | from | to |
//! |---|---|---|
//! | `to_claim` | dispatched | the unit's `claimed` |
//! | `work` | claimed (or dispatched, when never claimed) and every `returned` | the next receipt |
//! | `outer_wait` | a receipt | the outer loop's `accepted` / `returned` / `voided` |
//! | `total` | dispatched | `accepted` |
//!
//! `first_pass` is a card accepted without ever being returned;
//! `reversed` is one returned or voided after it had been accepted.
//! Durations are reported as the median and the 90th percentile, never a
//! mean: a few slow cards would hide the rest.
//!
//! What the log cannot tell yet is who did a step — an agent or a person
//! by hand — so nothing here measures human effort. This is a diagnosis,
//! not a target: it judges nothing and its exit code says nothing about
//! the numbers.
//!
//! Exit codes: 0; 2 usage; 4 the log could not be read.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use serde_json::json;

use super::log::{Event, Log};

const USAGE: &str = "用法: octoscode olp log metrics <日志目录> [--since 日期] [--until 日期] [--by unit|impl|week] [--cards] [--json]\n\n\
事件日志的交付指标:每张卡多久、时间花在哪一段、打回多少、被什么打断。只读整个历史(归档 + 在用),不写任何东西。\n\
--since / --until:只算这段时间内派出的卡(YYYY-MM-DD 或 RFC 3339,UTC;--until 不含当刻)。\n\
--by:按单元、实现方或派出的周分组。--cards:列出每张卡的明细。\n\
时长给中位数和 90 分位。日志里没有记是 agent 还是人做的,所以这里没有人工介入量。这是诊断,不是指标考核。\n\
退出码:0;2 用法;4 日志读不了";

/// How a card stands at the end of the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Accepted,
    Voided,
    Open,
}

/// One card, as the log tells it. Times are seconds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub number: u64,
    pub title: String,
    pub unit: Option<String>,
    #[serde(rename = "impl")]
    pub implementer: Option<String>,
    pub dispatched: String,
    pub outcome: Outcome,
    pub to_claim: Option<i64>,
    pub work: Option<i64>,
    pub outer_wait: Option<i64>,
    pub total: Option<i64>,
    pub returns: u64,
    pub review_rounds: u64,
    pub blocked: u64,
    pub alerts: u64,
    pub snoozes: u64,
    pub undelivered: u64,
    pub first_pass: bool,
    pub reversed: bool,
}

fn at(event: &Event) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&event.ts)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn word<'a>(event: &'a Event, field: &str) -> &'a str {
    event.data.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

/// Where a card is while its events are walked.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    /// Dispatched, not claimed.
    Waiting,
    /// With the unit.
    Working,
    /// A receipt is in; the outer loop has not ruled.
    AtOuter,
    /// Accepted or voided.
    Over,
}

/// One row per dispatched card, in number order. Events whose time cannot
/// be read still count (returns, alerts) but move no clock.
pub fn rows(events: &[Event]) -> Vec<Row> {
    let mut by_card: BTreeMap<u64, Vec<&Event>> = BTreeMap::new();
    for event in events {
        if let Some(card) = event.card {
            by_card.entry(card).or_default().push(event);
        }
    }
    let mut out = Vec::new();
    for (number, about) in by_card {
        let Some(first) = about.iter().find(|e| e.kind == "dispatched") else {
            continue;
        };
        let mut row = Row {
            number,
            title: word(first, "title").to_string(),
            unit: first.unit.clone(),
            implementer: Some(word(first, "impl").to_string()).filter(|s| !s.is_empty()),
            dispatched: first.ts.clone(),
            outcome: Outcome::Open,
            to_claim: None,
            work: None,
            outer_wait: None,
            total: None,
            returns: 0,
            review_rounds: 0,
            blocked: 0,
            alerts: 0,
            snoozes: 0,
            undelivered: 0,
            first_pass: false,
            reversed: false,
        };
        let start = at(first);
        let (mut stage, mut mark, mut accepted) = (Stage::Waiting, start, false);
        // Time from `mark` to `now`, added to one of the row's spans.
        let add =
            |span: &mut Option<i64>, mark: Option<DateTime<Utc>>, now: Option<DateTime<Utc>>| {
                if let (Some(from), Some(to)) = (mark, now) {
                    *span = Some(span.unwrap_or(0) + (to - from).num_seconds().max(0));
                }
            };
        for event in &about {
            let now = at(event);
            match event.kind.as_str() {
                "progress" => match word(event, "node") {
                    "claimed" if stage == Stage::Waiting => {
                        add(&mut row.to_claim, mark, now);
                        stage = Stage::Working;
                        mark = now.or(mark);
                    }
                    "review" => {
                        let round = event.data.get("round").and_then(|v| v.as_u64());
                        row.review_rounds = row.review_rounds.max(round.unwrap_or(1));
                    }
                    "undelivered" => row.undelivered += 1,
                    _ => {}
                },
                "ack" => {
                    if word(event, "result") == "blocked" {
                        row.blocked += 1;
                    }
                    if matches!(stage, Stage::Waiting | Stage::Working) {
                        add(&mut row.work, mark, now);
                        stage = Stage::AtOuter;
                        mark = now.or(mark);
                    }
                }
                "returned" => {
                    row.returns += 1;
                    if stage == Stage::AtOuter {
                        add(&mut row.outer_wait, mark, now);
                    }
                    if stage == Stage::Over && accepted {
                        row.reversed = true;
                    }
                    if stage != Stage::Working {
                        mark = now.or(mark);
                    }
                    stage = Stage::Working;
                    row.outcome = Outcome::Open;
                    row.total = None;
                }
                "accepted" => {
                    if stage == Stage::AtOuter {
                        add(&mut row.outer_wait, mark, now);
                    }
                    if stage != Stage::Over {
                        row.total = None;
                        add(&mut row.total, start, now);
                    }
                    stage = Stage::Over;
                    accepted = true;
                    row.outcome = Outcome::Accepted;
                }
                "voided" => {
                    if stage == Stage::AtOuter {
                        add(&mut row.outer_wait, mark, now);
                    }
                    if row.outcome == Outcome::Accepted {
                        row.reversed = true;
                    }
                    stage = Stage::Over;
                    row.outcome = Outcome::Voided;
                    row.total = None;
                }
                "snoozed" => row.snoozes += 1,
                "alert" => row.alerts += 1,
                _ => {}
            }
        }
        row.first_pass = row.outcome == Outcome::Accepted && row.returns == 0 && !row.reversed;
        out.push(row);
    }
    out
}

/// The median and the 90th percentile (nearest rank) of some seconds.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Spread {
    pub cards: usize,
    pub p50: Option<i64>,
    pub p90: Option<i64>,
}

pub fn spread(mut values: Vec<i64>) -> Spread {
    values.sort_unstable();
    let rank = |percent: usize| {
        let n = values.len();
        (n > 0).then(|| values[(n * percent).div_ceil(100).clamp(1, n) - 1])
    };
    Spread {
        cards: values.len(),
        p50: rank(50),
        p90: rank(90),
    }
}

/// The numbers of a set of cards.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Summary {
    pub cards: usize,
    pub accepted: usize,
    pub voided: usize,
    pub open: usize,
    /// Of the accepted cards: never returned.
    pub first_pass: usize,
    /// Returned or voided after having been accepted.
    pub reversed: usize,
    pub returns: u64,
    pub blocked: u64,
    pub alerts: u64,
    pub snoozes: u64,
    pub undelivered: u64,
    /// Accepted cards only.
    pub total: Spread,
    pub to_claim: Spread,
    pub work: Spread,
    pub outer_wait: Spread,
}

pub fn summarize(rows: &[&Row]) -> Summary {
    let count = |want: Outcome| rows.iter().filter(|r| r.outcome == want).count();
    let sum = |field: fn(&Row) -> u64| rows.iter().map(|r| field(r)).sum();
    // Voided cards are left out of the times: they were not delivered.
    let times = |field: fn(&Row) -> Option<i64>| {
        spread(
            rows.iter()
                .filter(|r| r.outcome != Outcome::Voided)
                .filter_map(|r| field(r))
                .collect(),
        )
    };
    Summary {
        cards: rows.len(),
        accepted: count(Outcome::Accepted),
        voided: count(Outcome::Voided),
        open: count(Outcome::Open),
        first_pass: rows.iter().filter(|r| r.first_pass).count(),
        reversed: rows.iter().filter(|r| r.reversed).count(),
        returns: sum(|r| r.returns),
        blocked: sum(|r| r.blocked),
        alerts: sum(|r| r.alerts),
        snoozes: sum(|r| r.snoozes),
        undelivered: sum(|r| r.undelivered),
        total: times(|r| r.total),
        to_claim: times(|r| r.to_claim),
        work: times(|r| r.work),
        outer_wait: times(|r| r.outer_wait),
    }
}

/// Questions from the units (`ask_outer`) in a span of time: asked, and of
/// those, how many were left to time out.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Questions {
    pub asked: usize,
    pub answered: usize,
    pub timed_out: usize,
}

fn questions(events: &[Event], inside: impl Fn(&str) -> bool) -> Questions {
    let mut ids: Vec<&str> = Vec::new();
    let mut found = Questions::default();
    for event in events.iter().filter(|e| e.kind == "asked") {
        let id = word(event, "id");
        if !id.is_empty() && inside(&event.ts) && !ids.contains(&id) {
            ids.push(id);
        }
    }
    found.asked = ids.len();
    for id in ids {
        let ended = |kind: &str| events.iter().any(|e| e.kind == kind && word(e, "id") == id);
        if ended("answered") {
            found.answered += 1;
        } else if ended("timed_out") {
            found.timed_out += 1;
        }
    }
    found
}

/// `1h05m`, `3d02h`: two units, enough to compare.
fn human(seconds: i64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s if s < 86_400 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        s => format!("{}d{:02}h", s / 86_400, s % 86_400 / 3600),
    }
}

fn shown(spread: &Spread) -> String {
    match (spread.p50, spread.p90) {
        (Some(p50), Some(p90)) => format!(
            "中位 {}  90 分位 {}  ({} 张)",
            human(p50),
            human(p90),
            spread.cards
        ),
        _ => "无数据".to_string(),
    }
}

fn print_summary(label: &str, s: &Summary) {
    println!(
        "{label}: {} 张卡 — 采认 {},作废 {},未了结 {}",
        s.cards, s.accepted, s.voided, s.open
    );
    if s.accepted > 0 {
        println!(
            "  一次通过 {}/{}({}%);采认后又改判 {}",
            s.first_pass,
            s.accepted,
            s.first_pass * 100 / s.accepted,
            s.reversed
        );
    }
    println!(
        "  打回 {} 次;阻塞回执 {};守护告警 {};搁置 {};送达失败 {}",
        s.returns, s.blocked, s.alerts, s.snoozes, s.undelivered
    );
    println!("  派出→采认   {}", shown(&s.total));
    println!("  等认领      {}", shown(&s.to_claim));
    println!("  单元在做    {}", shown(&s.work));
    println!("  等外环裁决  {}", shown(&s.outer_wait));
}

/// A bound given as a date or a full time, as a string that compares with
/// the log's UTC timestamps.
fn bound(text: &str) -> Option<String> {
    if chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok() {
        return Some(format!("{text}T00:00:00.000Z"));
    }
    DateTime::parse_from_rfc3339(text).ok().map(|t| {
        t.with_timezone(&Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    })
}

/// `argv` is what follows `olp log metrics`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp log metrics";
    let (mut since, mut until, mut by) = (None, None, None::<String>);
    let (mut list, mut as_json, mut rest) = (false, false, Vec::new());
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
            "--cards" => {
                list = true;
                true
            }
            "--since" => {
                since = words.next().and_then(|v| bound(v));
                since.is_some()
            }
            "--until" => {
                until = words.next().and_then(|v| bound(v));
                until.is_some()
            }
            "--by" => {
                by = words
                    .next()
                    .filter(|v| matches!(v.as_str(), "unit" | "impl" | "week"))
                    .cloned();
                by.is_some()
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
    let inside = |ts: &str| {
        since.as_deref().is_none_or(|from| ts >= from) && until.as_deref().is_none_or(|to| ts < to)
    };
    let all = rows(&events);
    let wanted: Vec<&Row> = all.iter().filter(|r| inside(&r.dispatched)).collect();
    let asked = questions(&events, inside);
    let key = |row: &Row| -> String {
        match by.as_deref() {
            Some("unit") => row.unit.clone().unwrap_or_else(|| "-".to_string()),
            Some("impl") => row.implementer.clone().unwrap_or_else(|| "-".to_string()),
            _ => match DateTime::parse_from_rfc3339(&row.dispatched) {
                Ok(t) => {
                    let week = t.with_timezone(&Utc).iso_week();
                    format!("{}-W{:02}", week.year(), week.week())
                }
                Err(_) => "-".to_string(),
            },
        }
    };
    let mut groups: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    if by.is_some() {
        for row in &wanted {
            groups.entry(key(row)).or_default().push(row);
        }
    }
    let whole = summarize(&wanted);
    if as_json {
        let groups: BTreeMap<&String, Summary> =
            groups.iter().map(|(k, v)| (k, summarize(v))).collect();
        let mut report = json!({
            "since": since,
            "until": until,
            "events": events.len(),
            "summary": whole,
            "questions": asked,
        });
        if let Some(by) = &by {
            report["by"] = json!(by);
            report["groups"] = json!(groups);
        }
        if list {
            report["cards"] = json!(wanted);
        }
        println!("{report}");
        return 0;
    }
    print_summary("全部", &whole);
    println!(
        "单元提问 {}(已答 {},超时 {})",
        asked.asked, asked.answered, asked.timed_out
    );
    for (name, members) in &groups {
        print_summary(name, &summarize(members));
    }
    if list {
        println!("卡号  结果      派出→采认  等认领  在做    等裁决  打回  单元  标题");
        let cell = |v: Option<i64>| v.map(human).unwrap_or_else(|| "-".to_string());
        for row in &wanted {
            println!(
                "#{:<4} {:<9} {:<10} {:<7} {:<7} {:<7} {:<5} {}  {}",
                row.number,
                match row.outcome {
                    Outcome::Accepted if row.reversed => "accepted*",
                    Outcome::Accepted => "accepted",
                    Outcome::Voided => "voided",
                    Outcome::Open => "open",
                },
                cell(row.total),
                cell(row.to_claim),
                cell(row.work),
                cell(row.outer_wait),
                row.returns,
                row.unit.as_deref().unwrap_or("-"),
                row.title.chars().take(40).collect::<String>(),
            );
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: u64, minute: i64, kind: &str, card: u64, data: serde_json::Value) -> Event {
        let base = DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap();
        Event {
            seq,
            ts: (base + chrono::Duration::minutes(minute))
                .with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            kind: kind.to_string(),
            card: Some(card),
            actor: "t".to_string(),
            unit: Some("m07".to_string()),
            key: None,
            data,
        }
    }

    #[test]
    fn a_cards_time_is_split_at_its_events() {
        let events = vec![
            event(1, 0, "dispatched", 1, json!({"title": "甲"})),
            event(2, 2, "progress", 1, json!({"node": "claimed"})),
            event(3, 12, "ack", 1, json!({"result": "done"})),
            event(4, 15, "returned", 1, json!({})),
            event(5, 20, "progress", 1, json!({"node": "review", "round": 2})),
            event(6, 25, "ack", 1, json!({"result": "done"})),
            event(7, 26, "accepted", 1, json!({})),
        ];
        let row = &rows(&events)[0];
        assert_eq!(row.outcome, Outcome::Accepted);
        assert_eq!(row.to_claim, Some(120));
        // 2→12 and 15→25.
        assert_eq!(row.work, Some(1200));
        // 12→15 and 25→26.
        assert_eq!(row.outer_wait, Some(240));
        assert_eq!(row.total, Some(26 * 60));
        assert_eq!((row.returns, row.review_rounds), (1, 2));
        assert!(!row.first_pass && !row.reversed);
    }

    #[test]
    fn a_ruling_changed_after_acceptance_is_counted_as_reversed() {
        let events = vec![
            event(1, 0, "dispatched", 1, json!({"title": "甲"})),
            event(2, 5, "ack", 1, json!({"result": "done"})),
            event(3, 6, "accepted", 1, json!({})),
            event(4, 60, "voided", 1, json!({})),
            event(5, 0, "dispatched", 2, json!({"title": "乙"})),
            event(6, 5, "ack", 2, json!({"result": "done"})),
            event(7, 6, "accepted", 2, json!({})),
        ];
        let all = rows(&events);
        assert!(all[0].reversed && all[0].outcome == Outcome::Voided);
        assert_eq!(all[0].total, None);
        assert!(all[1].first_pass);
        // Never claimed: the work runs from the dispatch.
        assert_eq!((all[1].to_claim, all[1].work), (None, Some(300)));
        let summary = summarize(&all.iter().collect::<Vec<_>>());
        assert_eq!(
            (summary.accepted, summary.voided, summary.reversed),
            (1, 1, 1)
        );
        // The voided card's times are not in the spread.
        assert_eq!(summary.work.cards, 1);
    }

    #[test]
    fn a_spread_is_the_median_and_the_ninetieth_percentile() {
        let got = spread((1..=10).collect());
        assert_eq!((got.cards, got.p50, got.p90), (10, Some(5), Some(9)));
        assert_eq!(spread(vec![7]).p90, Some(7));
        assert_eq!(spread(vec![]).p50, None);
        assert_eq!(human(59), "59s");
        assert_eq!(human(3900), "1h05m");
        assert_eq!(human(93_600), "1d02h");
    }
}
