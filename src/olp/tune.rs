//! `octoscode olp tune <动作> <日志目录>` — the few numbers of the loop
//! that are meant to change with use, kept in the event log.
//!
//! A knob is a named threshold with a built-in default and bounds. Its
//! value is the last `tuned` event for it; with none, the default. So a
//! change is a line in the log — who, why, from what — it is undone by
//! another line, and everything that reads the log sees the same value.
//!
//! | knob | default | read by |
//! |---|---|---|
//! | `review_backlog_max` | 0 (no limit) | `card dispatch`: no new card while this many wait for the outer loop |
//! | `follow_up_after` | 900 s | the unit's watcher, through its inbox: how long a claimed card may be silent |
//!
//! - `list`: every knob, its value and its last change.
//! - `set <knob> <value> --reason …`, `revert <knob>`: a person's change.
//! - `replay <knob> <value>`: what the history would have looked like
//!   under another value, and what became of the cards it touches.
//! - `auto [--apply]`: one change at a time, from a symptom
//!   (`symptom.rs`), on trial. The trial ends when enough cards have been
//!   dispatched since: if the hedge indicator got worse the change is
//!   undone, otherwise it is kept. Without `--apply` it only says what it
//!   would do.
//!
//! | rule | symptom | knob | hedge |
//! |---|---|---|---|
//! | `backlog` | `outer_wait` | `review_backlog_max` down | `total` |
//! | `follow-up` | `work` | `follow_up_after` down by a quarter | `returns_per_card` |
//!
//! Only thresholds are tuned. The log, the locks, the gates and the lease
//! are not knobs.
//!
//! Exit codes: 0; 1 refused (unknown knob, out of bounds, nothing to
//! revert); 2 usage; 4 the log could not be read or written; 6 the
//! outer-duty lease is no longer valid.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use super::log::{Draft, Event, Log};
use super::metrics::{rows, spread};
use super::symptom;

const USAGE: &str = "用法: octoscode olp tune list <日志目录> [--json]\n\
      octoscode olp tune set <日志目录> <旋钮> <值> --reason <为什么> [--symptom 键] [--by 署名]\n\
      octoscode olp tune revert <日志目录> <旋钮> [--reason 为什么] [--by 署名]\n\
      octoscode olp tune replay <日志目录> <旋钮> <值> [--json]\n\
      octoscode olp tune auto <日志目录> [--apply] [--window 7] [--min-cards 5] [--now 时刻] [--json]\n\n\
旋钮是随使用调整的阈值,值记在事件日志里(最后一条 tuned 事件;没有就是默认值),改动可查、可撤。\n\
  review_backlog_max  等外环裁决的卡到这么多张就不再派新卡;0 为不限(默认)\n\
  follow_up_after     单元认领后静默多少秒,值守提醒主持接着办(默认 900)\n\
replay:换一个值,历史上会有哪些不同,那些卡后来怎样。\n\
auto:按症状一次只试一处改动;派出的卡够数后,对冲指标变差就自动撤回,否则保留。不带 --apply 只说会做什么。\n\
退出码:0;1 被拒绝;2 用法;4 日志读写失败;6 主审权已失效";

pub struct Knob {
    pub name: &'static str,
    pub default: u64,
    pub min: u64,
    pub max: u64,
    pub what: &'static str,
}

pub const KNOBS: [Knob; 2] = [
    Knob {
        name: "review_backlog_max",
        default: 0,
        min: 0,
        max: 1000,
        what: "等外环裁决的卡到这么多张就不再派新卡;0 为不限",
    },
    Knob {
        name: "follow_up_after",
        default: 900,
        min: 60,
        max: 86_400,
        what: "单元认领后静默多少秒,值守提醒主持接着办",
    },
];

fn knob(name: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.name == name)
}

fn word<'a>(event: &'a Event, field: &str) -> &'a str {
    event.data.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

