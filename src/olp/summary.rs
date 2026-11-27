//! `octoscode olp board summary --board <板或日志目录>` — the project's
//! state in one piece, for the outer loop to hand to the project's
//! panorama: every card and where it stands, the units, the local pull
//! requests, alerts on open cards and questions still waiting.
//!
//! Read-only. A source that cannot be read is reported as
//! `{"unavailable": "<why>"}` in its place; the rest is still printed.
//! Exit codes: 0; 2 usage (no such board).

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use super::board;
use super::inbox::{Partition, items};
use super::unit;

const USAGE: &str = "用法: octoscode olp board summary --board <黑板文件或日志目录> [--repo <hub 仓库>] [--name <项目名>] [--json]

OLP 进展汇总(交给项目全景台):卡、各区计数、单元、本地 PR(给了 --repo 时)、未了结卡上的
告警、待回答的提问。只读。读不到的一项记为 {\"unavailable\": 原因},其余照常输出。
退出码:0 成功;2 用法错误(黑板不存在等)";

fn zh(partition: &str) -> &str {
    match partition {
        "inbox" => "收件箱",
        "working" => "进行中",
        "snoozed" => "搁置",
        "settled" => "已了结",
        other => other,
    }
}

/// `单元(U)·负哨(#N): …` lines on a Markdown board, for cards still open.
fn board_alerts(text: &str, open: &[u64]) -> Vec<Value> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix('>').unwrap_or(line).trim_start();
        let Some(rest) = line.strip_prefix("单元(") else {
            continue;
        };
        let Some((unit, rest)) = rest.split_once(")·负哨(") else {
            continue;
        };
        let Some((card, said)) = rest.split_once("):") else {
            continue;
        };
        let Ok(card) = card.trim_start_matches('#').parse::<u64>() else {
            continue;
        };
        if open.contains(&card) {
            let alert: String = said.trim_start().chars().take(160).collect();
            out.push(json!({"unit": unit, "card": card, "alert": alert}));
        }
    }
    let skip = out.len().saturating_sub(20);
    out.split_off(skip)
}

/// On the event log: what units reported as not delivered and what their
/// runtime guards saw, for open cards.
fn log_alerts(events: &[super::log::Event], open: &[u64]) -> Vec<Value> {
    let mut out: Vec<Value> = events
        .iter()
        .filter_map(|e| {
            let undelivered = e.kind == "progress"
                && e.data.get("node").and_then(Value::as_str) == Some("undelivered");
            if !undelivered && e.kind != "alert" {
                return None;
            }
            let card = e.card.filter(|c| open.contains(c))?;
            let said: String = e
                .data
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect();
            let alert = if undelivered {
                format!("undelivered {said}")
            } else {
                said
            };
            Some(json!({"unit": e.unit, "card": card, "alert": alert}))
        })
        .collect();
    let skip = out.len().saturating_sub(20);
    out.split_off(skip)
}

/// The local pull requests, from `olp repo status --json`.
fn local_prs(repo: &str, board: &str) -> Value {
    let unavailable = |why: String| json!({ "unavailable": why });
    let Ok(exe) = std::env::current_exe() else {
        return unavailable("找不到 octoscode 自身".to_string());
    };
    let out = Command::new(exe)
        .args([
            "olp", "repo", "status", "--repo", repo, "--board", board, "--json",
        ])
        .stdin(Stdio::null())
        .output();
    match out {
        Err(e) => unavailable(e.to_string()),
        Ok(out) if !out.status.success() => {
            let said = String::from_utf8_lossy(&out.stderr);
            let last: String = said
                .trim()
                .lines()
                .last()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            unavailable(if last.is_empty() {
                format!("退出码 {}", out.status.code().unwrap_or(-1))
            } else {
                last
            })
        }
        Ok(out) => serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| unavailable("输出不是 JSON".to_string())),
    }
}

