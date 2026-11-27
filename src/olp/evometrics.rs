//! `octoscode olp evo metrics <repo> [--since EVO-NNNN] [--json]
//! [--baseline <json>] [--stall <review board> --stall-threshold <minutes>
//! --now <time>]` — windowed diagnostics of the evolution board.
//!
//! Diagnostic only, not a KPI: a rising count may mean better detection.
//! Read-only: it writes no file and reads no retro state. The exit code is
//! always 0.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use fancy_regex::Regex;

use super::evolib::{Json, group, parse_cards};

const NOTE: &str = "diagnostic only, not a KPI; rising counts may mean better detection";
/// A slice id: `44`, `44a`, `43c-2`.
const SLICE: &str = r"[0-9]+[a-z]?(?:-[0-9a-z]+)*";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a pattern written in this file")
}

// `派单` counts only at the start of the text or after whitespace:
// `不派单 44a` and `立案并派单 44b` are not dispatches.
static FORWARD: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"(?:^|\s)派单\s+(?P<a>{SLICE})(?![0-9a-z])")));
static REVERSE: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"(?:^|\s)(?P<b>{SLICE})\s+派单(?![0-9a-z])")));
static ACK: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^(?:> )?ACK\((?P<s>{SLICE})\s+(?:done|blocked|wontdo)\b"
    ))
});
static DATE_INLINE: LazyLock<Regex> = LazyLock::new(|| re(r"\((\d{4}-\d\d-\d\d) (\d\d:\d\d)"));
static DATE_BRACKET: LazyLock<Regex> = LazyLock::new(|| re(r"\[(\d{4}-\d\d-\d\d)T(\d\d:\d\d)"));
static HEADING_DATE: LazyLock<Regex> = LazyLock::new(|| re(r"^###\s+\S+.*?\((\d{4}-\d\d-\d\d)"));
static QUOTE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:>\s+)?"));
static OUTER: LazyLock<Regex> = LazyLock::new(|| re(r"^外环(?:\([^)]*\))?·?"));

fn evo_number(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("EVO-")?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())?
}

type Time = chrono::DateTime<chrono::Utc>;

/// A date and an `HH:MM` as a UTC time; `None` for a date that is not one.
fn at(date: &str, clock: &str) -> Option<Time> {
    chrono::NaiveDateTime::parse_from_str(&format!("{date} {clock}"), "%Y-%m-%d %H:%M")
        .ok()
        .map(|t| t.and_utc())
}

/// Slices dispatched on a review board and not ACKed: (slice, minutes
/// since, or `None` when the dispatch carries no usable time).
fn stalls(board: &str, threshold: i64, now: Time) -> Vec<(String, Option<i64>)> {
    let mut dispatched: Vec<(String, Option<Time>)> = Vec::new();
    let mut acked: Vec<String> = Vec::new();
    let mut heading_date: Option<String> = None;
    for line in board.lines() {
        // Every entry starts without a date: a dateless heading must not
        // inherit the one before it.
        if line.starts_with("### ") {
            heading_date = None;
        }
        if let Ok(Some(found)) = HEADING_DATE.captures(line) {
            heading_date = Some(found[1].to_string());
        }
        let text = QUOTE.replace(line, "").replace("**", "");
        let text = OUTER.replace(&text, "");
        let slice = match FORWARD.captures(&text) {
            Ok(Some(found)) => Some(found["a"].to_string()),
            _ => match REVERSE.captures(&text) {
                Ok(Some(found)) => Some(found["b"].to_string()),
                _ => None,
            },
        };
        if let Some(slice) = slice.filter(|s| !dispatched.iter().any(|(seen, _)| seen == s)) {
            let stamped = DATE_INLINE
                .captures(line)
                .ok()
                .flatten()
                .or_else(|| DATE_BRACKET.captures(line).ok().flatten());
            let time = match stamped {
                Some(found) => at(&found[1], &found[2]),
                None => heading_date.as_deref().and_then(|d| at(d, "00:00")),
            };
            dispatched.push((slice, time));
        }
        if let Ok(Some(found)) = ACK.captures(line) {
            acked.push(found["s"].to_string());
        }
    }
    dispatched
        .into_iter()
        .filter(|(slice, _)| !acked.contains(slice))
        .filter_map(|(slice, time)| match time {
            None => Some((slice, None)),
            Some(time) => {
                let minutes = (now - time).num_seconds().div_euclid(60);
                (minutes >= threshold).then_some((slice, Some(minutes)))
            }
        })
        .collect()
}

