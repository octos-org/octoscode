//! `octoscode olp board stall <日志目录>` — has the whole cluster gone
//! quiet with work unfinished?
//!
//! A card that takes long is not a problem while somebody is working on
//! it: a long build is normal. A stall is when cards are open, no inner
//! loop on any unit that holds one is working, and nothing has been
//! written to the log for a while — everybody waits for somebody else and
//! nobody will move unless told. Then this says, per card, what it waits
//! on, since when (by event number) and who can move it. The same silence
//! is not reported twice in a row: again only when the card's state
//! changed or three more runs have passed, and three times at most.
//!
//! Read-only on the log; it keeps its own small state file. Run it on a
//! timer on the hub. Exit codes: 0 (stalled or not — read the output);
//! 2 usage; 4 the log could not be read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::inbox::{Partition, items};
use super::log::{Event, Log};
use super::unit;

const USAGE: &str = "用法: octoscode olp board stall <日志目录> [--quiet 秒] [--stale-after 秒] [--state 文件] [--json]

集群是否整体静默:有未了结的卡、占着卡的单元上没有内环在工作、日志已 --quiet 秒(默认 600)
没有新事件时,逐张卡说明在等什么、从哪条事件起、谁能推动。有人在工作就不算。
同一段沉默不连着报:卡的状态变了、或又过了 3 次运行才再报,最多 3 次。
退出码:0(是否静默看输出);2 用法;4 日志读不了";