fn render(s: &Value) {
    let word = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    println!("项目 {} · {}", word(s, "project"), word(s, "generated_at"));
    if let Some(counts) = s.get("counts").and_then(Value::as_object) {
        let parts: Vec<String> = ["inbox", "working", "snoozed", "settled"]
            .iter()
            .filter_map(|k| Some(format!("{} {}", zh(k), counts.get(*k)?)))
            .collect();
        println!("卡:{}", parts.join("  "));
    }
    if let Some(why) = s["cards"].get("unavailable").and_then(Value::as_str) {
        println!("卡:读不到({why})");
    }
    for c in s["cards"].as_array().into_iter().flatten() {
        if c["partition"] == "settled" {
            continue;
        }
        let review: Vec<&str> = c["review"]
            .as_array()
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let roles = if c["impl"].is_string() || !review.is_empty() {
            format!(
                " 实现 {} / 复验 {}",
                c["impl"].as_str().unwrap_or("-"),
                if review.is_empty() {
                    "-".to_string()
                } else {
                    review.join("、")
                }
            )
        } else {
            String::new()
        };
        let unit = c["unit"]
            .as_str()
            .map(|u| format!(" [{u}]"))
            .unwrap_or_default();
        println!(
            "  #{}{unit}{roles} {} — {}:{}",
            c["number"],
            word(c, "title"),
            zh(&word(c, "partition")),
            word(c, "reason")
        );
    }
    let units = s["units"].as_array().map(Vec::as_slice).unwrap_or_default();
    if !units.is_empty() {
        println!("单元:");
    }
    for u in units {
        let reasons: Vec<&str> = u["reasons"]
            .as_array()
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        println!(
            "  {} {} 心跳 {} 内环 {} 权限 {}{}",
            word(u, "unit"),
            unit::state_zh(&word(u, "state")),
            u["heartbeat_age_seconds"]
                .as_i64()
                .map(|a| format!("{a}s"))
                .unwrap_or_else(|| "-".to_string()),
            unit::kinds_text(u),
            u["permission"].as_str().unwrap_or("None"),
            if reasons.is_empty() {
                String::new()
            } else {
                format!(" — {}", reasons.join(";"))
            }
        );
    }
    match s.get("local_prs") {
        Some(prs) if prs.get("unavailable").is_some() => {
            println!("本地 PR:读不到({})", word(prs, "unavailable"));
        }
        Some(prs) => {
            let rows = prs["items"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            if !rows.is_empty() {
                println!("本地 PR:");
            }
            for p in rows {
                let label = p["branch"].as_str().map(str::to_string).unwrap_or_else(|| {
                    format!(
                        "task/{}",
                        p.get("number")
                            .map(Value::to_string)
                            .unwrap_or_else(|| "?".to_string())
                    )
                });
                println!("  {label} {}", p["state"].as_str().unwrap_or("?"));
            }
        }
        None => {}
    }
    for q in s["questions"].as_array().into_iter().flatten() {
        println!(
            "提问 [{}] {}",
            q["unit"].as_str().unwrap_or("None"),
            word(q, "question")
        );
    }
    for a in s["alerts"].as_array().into_iter().flatten() {
        println!(
            "负哨 #{} [{}] {}",
            a["card"],
            word(a, "unit"),
            word(a, "alert")
        );
    }
}

/// `argv` is what follows `olp board summary`.
pub fn run(argv: &[String]) -> i32 {
    let (mut source, mut repo, mut name, mut as_json) = (None, None, None, false);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let slot = match word.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--json" => {
                as_json = true;
                continue;
            }
            "--board" | "--log" => &mut source,
            "--repo" => &mut repo,
            "--name" => &mut name,
            other => {
                eprintln!("olp board summary: 不认识的参数: {other}\n{USAGE}");
                return 2;
            }
        };
        match words.next() {
            Some(value) => *slot = Some(value.clone()),
            None => {
                eprintln!("olp board summary: {word} 需要一个值\n{USAGE}");
                return 2;
            }
        }
    }
    let Some(source) = source else {
        eprintln!("olp board summary: 缺少 --board\n{USAGE}");
        return 2;
    };
    let path = std::path::absolute(&source).unwrap_or_else(|_| Path::new(&source).to_path_buf());
    if !path.is_file() && !path.is_dir() {
        eprintln!("olp-summary: 黑板不存在: {source}");
        return 2;
    }
    let shown = path.to_string_lossy().into_owned();
    // <project>/.octos/<board>: the project is two levels up.
    let project = name.unwrap_or_else(|| {
        path.parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| shown.clone())
    });
    let mut summary = json!({
        "project": project,
        "board": shown,
        "generated_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    });
    let today = chrono::Local::now().date_naive();
    // A directory is an event log; a file is a Markdown board.
    let read = if path.is_dir() {
        super::log::Log::at(&path)
            .read()
            .map(|events| {
                let scan = super::cards::scan(&events);
                let questions = super::cards::pending_questions(&events);
                (scan, questions, Ok(events))
            })
            .map_err(|e| e.to_string())
    } else {
        std::fs::read(&path)
            .map(|bytes| {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                (
                    board::scan(&text),
                    board::pending_questions(&text),
                    Err(text),
                )
            })
            .map_err(|e| e.to_string())
    };
    match read {
        Err(why) => {
            summary["cards"] = json!({ "unavailable": why });
            summary["alerts"] = json!([]);
        }
        Ok((mut scan, questions, origin)) => {
            let all = items(&mut scan, today, |_| true);
            let count = |p: Partition| all.iter().filter(|i| i.partition == p).count();
            summary["counts"] = json!({
                "inbox": count(Partition::Inbox),
                "working": count(Partition::Working),
                "snoozed": count(Partition::Snoozed),
                "settled": count(Partition::Settled),
            });
            let open: Vec<u64> = all
                .iter()
                .filter(|i| i.partition != Partition::Settled)
                .map(|i| i.number)
                .collect();
            summary["cards"] = all
                .iter()
                .map(|i| {
                    json!({
                        "number": i.number, "title": i.title, "unit": i.unit,
                        "impl": i.implementer, "review": i.review,
                        "partition": i.partition, "reason": i.reason,
                    })
                })
                .collect();
            summary["questions"] = questions
                .into_iter()
                .map(
                    |q| json!({"id": q.id, "unit": q.unit, "question": q.question, "line": q.line}),
                )
                .collect();
            summary["alerts"] = json!(match &origin {
                Ok(events) => log_alerts(events, &open),
                Err(text) => board_alerts(text, &open),
            });
        }
    }
    summary["units"] = unit::report(unit::DEFAULT_STALE)
        .into_iter()
        .map(|u| {
            json!({
                "unit": u["unit"], "state": u["state"],
                "heartbeat_age_seconds": u["heartbeat_age_seconds"],
                "agents": u["agents"], "permission": u["permission"],
                "reviewers": u["reviewers"], "reasons": u["reasons"],
            })
        })
        .collect();
    if let Some(repo) = repo {
        summary["local_prs"] = local_prs(&repo, &shown);
    }
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).unwrap_or_else(|_| summary.to_string())
        );
    } else {
        render(&summary);
    }
    0
}
