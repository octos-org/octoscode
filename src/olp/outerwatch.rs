//! `octoscode olp outer watch <日志目录> --pane <窗格>` — the outer loop's
//! side of "inbox plus wake": tell the outer loop's pane, with one fixed
//! sentence, that something on the event log now waits for it.
//!
//! What waits for the outer loop: a unit's ACK, a recorded push, a question,
//! a delivery a unit reported as failed, what a unit's runtime guard saw on
//! one of its cards (an inner loop gone, stalled) — and a stall of the whole cluster
//! (`olp board stall`). The sentence carries none of it; it only says where
//! to look. The outer loop is not polled and never polls.
//!
//! The watcher keeps its own cursor: the highest event number it has handed
//! over. An event that arrives while the outer loop is busy is not slept
//! through — the cursor moves only when the sentence was taken.
//!
//! A stall is asked about without spending anything (`board stall --peek`):
//! the reports of a stall are counted when the outer loop itself runs
//! `olp board stall`, which is where it reads the card numbers. The watcher
//! remembers which stall it has pointed to, and says it again — three times
//! at most — only while the outer loop has still not looked.
//!
//! Each round also keeps the log small: once the part in use is over
//! `OLP_LOG_COMPACT_MIB` (32 by default, 0 for never) and twice what the
//! last compaction left, it is compacted as `olp log compact` does.
//!
//! Exit codes: 0 (with `--once`: the round is done); 2 usage; 4 the log
//! could not be read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::log::{Event, Log};

const USAGE: &str = "用法: octoscode olp outer watch <日志目录> --pane <外环的窗格> | --label <外环工作区的标签> [--once] [--interval 30] [--quiet 600] [--idle-wait 5] [--session herdr 会话]

外环值守:日志里有等外环处理的新事件(单元的回执、推送登记、提问、送达失败、未了结卡上的守护告警),或集群整体静默时,
在外环的 agent 空闲时往它的窗格发一句固定短句。不带任何内容。
--label:每轮按标签找外环的窗格(olp outer install 注册的值守任务这样用);没有这个工作区时这一轮不提醒。
退出码:0;2 用法;4 日志读不了";

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Cursor {
    /// The highest event number handed over to the outer loop.
    seen: u64,
    /// Stalls pointed to, by card number.
    stalls: BTreeMap<String, Pointed>,
    /// The log's size in use when it was last compacted, or was found to
    /// have nothing to archive.
    compacted_at: u64,
}

/// One stall the outer loop's pane was told about.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Pointed {
    /// The card's last event at the time.
    seq: u64,
    /// Unix seconds of the last time it was said.
    at: u64,
    times: u32,
}

/// How often one stall is pointed to while the outer loop has not looked.
const STALL_REMINDERS: u32 = 3;

/// The one sentence the outer loop's pane is woken with.
pub fn sentence(log: &str) -> String {
    format!(
        "有等外环处理的新事件,请执行:octoscode olp inbox {log};整体静默时另见 octoscode olp board stall {log}"
    )
}

/// Does this event wait for the outer loop?
fn for_outer(event: &Event) -> bool {
    match event.kind.as_str() {
        "ack" | "pushed" | "asked" => true,
        "progress" => event.data.get("node").and_then(|n| n.as_str()) == Some("undelivered"),
        // What the unit's guard saw on a card; one tied to no card has
        // nothing the outer loop could decide.
        "alert" => event.card.is_some(),
        _ => false,
    }
}

fn herdr(args: &[&str], session: Option<&str>) -> Option<std::process::Output> {
    let exe = std::env::var("OLP_HERDR").ok().filter(|v| !v.is_empty());
    let mut cmd = Command::new(exe.as_deref().unwrap_or("herdr"));
    if let Some(session) = session {
        cmd.args(["--session", session]);
    }
    cmd.args(args).stdin(Stdio::null()).output().ok()
}

