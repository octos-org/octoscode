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
use super::{policy, unit};

const USAGE: &str = "用法: octoscode olp board stall <日志目录> [--quiet 秒] [--stale-after 秒] [--state 文件] [--policy 命令 | --policy-crate 目录] [--samples 文件] [--json]
      octoscode olp board stall --replay <样本文件> [--policy 命令 | --policy-crate 目录] [--json]

集群是否整体静默:有未了结的卡、占着卡的单元上没有内环在工作、日志已 --quiet 秒(默认 600)
没有新事件时,逐张卡说明在等什么、从哪条事件起、谁能推动。有人在工作就不算。
--policy:把事实(JSON)从 stdin 交给这条命令,由它判定是否静默、每张卡在等谁(stdout 一行 JSON);
命令失败、超时或输出不合规时改用内置规则并在 stderr 说明。也可用环境变量 OLP_STALL_POLICY。
--policy-crate:策略是一个 Rust 包,由 Relax 从源码直接运行(需要带 relax feature 的构建;relax 取 OLP_RELAX 或 PATH)。
--samples:把这次的事实和判定(内置规则的、策略的)追加到样本文件,一行一份 JSON;与上一份只差
秒数的不重复记。也可用环境变量 OLP_STALL_SAMPLES。
--replay:不读日志,把样本文件里的事实逐份重算,比较两个判定:带策略时拿内置规则对这个策略,
不带时拿这个版本的内置规则对样本里当时的判定。全部一致退出码 0,有不同退出码 1。
同一段沉默不连着报:卡的状态变了、或又过了 3 次运行才再报,最多 3 次。
退出码:0(是否静默看输出;--replay 时为全部一致);1 --replay 发现不同;2 用法;4 日志或样本读不了";

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

/// What a policy is told about one open card.
#[derive(Serialize, Deserialize)]
struct CardFact {
    number: u64,
    unit: String,
    /// `inbox` (waiting on the outer loop) or `working`.
    partition: String,
    reason: String,
    claimed: bool,
    last_seq: u64,
    last_ts: String,
    last_kind: String,
    silent_seconds: i64,
    /// What herdr last saw each inner loop on the card's unit doing
    /// (`working`, `idle`, `blocked`, `done`); empty when the unit is unknown.
    agents: Vec<String>,
}