/// The `tuned` events for a knob, oldest first.
fn changes<'a>(events: &'a [Event], name: &str) -> Vec<&'a Event> {
    events
        .iter()
        .filter(|e| e.kind == "tuned" && word(e, "name") == name)
        .collect()
}

/// A knob's value now. An unknown knob is 0; a value out of bounds (a log
/// written by hand) is brought back inside them.
pub fn value(events: &[Event], name: &str) -> u64 {
    let Some(knob) = knob(name) else { return 0 };
    changes(events, name)
        .last()
        .and_then(|e| e.data.get("value").and_then(Value::as_u64))
        .map_or(knob.default, |v| v.clamp(knob.min, knob.max))
}

/// Every knob's value, for whoever compares two readings of a log.
pub fn values(events: &[Event]) -> BTreeMap<&'static str, u64> {
    KNOBS
        .iter()
        .map(|k| (k.name, value(events, k.name)))
        .collect()
}

/// How many cards wait for the outer loop when each card is dispatched to
/// a unit: `(card, time, waiting)`.
fn backlog_at_dispatch(events: &[Event]) -> Vec<(u64, String, usize)> {
    let mut waiting: BTreeSet<u64> = BTreeSet::new();
    let mut over: BTreeSet<u64> = BTreeSet::new();
    let mut seen = Vec::new();
    for event in events {
        let Some(card) = event.card else { continue };
        match event.kind.as_str() {
            "dispatched" if event.unit.is_some() => {
                seen.push((card, event.ts.clone(), waiting.len()));
            }
            "ack" if !over.contains(&card) => {
                waiting.insert(card);
            }
            "returned" => {
                waiting.remove(&card);
                over.remove(&card);
            }
            "accepted" | "voided" => {
                waiting.remove(&card);
                over.insert(card);
            }
            _ => {}
        }
    }
    seen
}

/// The silences of claimed cards: `(card, seconds)` between two events of
/// a card while its unit holds it.
fn silences(events: &[Event]) -> Vec<(u64, i64)> {
    let at = |e: &Event| {
        DateTime::parse_from_rfc3339(&e.ts)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    };
    // Per card: held since the last event, if the unit holds it.
    let mut held: BTreeMap<u64, DateTime<Utc>> = BTreeMap::new();
    let mut found = Vec::new();
    for event in events {
        let (Some(card), Some(now)) = (event.card, at(event)) else {
            continue;
        };
        if let Some(since) = held.get(&card) {
            found.push((card, (now - *since).num_seconds().max(0)));
        }
        let claimed = event.kind == "progress" && word(event, "node") == "claimed";
        match event.kind.as_str() {
            "ack" | "accepted" | "voided" => {
                held.remove(&card);
            }
            _ if claimed || held.contains_key(&card) => {
                held.insert(card, now);
            }
            _ => {}
        }
    }
    found
}

/// What history would have looked like under `proposed`.
pub fn replay(events: &[Event], name: &str, proposed: u64) -> Value {
    let current = value(events, name);
    match name {
        "review_backlog_max" => {
            let seen = backlog_at_dispatch(events);
            let held_back = |cap: u64| -> Vec<u64> {
                seen.iter()
                    .filter(|(_, _, waiting)| cap > 0 && *waiting as u64 >= cap)
                    .map(|(card, _, _)| *card)
                    .collect()
            };
            let then = held_back(proposed);
            let all = rows(events);
            let wait = |inside: bool| {
                spread(
                    all.iter()
                        .filter(|r| then.contains(&r.number) == inside)
                        .filter_map(|r| r.outer_wait)
                        .collect(),
                )
            };
            json!({
                "knob": name,
                "current": current,
                "proposed": proposed,
                "dispatches": seen.len(),
                "peak_backlog": seen.iter().map(|(_, _, w)| *w).max().unwrap_or(0),
                "held_back_now": held_back(current).len(),
                "held_back_then": then.len(),
                "cards": then,
                // What became of the cards dispatched into a backlog, and of the rest.
                "outer_wait_of_those": wait(true),
                "outer_wait_of_the_rest": wait(false),
            })
        }
        "follow_up_after" => {
            let all = silences(events);
            // The watcher speaks when a silence reaches the value, and once more at twice that.
            let nudges = |after: u64| -> u64 {
                all.iter()
                    .map(|(_, s)| ((*s as u64) / after.max(1)).min(2))
                    .sum()
            };
            let cards: BTreeSet<u64> = all
                .iter()
                .filter(|(_, s)| *s as u64 >= proposed)
                .map(|(card, _)| *card)
                .collect();
            json!({
                "knob": name,
                "current": current,
                "proposed": proposed,
                "silences": all.len(),
                "follow_ups_now": nudges(current),
                "follow_ups_then": nudges(proposed),
                "cards": cards,
                // The time spent silent past the value: the most a follow-up could have saved.
                "seconds_past_then": all.iter().map(|(_, s)| (*s - proposed as i64).max(0)).sum::<i64>(),
                "seconds_past_now": all.iter().map(|(_, s)| (*s - current as i64).max(0)).sum::<i64>(),
            })
        }
        _ => json!({}),
    }
}