/// Runs between two reports of the same silence, and how many reports.
const REPEAT_AFTER_RUNS: u64 = 3;
const MAX_REPORTS: u32 = 3;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Seen {
    /// The card's last event when it was last reported.
    seq: u64,
    /// The run it was last reported on.
    run: u64,
    reports: u32,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct StallState {
    runs: u64,
    cards: BTreeMap<String, Seen>,
}

#[derive(Serialize)]
struct Alert {
    number: u64,
    unit: String,
    /// Who can move it: `unit` or `outer`.
    waits_on: &'static str,
    cause: String,
    since: String,
    seq: u64,
    reports: u32,
}

fn seconds_since(ts: &str, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let at = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    Some((now - at.with_timezone(&chrono::Utc)).num_seconds())
}

/// Units with a fresh heartbeat in which herdr saw a working inner loop.
fn working_units(units: &BTreeSet<String>, stale_after: u64) -> (BTreeSet<String>, Vec<String>) {
    let mut working = BTreeSet::new();
    let mut unknown = Vec::new();
    for name in units {
        let fresh = unit::health(name, stale_after)
            .filter(|h| h.state != "stale")
            .and_then(|h| h.beat);
        let Some(beat) = fresh else {
            unknown.push(name.clone());
            continue;
        };
        let busy = beat
            .get("agents")
            .and_then(|v| v.as_array())
            .is_some_and(|list| {
                list.iter()
                    .any(|a| a.get("status").and_then(|s| s.as_str()) == Some("working"))
            });
        if busy {
            working.insert(name.clone());
        }
    }
    (working, unknown)
}

fn last_about(events: &[Event], card: u64) -> Option<&Event> {
    events.iter().rev().find(|e| e.card == Some(card))
}

/// `argv` is what follows `olp board stall`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp board stall";
    let (mut quiet, mut stale_after, mut as_json) = (600u64, unit::DEFAULT_STALE, false);
    let mut state_file: Option<PathBuf> = None;
    let mut rest = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let mut number = |slot: &mut u64| match words.next().and_then(|v| v.parse::<u64>().ok()) {
            Some(value) => {
                *slot = value;
                true
            }
            None => false,
        };
        let ok = match word.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--json" => {
                as_json = true;
                true
            }
            "--quiet" => number(&mut quiet),
            "--stale-after" => number(&mut stale_after) && stale_after > 0,
            "--state" => match words.next() {
                Some(path) => {
                    state_file = Some(PathBuf::from(path));
                    true
                }
                None => false,
            },
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
    if !std::path::Path::new(dir).is_dir() {
        eprintln!("{prog}: 日志目录不存在: {dir}");
        return 2;
    }
    let events = match Log::at(dir).read() {
        Ok(events) => events,
        Err(e) => {
            eprintln!("{prog}: {e}");
            return 4;
        }
    };
    let state_file = state_file.unwrap_or_else(|| PathBuf::from(dir).join("stall-state.json"));
    let mut st: StallState = std::fs::read_to_string(&state_file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    st.runs += 1;

    let now = chrono::Utc::now();
    let mut scan = super::cards::scan(&events);
    let open: Vec<_> = items(&mut scan, now.date_naive(), |e| e.unit.is_some())
        .into_iter()
        // A snoozed card waits on purpose.
        .filter(|i| matches!(i.partition, Partition::Inbox | Partition::Working))
        .collect();
    let holders: BTreeSet<String> = open.iter().filter_map(|i| i.unit.clone()).collect();
    let (working, unknown) = working_units(&holders, stale_after);
    let silent_for = events
        .last()
        .and_then(|e| seconds_since(&e.ts, now))
        .unwrap_or(i64::MAX);
    let stalled = !open.is_empty() && working.is_empty() && silent_for >= quiet as i64;

    // A card that is no longer open is forgotten, so a later stall on a
    // reopened card is reported afresh.
    let open_ids: BTreeSet<String> = open.iter().map(|i| i.number.to_string()).collect();
    st.cards.retain(|card, _| open_ids.contains(card));

    let mut alerts = Vec::new();
    let mut held_back = 0usize;
    if stalled {
        for item in &open {
            let Some(last) = last_about(&events, item.number) else {
                continue;
            };
            let claimed = events.iter().any(|e| {
                e.card == Some(item.number)
                    && e.kind == "progress"
                    && e.data.get("node").and_then(|n| n.as_str()) == Some("claimed")
            });
            let target = item.unit.clone().unwrap_or_default();
            let (waits_on, cause) = match item.partition {
                Partition::Inbox => ("outer", format!("等外环:{}", item.reason)),
                _ if !claimed => ("unit", format!("派出后没有领取,{target} 上没有内环在工作")),
                _ => ("unit", format!("领取后没有回执,{target} 上没有内环在工作")),
            };
            let seen = st.cards.entry(item.number.to_string()).or_default();
            let changed = seen.seq != last.seq;
            if changed {
                seen.reports = 0;
            }
            let due = seen.reports == 0 || st.runs >= seen.run + REPEAT_AFTER_RUNS;
            if !due || seen.reports >= MAX_REPORTS {
                held_back += 1;
                continue;
            }
            seen.seq = last.seq;
            seen.run = st.runs;
            seen.reports += 1;
            alerts.push(Alert {
                number: item.number,
                unit: target,
                waits_on,
                cause,
                since: last.ts.clone(),
                seq: last.seq,
                reports: seen.reports,
            });
        }
    }
    if let Ok(text) = serde_json::to_string(&st) {
        if let Err(e) = std::fs::write(&state_file, text) {
            eprintln!("{prog}: 记不下已报告的卡 {}: {e}", state_file.display());
        }
    }

    if as_json {
        println!(
            "{}",
            json!({
                "stalled": stalled,
                "open": open.len(),
                "working_units": working,
                "unknown_units": unknown,
                "silent_seconds": if events.is_empty() { None } else { Some(silent_for) },
                "alerts": alerts,
                "held_back": held_back,
            })
        );
        return 0;
    }
    if open.is_empty() {
        println!("没有未了结的卡");
    } else if !working.is_empty() {
        let names: Vec<&str> = working.iter().map(String::as_str).collect();
        println!(
            "没有静默:{} 张卡未了结,{} 上有内环在工作",
            open.len(),
            names.join("、")
        );
    } else if !stalled {
        println!(
            "还没到静默的时间:{} 张卡未了结,没有内环在工作,日志已 {silent_for} 秒没有新事件(上限 {quiet} 秒)",
            open.len()
        );
    } else {
        for a in &alerts {
            println!(
                "[stall] #{} 单元 {}:{},自 {} 起(事件 {});能推动的是{}",
                a.number,
                a.unit,
                a.cause,
                a.since,
                a.seq,
                if a.waits_on == "outer" {
                    "外环"
                } else {
                    "该单元(看它的窗格)"
                }
            );
        }
        if alerts.is_empty() {
            println!("仍然静默,{held_back} 张卡已报告过,没有变化");
        }
    }
    if !unknown.is_empty() && !open.is_empty() {
        println!(
            "看不到这些单元的内环在不在工作(未登记或心跳失联):{}",
            unknown.join("、")
        );
    }
    0
}