/// Everything the decision may depend on. A policy sees exactly this.
#[derive(Serialize, Deserialize)]
struct Facts {
    quiet: u64,
    silent_seconds: i64,
    working_units: Vec<String>,
    unknown_units: Vec<String>,
    cards: Vec<CardFact>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Verdict {
    number: u64,
    waits_on: String,
    cause: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Decision {
    stalled: bool,
    #[serde(default)]
    alerts: Vec<Verdict>,
}

impl Decision {
    /// What the decision comes to: nothing is reported when it is no stall,
    /// and the order of the alerts does not matter.
    fn outcome(&self) -> Option<BTreeMap<u64, (&str, &str)>> {
        self.stalled.then(|| {
            self.alerts
                .iter()
                .map(|v| (v.number, (v.waits_on.as_str(), v.cause.as_str())))
                .collect()
        })
    }
}

/// One line of a samples file: what was known and what was decided.
#[derive(Serialize, Deserialize)]
struct Sample {
    ts: String,
    facts: Facts,
    builtin: Decision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy: Option<Decision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy_error: Option<String>,
}

/// A samples file keeps this many lines; the older half goes when it is full.
const MAX_SAMPLES: usize = 2000;

/// The sample without what changes by merely waiting.
fn shape(sample: &Sample) -> serde_json::Value {
    let mut value = serde_json::to_value(sample).unwrap_or_default();
    if let Some(map) = value.as_object_mut() {
        map.remove("ts");
    }
    if let Some(facts) = value.get_mut("facts").and_then(|f| f.as_object_mut()) {
        facts.remove("silent_seconds");
        for card in facts
            .get_mut("cards")
            .and_then(|c| c.as_array_mut())
            .into_iter()
            .flatten()
        {
            if let Some(card) = card.as_object_mut() {
                card.remove("silent_seconds");
            }
        }
    }
    value
}

/// Appends `sample` unless the last one recorded differs only in seconds.
fn record(path: &std::path::Path, sample: &Sample) -> Result<(), String> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.trim().is_empty()).collect();
    let same = lines
        .last()
        .and_then(|line| serde_json::from_str::<Sample>(line).ok())
        .is_some_and(|last| shape(&last) == shape(sample));
    if same {
        return Ok(());
    }
    let line = serde_json::to_string(sample).map_err(|e| e.to_string())?;
    if lines.len() >= MAX_SAMPLES {
        lines.drain(..lines.len() - MAX_SAMPLES / 2);
    }
    lines.push(&line);
    std::fs::write(path, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

/// How two decisions on the same facts differ, one line per difference.
fn differences(ours: &Decision, theirs: &Decision, names: (&str, &str)) -> Vec<String> {
    let (a, b) = (ours.outcome(), theirs.outcome());
    let says = |stalled: bool| if stalled { "静默" } else { "不算静默" };
    if a.is_some() != b.is_some() {
        return vec![format!(
            "{}判为{},{}判为{}",
            names.0,
            says(a.is_some()),
            names.1,
            says(b.is_some())
        )];
    }
    let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
    let cards: BTreeSet<u64> = a.keys().chain(b.keys()).copied().collect();
    let show = |v: Option<&(&str, &str)>| match v {
        Some((waits_on, cause)) => format!("等 {waits_on}:{cause}"),
        None => "不报".to_string(),
    };
    cards
        .into_iter()
        .filter(|n| a.get(n) != b.get(n))
        .map(|n| {
            format!(
                "#{n} {}:{};{}:{}",
                names.0,
                show(a.get(&n)),
                names.1,
                show(b.get(&n))
            )
        })
        .collect()
}

/// `--replay`: decide again on recorded facts and compare.
fn replay(prog: &str, file: &str, source: Option<&policy::Source>, as_json: bool) -> i32 {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{prog}: 读不了样本文件 {file}: {e}");
            return 4;
        }
    };
    let names = match source {
        Some(_) => ("内置规则", "策略"),
        None => ("这个版本的内置规则", "当时的判定"),
    };
    let (mut total, mut same) = (0usize, 0usize);
    let mut differing = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let sample: Sample = match serde_json::from_str(line) {
            Ok(sample) => sample,
            Err(e) => {
                eprintln!("{prog}: 样本文件第 {} 行不是样本: {e}", index + 1);
                return 4;
            }
        };
        total += 1;
        let ours = builtin(&sample.facts);
        let found = match source {
            Some(source) => match ask_policy(source, &sample.facts) {
                Ok(theirs) => differences(&ours, &theirs, names),
                Err(why) => vec![format!("策略没有给出可用的判定({why})")],
            },
            None => {
                let then = sample.policy.as_ref().unwrap_or(&sample.builtin);
                differences(&ours, then, names)
            }
        };
        if found.is_empty() {
            same += 1;
        } else {
            differing.push((index + 1, sample.ts, found));
        }
    }
    if as_json {
        let list: Vec<_> = differing
            .iter()
            .map(|(line, ts, found)| json!({"line": line, "ts": ts, "differences": found}))
            .collect();
        println!(
            "{}",
            json!({"samples": total, "same": same, "differing": list})
        );
    } else {
        println!(
            "{total} 份样本:{same} 份一致,{} 份不同({} 对 {})",
            differing.len(),
            names.0,
            names.1
        );
        for (line, ts, found) in &differing {
            for difference in found {
                println!("  第 {line} 行({ts}):{difference}");
            }
        }
    }
    if differing.is_empty() { 0 } else { 1 }
}

