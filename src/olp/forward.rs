//! `octoscode olp guard forward` — send what the unit's runtime guard saw
//! to the hub's blackboard as alerts.
//!
//! `octoscode olp-runtime-watch` observes an inner loop on the unit and
//! prints one record per heartbeat; it tells nobody. This reads those
//! records and appends, through the unit's write channel, a line for each
//! thing the outer loop should know:
//!
//! `> 单元(<unit>)·负哨(#<task>): <kind> turn=<turn> <detail>`
//!
//! To a hub that keeps an event log (`store = log`) the same thing goes as
//! an `alert` event through the log channel (`olp remote serve-log`); what
//! could not be sent because the hub was not reached is sent again before
//! the next one.
//!
//! Each thing is reported once while it lasts (the guard repeats it every
//! heartbeat); gone and back is a new one. Only whitelisted fields go out —
//! kind, pid, tool name, sequence number, stall seconds — never output.
//! It judges nothing and acts on nothing. Ends when stdin ends.
//!
//! Exit codes: 0 stdin ended; 2 usage or configuration.

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::mdremote::{EX_TEMPFAIL, Fail, deliver, make_key, resolve};

const DEFAULT_KINDS: [&str; 7] = [
    "agent_missing",
    "backend_missing",
    "backend_rebound",
    "watcher_missing",
    "process_identity_changed",
    "turn_error",
    "stalled_no_progress",
];

