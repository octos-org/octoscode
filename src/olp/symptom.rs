//! `octoscode olp log symptoms <日志目录>` — say where delivery got worse,
//! with the cards that show it.
//!
//! The cards of the log are put into three spans by the time they were
//! dispatched: the latest window, the window before it, and everything
//! earlier (the baseline). An indicator is a symptom when both windows are
//! worse than the baseline and the two windows together hold enough cards
//! to say so (`--min-cards`): one bad window is noise, and a number with
//! no cards behind it cannot be looked into. Each symptom names the cards
//! that weigh most on it.
//!
//! Indicators (`metrics.rs` gives the rows):
//!
//! | name | value | worse when |
//! |---|---|---|
//! | `total`, `to_claim`, `work`, `outer_wait` | median seconds | half as long again, and a minute more |
//! | `returns_per_card`, `alerts_per_card`, `blocked_per_card` | count per card | 0.2 more |
//! | `reversed_rate` | share of cards | 0.1 more |
//!
//! `--record` writes each symptom to the log once (an event of kind
//! `symptom`, keyed by indicator and window), where the outer loop's
//! watcher counts it as something waiting for the outer loop. A symptom is
//! a finding for someone to read. It changes nothing by itself.
//!
//! Exit codes: 0; 2 usage; 4 the log could not be read or written.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::log::{Draft, Log};
use super::metrics::{Outcome, Row, rows};

const USAGE: &str = "用法: octoscode olp log symptoms <日志目录> [--window 7] [--min-cards 5] [--now 时刻] [--record] [--json]\n\n\
交付在哪里变差了:最近一个窗口和它之前的那个窗口都比更早的基线差,而且两个窗口合起来至少有 --min-cards 张卡,\n\
才算一条症状;每条症状列出最能说明它的卡。只偏离一个窗口不算。\n\
--window:窗口天数。--now:把这一刻当作现在(RFC 3339),用来回看。--record:把症状各记进日志一次(同一窗口不重复)。\n\
症状只是给人看的发现,本身不改变任何东西。\n\
退出码:0;2 用法;4 日志读写失败";

pub const INDICATORS: [&str; 8] = [
    "total",
    "to_claim",
    "work",
    "outer_wait",
    "returns_per_card",
    "alerts_per_card",
    "blocked_per_card",
    "reversed_rate",
];