/// `argv` is what follows `olp evo metrics`.
pub fn run(argv: &[String]) -> i32 {
    let Some((repo, rest)) = argv.split_first() else {
        eprintln!(
            "usage: octoscode olp evo metrics <repo-root> [--since EVO-NNNN] [--json] [--baseline <json>]"
        );
        return 1;
    };
    if repo == "-h" || repo == "--help" {
        println!(
            "用法: octoscode olp evo metrics <repo-root> [--since EVO-NNNN] [--json] [--baseline <json>]\n         [--stall <审查黑板> --stall-threshold <分钟> --now <时间>]\n\n进化板的窗口化诊断:只读,不是 KPI。退出码恒为 0。"
        );
        return 0;
    }
    let (mut since, mut as_json, mut baseline) = (String::new(), false, String::new());
    let (mut stall_board, mut threshold, mut now_text) = (String::new(), 0i64, String::new());
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let mut value = |what: &str| match words.next().filter(|v| !v.is_empty()) {
            Some(v) => Some(v.clone()),
            None => {
                eprintln!("{word} needs {what}");
                None
            }
        };
        let ok = match word.as_str() {
            "--json" => {
                as_json = true;
                true
            }
            "--since" => value("EVO-NNNN").map(|v| since = v).is_some(),
            "--baseline" => value("a json file").map(|v| baseline = v).is_some(),
            "--stall" => value("a review-board path")
                .map(|v| stall_board = v)
                .is_some(),
            "--stall-threshold" => value("minutes")
                .map(|v| threshold = v.parse().unwrap_or(0))
                .is_some(),
            "--now" => value("ISO8601").map(|v| now_text = v).is_some(),
            other => {
                eprintln!("error: unknown flag {other}");
                return 0;
            }
        };
        if !ok {
            return 1;
        }
    }
    if !std::path::Path::new(repo).is_dir() {
        eprintln!("error: repo-root not found: {repo}");
        return 0;
    }
    let board = std::env::var_os("OLP_EVO_BOARD")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(repo).join(".octos/EVOLUTION.md"));

    // The window starts after --since, or after the baseline's last card.
    let mut window_base = evo_number(&since).unwrap_or(0);
    let mut base_counts: BTreeMap<String, i64> = BTreeMap::new();
    let read = std::fs::read_to_string(&baseline)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    if let Some(serde_json::Value::Object(doc)) = read {
        if since.is_empty() {
            let through = doc
                .get("through_evo")
                .and_then(|v| v.as_str())
                .unwrap_or("EVO-0000");
            if let Some(n) = evo_number(through) {
                window_base = n;
            }
        }
        if let Some(counts) = doc.get("by_trigger").and_then(|v| v.as_object()) {
            base_counts = counts
                .iter()
                .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0)))
                .collect();
        }
    }
    let cards = parse_cards(&std::fs::read_to_string(&board).unwrap_or_default());
    let window: Vec<_> = cards.into_iter().filter(|c| c.num > window_base).collect();
    let through = window.iter().map(|c| c.num).max().unwrap_or(0);
    let mut by_trigger: BTreeMap<String, i64> = BTreeMap::new();
    let mut by_source: Vec<(String, i64)> = ["review", "events", "mcp"]
        .map(|s| (s.to_string(), 0))
        .to_vec();
    for card in &window {
        let trigger = card
            .trigger
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        *by_trigger.entry(trigger).or_default() += 1;
        let source = if card.source.is_empty() {
            "?"
        } else {
            card.source.split(' ').next().unwrap_or("?")
        };
        match by_source.iter_mut().find(|(name, _)| name == source) {
            Some(entry) => entry.1 += 1,
            None => by_source.push((source.to_string(), 1)),
        }
    }
    let candidates = group(&window);
    let recurring: Vec<_> = candidates
        .iter()
        .filter(|c| c.recurrence_hint() >= 2)
        .collect();
    let through_evo = format!("EVO-{through:04}");
    // One line per trigger whose count moved against the baseline.
    let mut names: Vec<&String> = by_trigger.keys().chain(base_counts.keys()).collect();
    names.sort();
    names.dedup();
    let deltas: Vec<(&str, &String, i64, i64)> = names
        .into_iter()
        .filter_map(|name| {
            let now = by_trigger.get(name).copied().unwrap_or(0);
            let base = base_counts.get(name).copied().unwrap_or(0);
            match now.cmp(&base) {
                std::cmp::Ordering::Greater => Some(("increase", name, base, now)),
                std::cmp::Ordering::Less => Some(("decrease", name, base, now)),
                std::cmp::Ordering::Equal => None,
            }
        })
        .collect();
    let stalled = match std::fs::read_to_string(&stall_board) {
        Ok(text) if !stall_board.is_empty() => {
            let now = if now_text.is_empty() {
                chrono::Utc::now()
            } else {
                chrono::DateTime::parse_from_rfc3339(&now_text.replace('Z', "+00:00"))
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&now_text, "%Y-%m-%dT%H:%M:%S")
                            .map(|t| t.and_utc())
                    })
                    .unwrap_or_else(|_| chrono::Utc::now())
            };
            stalls(&text, threshold, now)
        }
        _ => Vec::new(),
    };
    let fake_verified = by_trigger.get("r2_record").copied().unwrap_or(0);

    if as_json {
        let text = |s: &str| Json::Str(s.to_string());
        let pairs = |list: Vec<(&str, Json)>| {
            Json::Obj(list.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
        };
        let counts = |list: Vec<(String, i64)>| {
            Json::Obj(list.into_iter().map(|(k, v)| (k, Json::Int(v))).collect())
        };
        let out = pairs(vec![
            ("note", text(NOTE)),
            ("through_evo", text(&through_evo)),
            ("cards", Json::Int(window.len() as i64)),
            (
                "by_trigger",
                counts(by_trigger.clone().into_iter().collect()),
            ),
            ("by_source", counts(by_source.clone())),
            ("recurring_candidates", Json::Int(recurring.len() as i64)),
            (
                "recurring",
                Json::Arr(
                    recurring
                        .iter()
                        .map(|c| {
                            pairs(vec![
                                ("trigger", text(&c.trigger)),
                                ("hint", Json::Int(c.recurrence_hint() as i64)),
                                (
                                    "anchors",
                                    Json::Arr(c.anchors.iter().map(|a| text(a)).collect()),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "deltas",
                Json::Arr(
                    deltas
                        .iter()
                        .map(|(kind, name, base, now)| {
                            pairs(vec![
                                ("kind", text(kind)),
                                ("trigger", text(name)),
                                ("base", Json::Int(*base)),
                                ("now", Json::Int(*now)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "stalls",
                Json::Arr(
                    stalled
                        .iter()
                        .map(|(slice, minutes)| {
                            pairs(vec![
                                ("slice", text(slice)),
                                ("minutes", minutes.map(Json::Int).unwrap_or(Json::Null)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("fake_verified", Json::Int(fake_verified)),
        ]);
        println!("{}", out.pretty(1));
        return 0;
    }
    println!("note: {NOTE}");
    println!("through_evo: {through_evo}");
    println!("cards: {}", window.len());
    let listed: Vec<String> = by_trigger.iter().map(|(k, v)| format!("{k}={v}")).collect();
    println!("by_trigger: {}", listed.join(" "));
    let sources: Vec<String> = by_source
        .iter()
        .take(3)
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    println!("by_source: {}", sources.join(" "));
    println!("recurring_candidates: {}", recurring.len());
    for candidate in &recurring {
        println!(
            "- {} hint={} anchors={}",
            candidate.trigger,
            candidate.recurrence_hint(),
            candidate.anchors.join(",")
        );
    }
    for (kind, name, base, now) in &deltas {
        println!("{kind}: {name} {base}->{now}");
    }
    if !stall_board.is_empty() {
        for (slice, minutes) in &stalled {
            match minutes {
                Some(minutes) => println!("stall: {slice} {minutes}"),
                None => println!("stall: {slice} open"),
            }
        }
        println!("stalls: {}", stalled.len());
    }
    println!("fake_verified: {fake_verified}");
    0
}