struct Rule {
    name: &'static str,
    trigger: &'static str,
    knob: &'static str,
    hedge: &'static str,
}

const RULES: [Rule; 2] = [
    Rule {
        name: "backlog",
        trigger: "outer_wait",
        knob: "review_backlog_max",
        hedge: "total",
    },
    Rule {
        name: "follow-up",
        trigger: "work",
        knob: "follow_up_after",
        hedge: "returns_per_card",
    },
];

/// The value a rule would try next, if any.
fn next_value(rule: &Rule, events: &[Event], since: &str) -> Option<u64> {
    let current = value(events, rule.knob);
    let knob = knob(rule.knob)?;
    let next = match rule.knob {
        "review_backlog_max" if current == 0 => {
            // Start one under the worst backlog cards were dispatched into.
            let peak = backlog_at_dispatch(events)
                .iter()
                .filter(|(_, ts, _)| ts.as_str() >= since)
                .map(|(_, _, waiting)| *waiting as u64)
                .max()
                .unwrap_or(0);
            if peak < 2 {
                return None;
            }
            peak - 1
        }
        "review_backlog_max" => current.saturating_sub(1).max(1),
        "follow_up_after" => (current * 3 / 4 / 60 * 60).max(knob.min),
        _ => return None,
    };
    (next != current).then_some(next.clamp(knob.min, knob.max))
}

/// The automatic change still on trial: a `tuned` event marked `trial`
/// that no later event judges and no later change of the same knob
/// replaces.
fn open_trial(events: &[Event]) -> Option<&Event> {
    let tuned: Vec<&Event> = events.iter().filter(|e| e.kind == "tuned").collect();
    tuned.iter().rev().copied().find(|e| {
        e.data.get("trial").and_then(Value::as_bool) == Some(true)
            && !tuned.iter().any(|later| {
                later.seq > e.seq
                    && (later.data.get("judges").and_then(Value::as_u64) == Some(e.seq)
                        || word(later, "name") == word(e, "name"))
            })
    })
}

/// What `auto` decides: the event it would write, and why.
#[derive(Debug, Serialize)]
pub struct Decision {
    /// `none`, `waiting`, `try`, `keep` or `revert`.
    pub action: &'static str,
    pub said: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<Value>,
}

fn quiet(action: &'static str, said: String) -> Decision {
    Decision {
        action,
        said,
        change: None,
    }
}