/// The rule this tool ships with.
fn builtin(facts: &Facts) -> Decision {
    let stalled = !facts.cards.is_empty()
        && facts.working_units.is_empty()
        && facts.silent_seconds >= facts.quiet as i64;
    let alerts = facts
        .cards
        .iter()
        .map(|c| {
            let (waits_on, cause) = match c.partition.as_str() {
                "inbox" => ("outer", format!("等外环:{}", c.reason)),
                _ if !c.claimed => (
                    "unit",
                    format!("派出后没有领取,{} 上没有内环在工作", c.unit),
                ),
                _ => (
                    "unit",
                    format!("领取后没有回执,{} 上没有内环在工作", c.unit),
                ),
            };
            Verdict {
                number: c.number,
                waits_on: waits_on.to_string(),
                cause,
            }
        })
        .collect();
    Decision { stalled, alerts }
}

/// Asks an external policy. It may only choose among the open cards, name
/// `unit` or `outer`, and give a short cause; anything else is refused and
/// the caller falls back to [`builtin`].
fn ask_policy(source: &policy::Source, facts: &Facts) -> Result<Decision, String> {
    let input = serde_json::to_vec(facts).map_err(|e| e.to_string())?;
    let line = policy::ask(source, input)?;
    let decision: Decision =
        serde_json::from_str(&line).map_err(|e| format!("输出不是约定的 JSON: {e}"))?;
    let open: BTreeSet<u64> = facts.cards.iter().map(|c| c.number).collect();
    let mut named = BTreeSet::new();
    for v in &decision.alerts {
        if !open.contains(&v.number) {
            return Err(format!("#{} 不是未了结的卡", v.number));
        }
        if !named.insert(v.number) {
            return Err(format!("#{} 出现了两次", v.number));
        }
        if v.waits_on != "unit" && v.waits_on != "outer" {
            return Err(format!("#{} 的 waits_on 只能是 unit 或 outer", v.number));
        }
        let length = v.cause.chars().count();
        if length == 0 || length > 200 || v.cause.contains('\n') {
            return Err(format!("#{} 的 cause 要是 1 到 200 字的一行", v.number));
        }
    }
    Ok(decision)
}

/// The status of every inner loop in a unit's fresh heartbeat.
fn agent_statuses(name: &str, stale_after: u64) -> Vec<String> {
    unit::health(name, stale_after)
        .filter(|h| h.state != "stale")
        .and_then(|h| h.beat)
        .and_then(|beat| beat.get("agents").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a.get("status").and_then(|s| s.as_str()).map(String::from))
        .collect()
}

fn last_about(events: &[Event], card: u64) -> Option<&Event> {
    events.iter().rev().find(|e| e.card == Some(card))
}