/// A value safe on one board line and inside a key.
fn clean(value: Option<&Value>, limit: usize) -> String {
    let text = match value {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._:/@+=-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(limit)
        .collect()
}

fn plain(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// What is worth reporting in one record: (kind, discriminator, detail).
fn events_of(record: &Value) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let word = |v: &Value, path: &str| v.pointer(path).and_then(Value::as_str).map(str::to_string);
    for role in ["agent", "backend"] {
        let state = word(record, &format!("/{role}/state")).unwrap_or_default();
        if matches!(state.as_str(), "process_missing" | "process_zombie") {
            let pid = record.pointer(&format!("/identity/{role}_pid"));
            out.push((
                format!("{role}_missing"),
                plain(pid),
                format!("pid={} state={state}", clean(pid, 80)),
            ));
        }
    }
    for monitor in record["monitors"].as_array().into_iter().flatten() {
        let state = word(monitor, "/state/state").unwrap_or_default();
        if matches!(state.as_str(), "watcher_missing" | "process_zombie") {
            let pid = monitor.get("pid");
            out.push((
                "watcher_missing".to_string(),
                plain(pid),
                format!("pid={} state={state}", clean(pid, 80)),
            ));
        }
    }
    for alert in record["alerts"].as_array().into_iter().flatten() {
        let kind = alert["kind"].as_str().unwrap_or_default();
        let field = |name: &str| clean(alert.get(name), 80);
        let (disc, detail) = match kind {
            "backend_rebound" => {
                let (old, new) = (field("from"), field("to"));
                (
                    format!("{old}->{new}"),
                    format!("from={old} to={new} ledger=not_rebound"),
                )
            }
            "process_identity_changed" => {
                (plain(alert.get("pid")), format!("pid={}", field("pid")))
            }
            "turn_error" => (plain(alert.get("seq")), format!("seq={}", field("seq"))),
            "tool_failed" => (
                plain(alert.get("seq")),
                format!("tool={} seq={}", field("tool_name"), field("seq")),
            ),
            "stalled_no_progress" => (
                String::new(),
                format!(
                    "age={}s activity={}",
                    field("progress_age_seconds"),
                    field("activity")
                ),
            ),
            _ => continue,
        };
        out.push((kind.to_string(), disc, detail));
    }
    out
}

/// `argv` is what follows `olp guard forward`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp-alert-forward";
    let (mut hub, mut unit, mut only) = (None, None, None);
    let mut include: Vec<String> = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        if matches!(word.as_str(), "-h" | "--help") {
            println!(
                "用法: octoscode olp-runtime-watch … | octoscode olp guard forward [--include tool_failed] [--only <种类>[,…]] [--hub H] [--unit U]\n\n把守护的告警作为负哨送到 hub 的黑板。每件事在它持续期间只报一次。\n默认种类:{};另可 --include tool_failed。\n退出码:0 stdin 结束;2 用法或配置错误",
                DEFAULT_KINDS.join("、")
            );
            return 0;
        }
        let Some(value) = words.next().cloned() else {
            eprintln!("{prog}: {word} 需要一个值");
            return 2;
        };
        match word.as_str() {
            "--hub" => hub = Some(value),
            "--unit" => unit = Some(value),
            "--only" => only = Some(value),
            "--include" => include.push(value),
            other => {
                eprintln!("{prog}: 不认识的参数: {other}");
                return 2;
            }
        }
    }
    let mut kinds: BTreeSet<String> = match &only {
        Some(list) => list
            .split(',')
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect(),
        None => DEFAULT_KINDS.iter().map(|k| k.to_string()).collect(),
    };
    kinds.extend(include);
    let unknown: Vec<&str> = kinds
        .iter()
        .map(String::as_str)
        .filter(|k| !DEFAULT_KINDS.contains(k) && *k != "tool_failed")
        .collect();
    if !unknown.is_empty() || kinds.is_empty() {
        eprintln!(
            "{prog}: 未知的种类: {}",
            if unknown.is_empty() {
                "(空)".to_string()
            } else {
                unknown.join(", ")
            }
        );
        return 2;
    }
    // A hub that keeps an event log takes alerts on the log channel; the
    // Markdown channel's forced command is not installed there.
    let on_log = super::client::log_mode();
    let (hub_md, unit_name) = if on_log {
        match super::client::unit_name(hub.as_ref(), unit.as_ref()) {
            Ok(name) => (None, name),
            Err(why) => {
                eprintln!("{prog}: {why}");
                return 2;
            }
        }
    } else {
        match resolve(hub.as_ref(), unit.as_ref()) {
            Ok(found) => {
                let name = found.unit.clone();
                (Some(found), name)
            }
            Err(Fail(_, why)) => {
                eprintln!("{prog}: {why}");
                return 2;
            }
        }
    };
    // Log channel only: alerts the hub was not reached for, oldest first.
    let mut unsent: Vec<(String, String, String)> = Vec::new();
    // What is going on now, and how many times each thing has started.
    let mut active: BTreeSet<(String, String)> = BTreeSet::new();
    let mut episodes: BTreeMap<(String, String), u64> = BTreeMap::new();
    for raw in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(raw.trim()) else {
            continue;
        };
        if !record.is_object() {
            continue;
        }
        // The hub may be back although nothing new has happened here.
        send_waiting(prog, hub.as_ref(), unit.as_ref(), &mut unsent);
        let first_task = record.pointer("/identity/task/0");
        let task = match first_task {
            Some(task) => clean(Some(task), 40),
            None => "unknown".to_string(),
        };
        let label = if !task.is_empty() && task.bytes().all(|b| b.is_ascii_digit()) {
            format!("#{task}")
        } else {
            task.clone()
        };
        // An inner loop without an octos ledger has no turn.
        let turn = match record
            .pointer("/identity/turn_id")
            .filter(|t| !t.is_null() && t.as_str() != Some(""))
        {
            Some(turn) => clean(Some(turn), 60),
            None => "-".to_string(),
        };
        let mut present = BTreeSet::new();
        for (kind, disc, detail) in events_of(&record) {
            if !kinds.contains(&kind) {
                continue;
            }
            let ident = (kind.clone(), disc.clone());
            present.insert(ident.clone());
            if active.contains(&ident) {
                continue;
            }
            let episode = episodes.entry(ident.clone()).or_default();
            *episode += 1;
            active.insert(ident);
            let said = format!("{kind} turn={turn} {detail}");
            let line = format!("> 单元({unit_name})·负哨({label}): {said}\n");
            // Stable for the same thing; a digest keeps the key short.
            let digest =
                Sha256::digest(format!("{task}\n{turn}\n{kind}\n{disc}\n{episode}").as_bytes());
            let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
            let key = match make_key(
                &unit_name,
                Some(&format!("neg:{kind}:{hex}")),
                line.as_bytes(),
            ) {
                Ok(key) => key,
                Err(Fail(_, why)) => {
                    eprintln!("{prog}: {why}");
                    continue;
                }
            };
            let Some(hub_md) = &hub_md else {
                unsent.push((key, label.clone(), said));
                send_waiting(prog, hub.as_ref(), unit.as_ref(), &mut unsent);
                continue;
            };
            let code = deliver(hub_md, &key, line.as_bytes());
            if code != 0 && code != EX_TEMPFAIL {
                eprintln!("{prog}: hub 拒绝了 {kind}(退出码 {code})");
            }
        }
        // What this record no longer shows is over; back again is new.
        active.retain(|ident| present.contains(ident));
    }
    send_waiting(prog, hub.as_ref(), unit.as_ref(), &mut unsent);
    0
}

/// Log channel: send the alerts that wait, oldest first, stopping at the
/// first the hub cannot be reached for. Nothing newer goes out before
/// what waits.
fn send_waiting(
    prog: &str,
    hub: Option<&String>,
    unit: Option<&String>,
    unsent: &mut Vec<(String, String, String)>,
) {
    while let Some((key, label, said)) = unsent.first() {
        match super::client::alert(hub, unit, key, label, said) {
            Ok(Some((0, _))) => {}
            Ok(Some((code, why))) => {
                eprintln!("{prog}: hub 拒绝了一条告警(退出码 {code}){}", {
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(":{why}")
                    }
                });
            }
            Ok(None) => {
                eprintln!("{prog}: 没有连上 hub,{} 条告警留待下次再发", unsent.len());
                return;
            }
            Err(why) => {
                eprintln!("{prog}: {why}");
                return;
            }
        }
        unsent.remove(0);
    }
}