pub fn decide(
    events: &[Event],
    now: DateTime<Utc>,
    window_days: i64,
    min_cards: usize,
) -> Decision {
    let all = rows(events);
    let stamp = |at: DateTime<Utc>| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Some(trial) = open_trial(events) {
        let (name, hedge) = (word(trial, "name"), word(trial, "hedge"));
        let since = symptom::between(&all, Some(&trial.ts), Some(&stamp(now)));
        let Some((after, n)) = symptom::indicator(hedge, &since).filter(|(_, n)| *n >= min_cards)
        else {
            return quiet(
                "waiting",
                format!(
                    "{name} 的改动(第 {} 条事件)还在观察:之后派出的卡不够 {min_cards} 张,先不判",
                    trial.seq
                ),
            );
        };
        let before = trial.data.get("hedge_before").and_then(Value::as_f64);
        let tried = trial.data.get("value").and_then(Value::as_u64).unwrap_or(0);
        let previous = trial
            .data
            .get("previous")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let bad = before.is_some_and(|b| symptom::worse(hedge, b, after));
        let (action, value, previous, verdict, said) = if bad {
            (
                "revert",
                previous,
                tried,
                "reverted",
                format!(
                    "{name} 撤回到 {previous}:对冲指标 {hedge} 从 {:.2} 变到 {after:.2}({n} 张卡)",
                    before.unwrap_or(0.0)
                ),
            )
        } else {
            (
                "keep",
                tried,
                tried,
                "kept",
                format!("{name} 保留为 {tried}:对冲指标 {hedge} 没有变差({after:.2},{n} 张卡)"),
            )
        };
        return Decision {
            action,
            said: said.clone(),
            change: Some(json!({
                "name": name, "value": value, "previous": previous, "source": "auto",
                "reason": said, "judges": trial.seq, "verdict": verdict,
                "hedge": hedge, "hedge_before": before, "hedge_after": after,
            })),
        };
    }
    let symptoms = symptom::find(&all, now, window_days, min_cards);
    let since = stamp(now - Duration::days(window_days * 2));
    for rule in &RULES {
        let Some(found) = symptoms.iter().find(|s| s.indicator == rule.trigger) else {
            continue;
        };
        let Some(next) = next_value(rule, events, &since) else {
            continue;
        };
        let current = value(events, rule.knob);
        let recent = symptom::between(&all, Some(&since), Some(&stamp(now)));
        let hedge_before = symptom::indicator(rule.hedge, &recent).map(|(v, _)| v);
        let said = format!(
            "{} 从 {current} 试改为 {next}(规则 {}):{}",
            rule.knob,
            rule.name,
            symptom::line(found)
        );
        return Decision {
            action: "try",
            said: said.clone(),
            change: Some(json!({
                "name": rule.knob, "value": next, "previous": current, "source": "auto",
                "reason": said, "trial": true, "rule": rule.name,
                "symptom": symptom::key(found), "cards": found.cards,
                "hedge": rule.hedge, "hedge_before": hedge_before,
            })),
        };
    }
    quiet(
        "none",
        if symptoms.is_empty() {
            "没有症状,不改".to_string()
        } else {
            "有症状,但没有哪条规则能再调".to_string()
        },
    )
}

fn append(log: &Log, data: Value, by: &str) -> Result<Event, super::log::LogError> {
    log.append(Draft {
        kind: "tuned".to_string(),
        card: None,
        actor: by.to_string(),
        unit: None,
        key: None,
        data,
    })
    .map(|(event, _)| event)
}

/// Decide and, when something is to be written, write it. For the outer
/// loop's watcher (`OLP_TUNE_AUTO=1`) and `tune auto --apply`.
pub fn auto_apply(
    log: &Log,
    now: DateTime<Utc>,
    window_days: i64,
    min_cards: usize,
) -> Result<Decision, String> {
    let events = log.history().map_err(|e| e.to_string())?;
    let decision = decide(&events, now, window_days, min_cards);
    if let Some(change) = &decision.change {
        append(log, change.clone(), "进化(hub)").map_err(|e| e.to_string())?;
    }
    Ok(decision)
}

struct Words {
    rest: Vec<String>,
    flags: BTreeMap<String, String>,
    on: BTreeSet<String>,
}