/// What one card adds to an indicator: seconds, or a count.
fn weight(name: &str, row: &Row) -> Option<f64> {
    if row.outcome == Outcome::Voided {
        return None;
    }
    let seconds = |v: Option<i64>| v.map(|s| s as f64);
    match name {
        "total" => seconds(row.total),
        "to_claim" => seconds(row.to_claim),
        "work" => seconds(row.work),
        "outer_wait" => seconds(row.outer_wait),
        "returns_per_card" => Some(row.returns as f64),
        "alerts_per_card" => Some(row.alerts as f64),
        "blocked_per_card" => Some(row.blocked as f64),
        "reversed_rate" => Some(if row.reversed { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn is_duration(name: &str) -> bool {
    matches!(name, "total" | "to_claim" | "work" | "outer_wait")
}

/// An indicator over some cards, and how many cards it rests on. Durations
/// are medians; the rest are means.
pub fn indicator(name: &str, rows: &[&Row]) -> Option<(f64, usize)> {
    let mut values: Vec<f64> = rows.iter().filter_map(|r| weight(name, r)).collect();
    if values.is_empty() {
        return None;
    }
    let n = values.len();
    let value = if is_duration(name) {
        values.sort_by(|a, b| a.total_cmp(b));
        values[n.div_ceil(2) - 1]
    } else {
        values.iter().sum::<f64>() / n as f64
    };
    Some((value, n))
}

/// Whether `value` is worse than `baseline` by more than chance moves it.
pub fn worse(name: &str, baseline: f64, value: f64) -> bool {
    if is_duration(name) {
        value >= baseline * 1.5 && value - baseline >= 60.0
    } else {
        // A step measured in tenths: not to be lost to rounding.
        let step = if name == "reversed_rate" { 0.1 } else { 0.2 };
        value - baseline >= step - 1e-9
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symptom {
    pub indicator: String,
    pub baseline: f64,
    pub earlier: f64,
    pub latest: f64,
    /// The cards of the two windows that weigh most on the indicator.
    pub cards: Vec<u64>,
    /// Where the latest window starts (UTC).
    pub window_start: String,
    pub window_days: i64,
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The rows dispatched in `[from, to)`.
pub fn between<'a>(rows: &'a [Row], from: Option<&str>, to: Option<&str>) -> Vec<&'a Row> {
    rows.iter()
        .filter(|r| from.is_none_or(|f| r.dispatched.as_str() >= f))
        .filter(|r| to.is_none_or(|t| r.dispatched.as_str() < t))
        .collect()
}

pub fn find(rows: &[Row], now: DateTime<Utc>, window_days: i64, min_cards: usize) -> Vec<Symptom> {
    let end = stamp(now);
    let start = stamp(now - Duration::days(window_days));
    let before = stamp(now - Duration::days(window_days * 2));
    let baseline = between(rows, None, Some(&before));
    let earlier = between(rows, Some(&before), Some(&start));
    let latest = between(rows, Some(&start), Some(&end));
    let mut found = Vec::new();
    for name in INDICATORS {
        let (Some((base, base_n)), Some((first, first_n)), Some((last, last_n))) = (
            indicator(name, &baseline),
            indicator(name, &earlier),
            indicator(name, &latest),
        ) else {
            continue;
        };
        if base_n < min_cards || first_n + last_n < min_cards {
            continue;
        }
        if !(worse(name, base, first) && worse(name, base, last)) {
            continue;
        }
        let mut heavy: Vec<(f64, u64)> = earlier
            .iter()
            .chain(&latest)
            .filter_map(|r| weight(name, r).map(|w| (w, r.number)))
            .filter(|(w, _)| *w > 0.0)
            .collect();
        heavy.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        found.push(Symptom {
            indicator: name.to_string(),
            baseline: base,
            earlier: first,
            latest: last,
            cards: heavy.into_iter().take(5).map(|(_, card)| card).collect(),
            window_start: start.clone(),
            window_days,
        });
    }
    found
}

/// The key a symptom is recorded under: one per indicator and window.
pub fn key(symptom: &Symptom) -> String {
    format!(
        "symptom:{}:{}",
        symptom.indicator,
        symptom.window_start.get(..10).unwrap_or("")
    )
}

/// Write the symptoms that are not in the log yet; the keys of the new ones.
pub fn record(log: &Log, symptoms: &[Symptom]) -> Result<Vec<String>, super::log::LogError> {
    let mut new = Vec::new();
    for symptom in symptoms {
        let key = key(symptom);
        let (_, added) = log.append(Draft {
            kind: "symptom".to_string(),
            card: None,
            actor: "进化(hub)".to_string(),
            unit: None,
            key: Some(key.clone()),
            data: json!(symptom),
        })?;
        if added {
            new.push(key);
        }
    }
    Ok(new)
}

fn shown(name: &str, value: f64) -> String {
    if is_duration(name) {
        format!("{}s", value.round() as i64)
    } else {
        format!("{value:.2}")
    }
}

pub fn line(symptom: &Symptom) -> String {
    let name = symptom.indicator.as_str();
    format!(
        "{name}: 基线 {} → 前一窗口 {} → 最近 {}({} 天一窗);看 {}",
        shown(name, symptom.baseline),
        shown(name, symptom.earlier),
        shown(name, symptom.latest),
        symptom.window_days,
        symptom
            .cards
            .iter()
            .map(|c| format!("#{c}"))
            .collect::<Vec<_>>()
            .join("、")
    )
}

/// `--now`, or the clock.
pub fn moment(text: Option<&String>) -> Option<DateTime<Utc>> {
    match text {
        None => Some(Utc::now()),
        Some(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
    }
}

/// `argv` is what follows `olp log symptoms`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp log symptoms";
    let (mut window, mut min_cards, mut now) = (7i64, 5usize, None::<String>);
    let (mut write, mut as_json, mut rest) = (false, false, Vec::new());
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
            "--record" => {
                write = true;
                true
            }
            "--window" => match words.next().and_then(|v| v.parse().ok()) {
                Some(days) if days > 0 => {
                    window = days;
                    true
                }
                _ => false,
            },
            "--min-cards" => match words.next().and_then(|v| v.parse().ok()) {
                Some(n) if n > 0 => {
                    min_cards = n;
                    true
                }
                _ => false,
            },
            "--now" => {
                now = words.next().cloned();
                moment(now.as_ref()).is_some()
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
    let log = Log::at(dir);
    let events = match log.history() {
        Ok(events) => events,
        Err(e) => {
            eprintln!("{prog}: {e}");
            return 4;
        }
    };
    let at = moment(now.as_ref()).unwrap_or_else(Utc::now);
    let symptoms = find(&rows(&events), at, window, min_cards);
    let new = if write {
        if let Err(code) = super::duty::gate(prog) {
            return code;
        }
        match record(&log, &symptoms) {
            Ok(new) => new,
            Err(e) => {
                eprintln!("{prog}: {e}");
                return 4;
            }
        }
    } else {
        Vec::new()
    };
    if as_json {
        println!("{}", json!({ "symptoms": symptoms, "recorded": new }));
        return 0;
    }
    if symptoms.is_empty() {
        println!("没有症状:没有哪个指标连续两个窗口都比基线差(或卡不够 {min_cards} 张)");
    }
    for symptom in &symptoms {
        println!("{}", line(symptom));
    }
    if write {
        println!("新记入日志 {} 条", new.len());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(number: u64, day: i64, outer_wait: i64, returns: u64) -> Row {
        let at =
            DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z").unwrap() + Duration::days(day);
        Row {
            number,
            title: String::new(),
            unit: None,
            implementer: None,
            dispatched: stamp(at.with_timezone(&Utc)),
            outcome: Outcome::Accepted,
            to_claim: Some(10),
            work: Some(100),
            outer_wait: Some(outer_wait),
            total: Some(110 + outer_wait),
            returns,
            review_rounds: 0,
            blocked: 0,
            alerts: 0,
            snoozes: 0,
            undelivered: 0,
            first_pass: returns == 0,
            reversed: false,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_symptom_needs_two_bad_windows_and_enough_cards() {
        // Baseline: days 0..13, outer wait 100 s. Windows: days 14..20, 21..27.
        let mut rows: Vec<Row> = (0..10).map(|i| row(i + 1, i as i64, 100, 0)).collect();
        // Only the latest window is slow: not a symptom.
        rows.extend((0..3).map(|i| row(20 + i, 15 + i as i64, 100, 0)));
        rows.extend((0..3).map(|i| row(30 + i, 22 + i as i64, 900 + i as i64, 0)));
        assert!(find(&rows, now(), 7, 5).is_empty());

        // Both windows slow.
        for r in rows.iter_mut().filter(|r| (20..23).contains(&r.number)) {
            r.outer_wait = Some(700);
            r.total = Some(810);
        }
        let found = find(&rows, now(), 7, 5);
        let wait = found.iter().find(|s| s.indicator == "outer_wait").unwrap();
        assert_eq!(
            (wait.baseline, wait.earlier, wait.latest),
            (100.0, 700.0, 901.0)
        );
        // Heaviest first.
        assert_eq!(wait.cards, [32, 31, 30, 20, 21]);
        assert_eq!(key(wait), "symptom:outer_wait:2026-11-22");
        // The total moved with it; nothing else did.
        let names: Vec<&str> = found.iter().map(|s| s.indicator.as_str()).collect();
        assert_eq!(names, ["total", "outer_wait"]);

        // Six cards across the windows are not eight.
        assert!(find(&rows, now(), 7, 8).is_empty());
    }

    #[test]
    fn a_rate_is_worse_by_a_fixed_step_and_a_duration_by_half() {
        assert!(worse("work", 100.0, 160.0));
        assert!(!worse("work", 100.0, 140.0));
        // Half as long again, but under a minute more.
        assert!(!worse("to_claim", 10.0, 30.0));
        assert!(worse("returns_per_card", 0.1, 0.3));
        assert!(!worse("returns_per_card", 0.1, 0.25));
        assert!(worse("reversed_rate", 0.0, 0.1));
        let rows: Vec<Row> = vec![row(1, 0, 10, 1), row(2, 0, 30, 0), row(3, 0, 20, 2)];
        let all: Vec<&Row> = rows.iter().collect();
        assert_eq!(indicator("outer_wait", &all), Some((20.0, 3)));
        assert_eq!(indicator("returns_per_card", &all), Some((1.0, 3)));
        assert_eq!(indicator("total", &[]), None);
    }
}