/// The stalls `olp board stall` would report now, as (card, its last
/// event). Asked with `--peek`: nothing is counted as reported.
fn stall_alerts(log: &str, quiet: u64) -> Vec<(u64, u64)> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    Command::new(exe)
        .args([
            "olp",
            "board",
            "stall",
            log,
            "--json",
            "--peek",
            "--quiet",
            &quiet.to_string(),
        ])
        .output()
        .ok()
        .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
        .and_then(|doc| {
            doc["alerts"].as_array().map(|alerts| {
                alerts
                    .iter()
                    .filter_map(|a| Some((a["number"].as_u64()?, a["seq"].as_u64()?)))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// `argv` is what follows `olp outer watch`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp outer watch";
    let (mut once, mut interval, mut quiet, mut idle_wait) = (false, 30u64, 600u64, 5u64);
    let (mut pane, mut session, mut rest) = (None, None, Vec::new());
    let mut label: Option<String> = None;
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
            "--once" => {
                once = true;
                true
            }
            "--interval" => number(&mut interval),
            "--quiet" => number(&mut quiet),
            "--idle-wait" => number(&mut idle_wait),
            "--pane" => {
                pane = words.next().cloned();
                pane.is_some()
            }
            "--label" => {
                label = words.next().cloned();
                label.as_deref().is_some_and(|l| !l.is_empty())
            }
            "--session" => {
                session = words.next().cloned();
                session.is_some()
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
    let [log] = rest.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };
    if pane.is_some() == label.is_some() {
        eprintln!("{prog}: --pane 与 --label 给且只给一个\n{USAGE}");
        return 2;
    }
    if pane.as_deref().is_some_and(|p| !super::card::is_pane(p)) || !Path::new(log).is_dir() {
        eprintln!("{prog}: 窗格标识不合法,或日志目录不存在\n{USAGE}");
        return 2;
    }
    let state_path = std::env::var_os("OLP_OUTER_WATCH_STATE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let real = std::fs::canonicalize(log).unwrap_or_else(|_| PathBuf::from(log));
            let digest = Sha256::digest(real.to_string_lossy().as_bytes());
            let name: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(format!(".octos/outer/watch/outer-{name}.json"))
        });
    let session = session.as_deref();
    let read_cursor = || -> Cursor {
        std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    };
    let write_cursor = |cursor: &Cursor| {
        let written = state_path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|()| {
                std::fs::write(
                    &state_path,
                    serde_json::to_string(cursor).unwrap_or_default(),
                )
            });
        if let Err(e) = written {
            eprintln!("{prog}: 记不下读到哪条 {}: {e}", state_path.display());
        }
    };
    loop {
        // Every command replays the log: keep what is over out of its way.
        // Tried when the log in use has grown large, and not again until
        // it has grown by a quarter more.
        if let (Some(least), Ok((base, segments))) =
            (super::compact::auto_threshold(), Log::at(log).bytes())
        {
            let size = base + segments;
            let mut cursor = read_cursor();
            if size >= least.max(cursor.compacted_at / 4 * 5) {
                match super::compact::compact(Path::new(log), 14, 10, Some(least), false) {
                    Ok(done) => {
                        if done.compacted {
                            println!(
                                "已压缩日志:归档 {} 张卡,在用 {} → {} 字节",
                                done.cards_archived.len(),
                                done.bytes_before,
                                done.bytes_after
                            );
                        }
                        cursor.compacted_at = done.bytes_after;
                    }
                    Err(super::compact::Failed::Log(why))
                    | Err(super::compact::Failed::Differs(why)) => {
                        eprintln!("{prog}: 日志没有压缩: {why}");
                        cursor.compacted_at = size;
                    }
                }
                write_cursor(&cursor);
            }
        }
        let events = match Log::at(log).read() {
            Ok(events) => events,
            Err(e) => {
                eprintln!("{prog}: {e}");
                return 4;
            }
        };
        let mut cursor = read_cursor();
        let head = events.last().map(|e| e.seq).unwrap_or(0);
        // What is already over does not wait for anyone: a watcher that
        // starts on an old log says nothing about its history.
        let settled = super::cards::settled(&events, chrono::Local::now().date_naive());
        let open_questions = super::cards::pending_questions(&events);
        let still_open = |e: &Event| match e.kind.as_str() {
            "asked" => {
                let id = e.data.get("id").and_then(|v| v.as_str());
                open_questions.iter().any(|q| Some(q.id.as_str()) == id)
            }
            _ => e.card.is_none_or(|n| !settled.contains(&n)),
        };
        let waiting = events
            .iter()
            .filter(|e| e.seq > cursor.seen && for_outer(e) && still_open(e))
            .count();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // A stall not pointed to yet, or one the outer loop was told of a
        // quiet period ago and has still not looked at.
        let stalls: Vec<(u64, u64)> = stall_alerts(log, quiet)
            .into_iter()
            .filter(|(card, seq)| match cursor.stalls.get(&card.to_string()) {
                Some(said) if said.seq == *seq => {
                    said.times < STALL_REMINDERS && now >= said.at + quiet.max(60)
                }
                _ => true,
            })
            .collect();
        let stalled = stalls.len();
        let pane = match (&pane, &label) {
            (Some(pane), _) => Some(pane.clone()),
            (None, Some(label)) => super::outer::pane_of_label(label, session),
            (None, None) => None,
        };
        if let (Some(pane), true) = (pane, waiting > 0 || stalled > 0) {
            let timeout = (idle_wait * 1000).to_string();
            let idle = herdr(
                &[
                    "agent",
                    "wait",
                    &pane,
                    "--until",
                    "idle",
                    "--until",
                    "done",
                    "--timeout",
                    &timeout,
                ],
                session,
            )
            .is_some_and(|out| out.status.success());
            let taken = idle
                && herdr(
                    &[
                        "agent",
                        "prompt",
                        &pane,
                        &sentence(log),
                        "--wait",
                        "--until",
                        "working",
                        "--until",
                        "blocked",
                        "--timeout",
                        "10000",
                    ],
                    session,
                )
                .is_some_and(|out| out.status.success());
            if taken {
                // Everything up to the log's head has been pointed to.
                cursor.seen = head;
                for (card, seq) in &stalls {
                    let said = cursor.stalls.entry(card.to_string()).or_default();
                    if said.seq != *seq {
                        said.times = 0;
                    }
                    said.seq = *seq;
                    said.at = now;
                    said.times += 1;
                }
                println!(
                    "已提醒外环窗格 {pane}:{waiting} 条新事件{}",
                    if stalled > 0 {
                        format!(",{stalled} 条静默提醒")
                    } else {
                        String::new()
                    }
                );
                write_cursor(&cursor);
            } else {
                println!("外环窗格 {pane} 现在不空闲或没有接住提醒,下一轮再试");
            }
        }
        if once {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1)));
    }
}