fn parse(argv: &[String]) -> Result<Option<Words>, String> {
    let mut w = Words {
        rest: Vec::new(),
        flags: BTreeMap::new(),
        on: BTreeSet::new(),
    };
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" | "--apply" => {
                w.on.insert(word.clone());
            }
            "--reason" | "--symptom" | "--by" | "--window" | "--min-cards" | "--now" => {
                let value = words.next().ok_or(format!("{word} 需要一个值"))?;
                w.flags.insert(word.clone(), value.clone());
            }
            other if other.starts_with("--") => return Err(format!("不认识的参数: {other}")),
            other => w.rest.push(other.to_string()),
        }
    }
    Ok(Some(w))
}

/// `argv` is what follows `olp tune`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp tune";
    let w = match parse(argv) {
        Ok(Some(w)) => w,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(why) => {
            eprintln!("{prog}: {why}\n{USAGE}");
            return 2;
        }
    };
    let usage = || {
        eprintln!("{USAGE}");
        2
    };
    let (Some(action), Some(dir)) = (w.rest.first(), w.rest.get(1)) else {
        return usage();
    };
    if !Path::new(dir).is_dir() {
        eprintln!("{prog}: 日志目录不存在: {dir}");
        return 2;
    }
    let log = Log::at(dir);
    let as_json = w.on.contains("--json");
    let by = w.flags.get("--by").map_or("外环(hub)", String::as_str);
    let read = || {
        log.history().map_err(|e| {
            eprintln!("{prog}: {e}");
            4
        })
    };
    let refuse = |why: String| {
        eprintln!("{prog}: {why},未写入");
        1
    };
    let write = |data: Value| -> i32 {
        if let Err(code) = super::duty::gate(prog) {
            return code;
        }
        match append(&log, data, by) {
            Ok(event) => {
                println!(
                    "{} = {}(第 {} 条事件)",
                    word(&event, "name"),
                    event.data["value"],
                    event.seq
                );
                0
            }
            Err(e) => {
                eprintln!("{prog}: {e}");
                4
            }
        }
    };
    match (action.as_str(), &w.rest[2..]) {
        ("list", []) => {
            let events = match read() {
                Ok(events) => events,
                Err(code) => return code,
            };
            let trial = open_trial(&events).map(|e| e.seq);
            let knobs: Vec<Value> = KNOBS
                .iter()
                .map(|k| {
                    let last = changes(&events, k.name).last().copied();
                    json!({
                        "name": k.name, "value": value(&events, k.name), "default": k.default,
                        "min": k.min, "max": k.max, "what": k.what,
                        "changed": last.map(|e| json!({
                            "seq": e.seq, "ts": e.ts, "by": e.actor, "source": word(e, "source"),
                            "reason": word(e, "reason"), "on_trial": Some(e.seq) == trial,
                        })),
                    })
                })
                .collect();
            if as_json {
                println!("{}", json!({ "knobs": knobs }));
                return 0;
            }
            for k in &knobs {
                println!(
                    "{} = {}(默认 {})  {}",
                    k["name"].as_str().unwrap_or(""),
                    k["value"],
                    k["default"],
                    k["what"].as_str().unwrap_or("")
                );
                if let Some(c) = k["changed"].as_object() {
                    println!(
                        "    第 {} 条事件 {} {}{}:{}",
                        c["seq"],
                        c["ts"].as_str().unwrap_or(""),
                        c["by"].as_str().unwrap_or(""),
                        if c["on_trial"] == true {
                            "(试行中)"
                        } else {
                            ""
                        },
                        c["reason"].as_str().unwrap_or("")
                    );
                }
            }
            0
        }
        ("set", [name, text]) => {
            let Some(knob) = knob(name) else {
                return refuse(format!("没有这个旋钮: {name}"));
            };
            let Some(new) = text
                .parse::<u64>()
                .ok()
                .filter(|v| (knob.min..=knob.max).contains(v))
            else {
                return refuse(format!("{name} 的值要在 {} 到 {} 之间", knob.min, knob.max));
            };
            let Some(reason) = w.flags.get("--reason").filter(|r| !r.trim().is_empty()) else {
                eprintln!("{prog}: 改旋钮要写 --reason");
                return 2;
            };
            let events = match read() {
                Ok(events) => events,
                Err(code) => return code,
            };
            let current = value(&events, name);
            if current == new {
                println!("{name} 已经是 {new},没有写入");
                return 0;
            }
            let mut data = json!({
                "name": name, "value": new, "previous": current,
                "source": "manual", "reason": reason,
            });
            if let Some(symptom) = w.flags.get("--symptom") {
                data["symptom"] = json!(symptom);
            }
            write(data)
        }
        ("revert", [name]) => {
            if knob(name).is_none() {
                return refuse(format!("没有这个旋钮: {name}"));
            }
            let events = match read() {
                Ok(events) => events,
                Err(code) => return code,
            };
            let Some(last) = changes(&events, name).last().copied() else {
                return refuse(format!("{name} 没有改动过"));
            };
            let current = value(&events, name);
            let previous = last
                .data
                .get("previous")
                .and_then(Value::as_u64)
                .unwrap_or(current);
            if previous == current {
                return refuse(format!("{name} 上一次改动没有改变它的值"));
            }
            let reason = w
                .flags
                .get("--reason")
                .cloned()
                .unwrap_or_else(|| format!("撤回第 {} 条事件的改动", last.seq));
            write(json!({
                "name": name, "value": previous, "previous": current,
                "source": "manual", "reason": reason, "reverts": last.seq,
            }))
        }
        ("replay", [name, text]) => {
            let (Some(_), Ok(proposed)) = (knob(name), text.parse::<u64>()) else {
                return refuse(format!("没有这个旋钮,或值不是数: {name} {text}"));
            };
            let events = match read() {
                Ok(events) => events,
                Err(code) => return code,
            };
            let report = replay(&events, name, proposed);
            if as_json {
                println!("{report}");
                return 0;
            }
            let cards = report["cards"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .map(|n| format!("#{n}"))
                        .collect::<Vec<_>>()
                        .join("、")
                })
                .unwrap_or_default();
            match name.as_str() {
                "review_backlog_max" => {
                    println!(
                        "{name}: 现在 {},换成 {proposed} — 历史上 {} 次派卡,当时积压最多 {} 张;会被拦下的派卡:现在的值 {} 次,新值 {} 次",
                        report["current"],
                        report["dispatches"],
                        report["peak_backlog"],
                        report["held_back_now"],
                        report["held_back_then"]
                    );
                    let shown = |v: &Value| match v["p50"].as_i64() {
                        Some(p50) => format!("中位 {p50}s({} 张)", v["cards"]),
                        None => "无数据".to_string(),
                    };
                    println!(
                        "  那些卡后来等外环裁决:{};其余的卡:{}",
                        shown(&report["outer_wait_of_those"]),
                        shown(&report["outer_wait_of_the_rest"])
                    );
                }
                _ => println!(
                    "{name}: 现在 {},换成 {proposed} — 历史上 {} 段在手的静默;提醒次数至多:现在的值 {} 次,新值 {} 次;超过阈值的静默共 {} 秒(现在的值 {} 秒)",
                    report["current"],
                    report["silences"],
                    report["follow_ups_now"],
                    report["follow_ups_then"],
                    report["seconds_past_then"],
                    report["seconds_past_now"]
                ),
            }
            if !cards.is_empty() {
                println!("  涉及的卡:{cards}");
            }
            0
        }
        ("auto", []) => {
            let number = |flag: &str, default: i64| match w.flags.get(flag) {
                None => Some(default),
                Some(text) => text.parse::<i64>().ok().filter(|n| *n > 0),
            };
            let (Some(window), Some(min_cards), Some(now)) = (
                number("--window", 7),
                number("--min-cards", 5),
                symptom::moment(w.flags.get("--now")),
            ) else {
                return usage();
            };
            let decision = if w.on.contains("--apply") {
                if let Err(code) = super::duty::gate(prog) {
                    return code;
                }
                match auto_apply(&log, now, window, min_cards as usize) {
                    Ok(decision) => decision,
                    Err(why) => {
                        eprintln!("{prog}: {why}");
                        return 4;
                    }
                }
            } else {
                match read() {
                    Ok(events) => decide(&events, now, window, min_cards as usize),
                    Err(code) => return code,
                }
            };
            if as_json {
                println!(
                    "{}",
                    json!({ "applied": w.on.contains("--apply"), "decision": decision })
                );
            } else {
                let wrote = decision.change.is_some() && w.on.contains("--apply");
                println!(
                    "{}{}",
                    if decision.change.is_some() && !wrote {
                        "(未写入,加 --apply 才生效)"
                    } else {
                        ""
                    },
                    decision.said
                );
            }
            0
        }
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: u64, minute: i64, kind: &str, card: Option<u64>, data: Value) -> Event {
        let base = DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z").unwrap();
        Event {
            seq,
            ts: (base + Duration::minutes(minute))
                .with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            kind: kind.to_string(),
            card,
            actor: "t".to_string(),
            unit: Some("m07".to_string()),
            key: None,
            data,
        }
    }

    #[test]
    fn a_knob_is_its_last_change_within_its_bounds() {
        let mut events = vec![];
        assert_eq!(value(&events, "follow_up_after"), 900);
        assert_eq!(value(&events, "review_backlog_max"), 0);
        events.push(event(
            1,
            0,
            "tuned",
            None,
            json!({"name": "follow_up_after", "value": 600}),
        ));
        events.push(event(
            2,
            1,
            "tuned",
            None,
            json!({"name": "review_backlog_max", "value": 3}),
        ));
        events.push(event(
            3,
            2,
            "tuned",
            None,
            json!({"name": "follow_up_after", "value": 5}),
        ));
        // Below the knob's minimum: brought back to it.
        assert_eq!(value(&events, "follow_up_after"), 60);
        assert_eq!(value(&events, "review_backlog_max"), 3);
        assert_eq!(value(&events, "no_such_knob"), 0);
    }

    #[test]
    fn a_replay_counts_what_another_value_would_have_changed() {
        let c = |n| Some(n);
        let events = vec![
            event(1, 0, "dispatched", c(1), json!({})),
            event(2, 1, "ack", c(1), json!({"result": "done"})),
            // One card waits for the outer loop when #2 goes out, two when #3 does.
            event(3, 2, "dispatched", c(2), json!({})),
            event(4, 3, "ack", c(2), json!({"result": "done"})),
            event(5, 4, "dispatched", c(3), json!({})),
            event(6, 5, "progress", c(3), json!({"node": "claimed"})),
            // 40 minutes of silence on a claimed card.
            event(7, 45, "ack", c(3), json!({"result": "done"})),
            event(8, 50, "accepted", c(1), json!({})),
        ];
        let report = replay(&events, "review_backlog_max", 2);
        assert_eq!(report["dispatches"], 3);
        assert_eq!(report["peak_backlog"], 2);
        assert_eq!(report["held_back_now"], 0);
        assert_eq!(report["cards"], json!([3]));
        let report = replay(&events, "review_backlog_max", 1);
        assert_eq!(report["cards"], json!([2, 3]));

        let report = replay(&events, "follow_up_after", 600);
        assert_eq!(report["silences"], 1);
        // 2400 s of silence: one follow-up at 900 s and one at 1800 s now; two under 600 s.
        assert_eq!(
            (
                report["follow_ups_now"].as_u64(),
                report["follow_ups_then"].as_u64()
            ),
            (Some(2), Some(2))
        );
        assert_eq!(report["seconds_past_then"], 1800);
        assert_eq!(report["seconds_past_now"], 1500);
        let report = replay(&events, "follow_up_after", 3600);
        assert_eq!(report["follow_ups_then"], 0);
    }
}