/// `argv` is what follows `olp board stall`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp board stall";
    let (mut quiet, mut stale_after, mut as_json) = (600u64, unit::DEFAULT_STALE, false);
    let mut state_file: Option<PathBuf> = None;
    let mut source = std::env::var("OLP_STALL_POLICY")
        .ok()
        .filter(|p| !p.is_empty())
        .map(policy::Source::Command);
    let mut samples = std::env::var("OLP_STALL_SAMPLES")
        .ok()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let mut replay_file: Option<String> = None;
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
            "--samples" => match words.next() {
                Some(path) => {
                    samples = Some(PathBuf::from(path));
                    true
                }
                None => false,
            },
            "--replay" => match words.next() {
                Some(path) => {
                    replay_file = Some(path.clone());
                    true
                }
                None => false,
            },
            "--policy" => match words.next() {
                Some(command) => {
                    source = Some(policy::Source::Command(command.clone()));
                    true
                }
                None => false,
            },
            "--policy-crate" => match words.next().map(|dir| policy::package(dir)) {
                Some(Ok(package)) => {
                    source = Some(package);
                    true
                }
                Some(Err(why)) => {
                    eprintln!("{prog}: --policy-crate: {why}");
                    return 2;
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
    if let Some(file) = &replay_file {
        if !rest.is_empty() {
            eprintln!("{prog}: --replay 不读日志,不要再给日志目录\n{USAGE}");
            return 2;
        }
        return replay(prog, file, source.as_ref(), as_json);
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
    let mut facts = Facts {
        quiet,
        silent_seconds: silent_for,
        working_units: working.iter().cloned().collect(),
        unknown_units: unknown.clone(),
        cards: open
            .iter()
            .filter_map(|item| {
                let last = last_about(&events, item.number)?;
                Some(CardFact {
                    number: item.number,
                    unit: item.unit.clone().unwrap_or_default(),
                    partition: if matches!(item.partition, Partition::Inbox) {
                        "inbox".to_string()
                    } else {
                        "working".to_string()
                    },
                    reason: item.reason.clone(),
                    claimed: events.iter().any(|e| {
                        e.card == Some(item.number)
                            && e.kind == "progress"
                            && e.data.get("node").and_then(|n| n.as_str()) == Some("claimed")
                    }),
                    last_seq: last.seq,
                    last_ts: last.ts.clone(),
                    last_kind: last.kind.clone(),
                    silent_seconds: seconds_since(&last.ts, now).unwrap_or(i64::MAX),
                    agents: item
                        .unit
                        .as_deref()
                        .map(|name| agent_statuses(name, stale_after))
                        .unwrap_or_default(),
                })
            })
            .collect(),
    };
    let ours = builtin(&facts);
    let asked = source.as_ref().map(|source| ask_policy(source, &facts));
    if let Some(Err(why)) = &asked {
        eprintln!("{prog}: 策略没有给出可用的判定({why}),改用内置规则");
    }
    if let Some(path) = &samples {
        // Only a moment with open cards has anything to decide.
        if !facts.cards.is_empty() {
            let (theirs, failed) = match &asked {
                Some(Ok(decision)) => (Some(decision.clone()), None),
                Some(Err(why)) => (None, Some(why.clone())),
                None => (None, None),
            };
            let sample = Sample {
                ts: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                facts,
                builtin: ours.clone(),
                policy: theirs,
                policy_error: failed,
            };
            if let Err(e) = record(path, &sample) {
                eprintln!("{prog}: 记不下样本 {}: {e}", path.display());
            }
            facts = sample.facts;
        }
    }
    let decision = match asked {
        Some(Ok(decision)) => decision,
        _ => ours,
    };
    let stalled = decision.stalled;

    // A card that is no longer open is forgotten, so a later stall on a
    // reopened card is reported afresh.
    let open_ids: BTreeSet<String> = open.iter().map(|i| i.number.to_string()).collect();
    st.cards.retain(|card, _| open_ids.contains(card));

    let mut alerts = Vec::new();
    let mut held_back = 0usize;
    if stalled {
        for verdict in decision.alerts {
            let Some(fact) = facts.cards.iter().find(|c| c.number == verdict.number) else {
                continue;
            };
            let seen = st.cards.entry(fact.number.to_string()).or_default();
            let changed = seen.seq != fact.last_seq;
            if changed {
                seen.reports = 0;
            }
            let due = seen.reports == 0 || st.runs >= seen.run + REPEAT_AFTER_RUNS;
            if !due || seen.reports >= MAX_REPORTS {
                held_back += 1;
                continue;
            }
            seen.seq = fact.last_seq;
            seen.run = st.runs;
            seen.reports += 1;
            alerts.push(Alert {
                number: fact.number,
                unit: fact.unit.clone(),
                waits_on: if verdict.waits_on == "outer" {
                    "outer"
                } else {
                    "unit"
                },
                cause: verdict.cause,
                since: fact.last_ts.clone(),
                seq: fact.last_seq,
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
