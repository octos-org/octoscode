//! `octoscode olp card dispatch <单元> --log <目录> …` — dispatch a card in
//! one step: is the unit healthy, can it run the reviewers the card names,
//! is it free, then write the card. The checks that read the log and the
//! write happen under the log's lock, so two dispatches cannot both find
//! the unit free.
//!
//! Delivery is the log itself: with `--wake <窗格>` the card names a pane
//! and sits in the unit's inbox, where the unit's watcher finds it
//! (`olp remote watch`). Nothing is typed into a pane from the hub.
//!
//! Exit codes: 0 dispatched (or already there, same key); 1 refused by a
//! rule, nothing written; 2 usage; 4 the log could not be read or written;
//! 6 the outer-duty lease is no longer valid.

use std::io::Read;

use serde_json::json;
use sha2::{Digest, Sha256};

use super::card::is_pane;
use super::inbox::{Partition, items};
use super::log::{AppendError, Draft, Log};
use super::unit;

const USAGE: &str = "用法: octoscode olp card dispatch <单元号> --log <日志目录> --title <标题> [选项]  (正文从 stdin)

  --impl AGENT        单元里由哪种内环实现
  --review A[,B…]     单元里由哪些内环对抗复验
  --avoid 文本        这张卡不许碰的东西(文件、目录、接口);可重复
  --verify-of M       这是 #M 的回测卡:不能派给做 #M 的那台单元
  --wake 窗格         卡送到这台单元的这个窗格(进它的收件箱,由单元值守唤醒)
  --key K             幂等键;不给时由单元号、标题、正文与分工算出
  --by 署名           默认 hub
  --skip-health       不查单元健康,也不核对复验方(登记没部署时)
  --allow-busy        单元还占着别的卡也派
  --allow-reviewers   复验方不在单元心跳里、或没登录也派(operator 点名时)
  --stale-after 秒    心跳多久算失联,默认 180
  --json              以 JSON 输出结果

Markdown 板:把 --log 换成 --board <板>。卡写进板里,--wake 经 herdr 往窗格发一句话;署名用
--signature(默认 外环(hub)),--date 指定标题里的日期;不核对复验方。退出码另有 5:已写板但
唤醒失败,原样重跑即可。
退出码:0 成功;1 被规则拒绝,未写入;2 用法;4 日志读写失败;6 主审权已失效";

#[derive(Default)]
struct Args {
    unit: String,
    log: String,
    title: String,
    by: Option<String>,
    key: Option<String>,
    implementer: Option<String>,
    review: Vec<String>,
    avoid: Vec<String>,
    verify_of: Option<u64>,
    pane: Option<String>,
    stale_after: u64,
    skip_health: bool,
    allow_busy: bool,
    allow_reviewers: bool,
    json: bool,
    /// A Markdown board instead of an event log.
    board: Option<String>,
    /// The date in a Markdown card's heading (default: today).
    date: Option<String>,
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut args = Args {
        stale_after: unit::DEFAULT_STALE,
        ..Args::default()
    };
    let mut rest = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let mut value = || {
            words
                .next()
                .cloned()
                .ok_or_else(|| format!("{word} 需要一个值"))
        };
        match word.as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => args.json = true,
            "--skip-health" => args.skip_health = true,
            "--allow-busy" => args.allow_busy = true,
            "--allow-reviewers" => args.allow_reviewers = true,
            "--log" => args.log = value()?,
            "--board" => args.board = Some(value()?),
            "--date" => args.date = Some(value()?),
            "--title" => args.title = value()?.trim().to_string(),
            "--by" | "--signature" => args.by = Some(value()?),
            "--key" => args.key = Some(value()?),
            "--impl" => args.implementer = Some(value()?.trim().to_string()),
            "--wake" | "--pane" => args.pane = Some(value()?),
            "--avoid" => args.avoid.push(value()?.trim().to_string()),
            "--review" => {
                args.review = value()?
                    .split(',')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect();
            }
            "--verify-of" => {
                args.verify_of = Some(
                    value()?
                        .trim_start_matches('#')
                        .parse::<u64>()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or("--verify-of 要一个卡号")?,
                );
            }
            "--stale-after" => {
                args.stale_after = value()?
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or("--stale-after 必须是正整数")?;
            }
            other if other.starts_with("--") => return Err(format!("不认识的参数: {other}")),
            other => rest.push(other.to_string()),
        }
    }
    match rest.as_slice() {
        [unit] => args.unit = unit.clone(),
        [] => return Err("缺少单元号".to_string()),
        more => return Err(format!("多余的参数: {}", more[1..].join(" "))),
    }
    match (args.log.is_empty(), &args.board) {
        (true, None) => return Err("缺少 --log <日志目录> 或 --board <黑板文件>".to_string()),
        (false, Some(_)) => return Err("--log 与 --board 只能给一个".to_string()),
        _ => {}
    }
    let single = |s: &str| !s.trim().is_empty() && !s.contains(['\n', '\r']);
    if !single(&args.title) {
        return Err("--title 必须是非空的单行".to_string());
    }
    for name in std::iter::once(&args.unit)
        .chain(&args.implementer)
        .chain(&args.review)
    {
        if !unit::is_unit(name) {
            return Err(format!(
                "名字不合法: {name}(小写字母开头,小写字母、数字、连字符,最长 32)"
            ));
        }
    }
    let mut unique = args.review.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != args.review.len() {
        return Err("--review 里有重复的内环".to_string());
    }
    if args.implementer.is_some() && args.review.is_empty() {
        return Err(
            "指定了 --impl 就要用 --review 指定至少一个复验方:单元复验通过才交回".to_string(),
        );
    }
    if let Some(bad) = args.avoid.iter().find(|a| !single(a) || a.len() > 400) {
        return Err(format!("--avoid 必须是非空的单行,最长 400 字节: {bad}"));
    }
    if let Some(pane) = args.pane.as_deref().filter(|p| !is_pane(p)) {
        return Err(format!(
            "--wake 不是合法的窗格标识: {pane}(字母或数字开头,字母、数字、_ . : -,最长 128)"
        ));
    }
    if let Some(key) = &args.key {
        let ok = !key.is_empty()
            && key.len() <= 200
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._:/@+=-".contains(c));
        if !ok {
            return Err("--key 只允许 [A-Za-z0-9._:/@+=-],最长 200".to_string());
        }
    }
    if args.board.is_some() {
        // On a Markdown board the signature is the whole `外环(…)` text.
        let signature = args.by.as_deref().unwrap_or("外环(hub)").trim();
        if !single(signature) {
            return Err("--signature 必须是非空的单行".to_string());
        }
        args.by = Some(signature.to_string());
        return Ok(Some(args));
    }
    let by = args.by.as_deref().unwrap_or("hub").trim();
    if !single(by) || by.contains(['(', ')', '(', ')']) {
        return Err("--by 必须是非空的单行,不含括号".to_string());
    }
    args.by = Some(by.to_string());
    Ok(Some(args))
}

/// Why this unit cannot run a reviewer the card names, from its last
/// heartbeat: the reviewer CLIs it reported, and the inner loops herdr had.
fn missing_reviewers(beat: &serde_json::Value, review: &[String]) -> Vec<String> {
    let reported = unit::reviewers_of(beat);
    let running: Vec<&str> = beat
        .get("agents")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| a.get("kind").and_then(|k| k.as_str()))
                .collect()
        })
        .unwrap_or_default();
    review
        .iter()
        .filter_map(|name| {
            match reported.iter().find(|(kind, _, _)| kind == name) {
                Some((_, false, _)) => Some(format!("{name} 未安装")),
                Some((_, true, Some(false))) => Some(format!("{name} 未登录")),
                Some(_) => None,
                // Not a CLI the heartbeat probes: fine when one is running.
                None if running.contains(&name.as_str()) => None,
                None => Some(format!("{name} 不在心跳里")),
            }
        })
        .collect()
}

/// Wake the unit's pane for card `number`: one sentence that says where the
/// card is. `Err` is why herdr did not take it.
fn wake(args: &Args, pane: &str, number: u64) -> Result<(), String> {
    // The unit reads a mirror that was last synced before this card was
    // written, so the wake names the sync as the first step, as a command
    // that runs from any directory.
    let sync = "octoscode olp remote mirror(不可用时:python3 \"$(cat ~/.octos/outer/octoscode-home)/scripts/olp-board-remote.py\" mirror)";
    let message = if args.implementer.is_some() || !args.review.is_empty() {
        // The pane woken for a card with roles is the unit host, who
        // organises the work rather than doing it.
        format!(
            "黑板新条目 #{number} 派给本单元,指定实现方 {}、复验方 {}。你是单元主持,不要自己实现:按 $(cat ~/.octos/outer/octoscode-home)/skills/octoloop/AGENTS.md 的「模式 unit」组织实现与对抗复验,全部复验通过才推送并向 hub 回执。第一步同步镜像:{sync}",
            args.implementer.as_deref().unwrap_or("(由你定)"),
            if args.review.is_empty() {
                "(由你定)".to_string()
            } else {
                args.review.join("、")
            }
        )
    } else {
        format!("黑板新条目 #{number} 派给本单元。先同步本机的黑板镜像:{sync},再读 #{number} 执行")
    };
    let herdr = std::env::var("OLP_HERDR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "herdr".to_string());
    let mut cmd = std::process::Command::new(herdr);
    // A unit registered with this machine's hostname has its panes in the
    // local herdr, not on a saved herdr machine.
    if unit::is_local(&args.unit) != Some(true) {
        cmd.args(["--machine", &args.unit]);
    }
    match cmd.args(["agent", "prompt", pane, &message]).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if said.is_empty() {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            } else {
                said
            })
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Dispatch on a Markdown board: the same checks, the card appended under
/// the board's lock, and the unit's pane woken through herdr.
///
/// Exit codes here: 0; 1 refused, nothing written; 2 usage; 4 the board
/// could not be written; 5 written but the wake failed (run it again: the
/// key is on the board, so only the wake is repeated); 6 lease invalid.
fn on_board(args: &Args, raw_body: &str) -> i32 {
    let prog = "olp-dispatch";
    let fail = |code: i32, why: String| {
        eprintln!("{prog}: {why}");
        code
    };
    let target = &args.unit;
    let board = std::path::Path::new(args.board.as_deref().unwrap_or_default());
    if !board.is_file() {
        return fail(2, format!("黑板不存在: {}", board.display()));
    }
    let date = match &args.date {
        Some(text) => match chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => return fail(2, format!("--date 不是合法日期: {text}")),
        },
        None => chrono::Local::now().date_naive(),
    };
    let key = args.key.clone().unwrap_or_else(|| {
        let roles = if args.implementer.is_some() || !args.review.is_empty() {
            format!(
                "\nimpl={}\nreview={}",
                args.implementer.as_deref().unwrap_or(""),
                args.review.join(",")
            )
        } else {
            String::new()
        };
        let said = format!("{target}\n{}\n{raw_body}{roles}", args.title);
        let digest = Sha256::digest(said.as_bytes());
        let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        format!("dispatch:{target}:{hex}")
    });
    let existing = super::append::number_for_key(board, &key);
    let number = match existing {
        Some(number) => number,
        None => {
            if !args.skip_health {
                let why = match unit::health(target, args.stale_after) {
                    None => Some(format!("olp-unit: 单元未登记: {target}")),
                    Some(h) if h.state != "healthy" => Some(format!(
                        "{target} {},不要派卡:{}",
                        unit::state_zh(h.state),
                        h.reasons.join(";")
                    )),
                    Some(_) => None,
                };
                if let Some(why) = why {
                    return fail(1, format!("单元 {target} 现在不能接卡:{why}"));
                }
            }
            let text = match std::fs::read(board) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => return fail(2, format!("读不了黑板: {e}")),
            };
            let mut scan = super::board::scan(&text);
            if let Some(original) = args.verify_of {
                match scan.entries.get(&original) {
                    None => {
                        return fail(1, format!("板上没有 #{original},无法派它的回测卡"));
                    }
                    Some(entry) if entry.unit.as_deref() == Some(target) => {
                        return fail(
                            1,
                            format!("#{original} 就是 {target} 做的,回测卡要派给另一台单元"),
                        );
                    }
                    Some(_) => {}
                }
            }
            if !args.allow_busy {
                let held: Vec<String> =
                    items(&mut scan, date, |e| e.unit.as_deref() == Some(target))
                        .into_iter()
                        .filter(|i| i.partition != Partition::Settled)
                        .map(|i| format!("#{}", i.number))
                        .collect();
                if !held.is_empty() {
                    return fail(
                        1,
                        format!(
                            "单元 {target} 正忙:还占着 {}。等它了结,或加 --allow-busy",
                            held.join("、")
                        ),
                    );
                }
            }
            let mut entry = format!(
                "### {{N}}. {}({date},{})\nunit: {target}\n",
                args.title,
                args.by.as_deref().unwrap_or("外环(hub)")
            );
            if let Some(original) = args.verify_of {
                entry.push_str(&format!("verify-of: #{original}\n"));
            }
            if let Some(implementer) = &args.implementer {
                entry.push_str(&format!("impl: {implementer}\n"));
            }
            if !args.review.is_empty() {
                entry.push_str(&format!("review: {}\n", args.review.join(", ")));
            }
            for avoid in &args.avoid {
                entry.push_str(&format!("avoid: {avoid}\n"));
            }
            entry.push_str(&format!("\n{}\n", raw_body.trim_end_matches('\n')));
            match super::append::append_entry(board, Some(&key), entry.into_bytes()) {
                Ok(super::append::Written::Numbered(n))
                | Ok(super::append::Written::Already(Some(n))) => n,
                Ok(_) => return fail(4, "写板失败: 没有得到编号".to_string()),
                Err((6, _)) => return 6,
                Err((_, why)) => return fail(4, format!("写板失败: {why}")),
            }
        }
    };
    let created = existing.is_none();
    let woken = args.pane.as_deref().map(|pane| wake(args, pane, number));
    if args.json {
        println!(
            "{}",
            json!({
                "number": number,
                "unit": target,
                "created": created,
                "key": key,
                "woken": woken.as_ref().map(Result::is_ok),
                "watch_token": format!("本地复验通过(#{number})"),
            })
        );
    } else {
        if created {
            println!("已派出 #{number} → {target}");
        } else {
            println!("#{number} 已在板上(同一个键),没有重复派卡");
        }
        if let (Some(Ok(())), Some(pane)) = (&woken, &args.pane) {
            println!("已唤醒 {target} 的窗格 {pane}");
        }
        println!("挂哨标记:本地复验通过(#{number})");
        println!("{number}");
    }
    if let Some(Err(why)) = woken {
        eprintln!(
            "{prog}: #{number} 已写板,但唤醒 {target} 失败:{why}。原样重跑本命令即可补做唤醒"
        );
        return 5;
    }
    0
}

/// `argv` is what follows `olp card dispatch`.
pub fn run(argv: &[String]) -> i32 {
    let prog = "olp card dispatch";
    let args = match parse(argv) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(why) => {
            eprintln!("{prog}: {why}\n{USAGE}");
            return 2;
        }
    };
    let mut body = String::new();
    if std::io::stdin().read_to_string(&mut body).is_err() || body.trim().is_empty() {
        eprintln!("{prog}: 正文为空(从 stdin 喂)");
        return 2;
    }
    if args.board.is_some() {
        return on_board(&args, &body);
    }
    let body = format!("{}\n", body.trim_end_matches(['\n', '\r']));
    let target = &args.unit;
    let key = args.key.clone().unwrap_or_else(|| {
        // What is dispatched: the unit, the card, and who does what. The
        // same card with other reviewers is a different dispatch.
        let mut said = format!("{target}\n{}\n{body}", args.title);
        if args.implementer.is_some() || !args.review.is_empty() {
            said.push_str(&format!(
                "\nimpl={}\nreview={}",
                args.implementer.as_deref().unwrap_or(""),
                args.review.join(",")
            ));
        }
        for avoid in &args.avoid {
            said.push_str(&format!("\navoid={avoid}"));
        }
        let digest = Sha256::digest(said.as_bytes());
        let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        format!("dispatch:{target}:{hex}")
    });
    if let Err(code) = super::duty::gate(prog) {
        return code;
    }
    let log = Log::at(&args.log);
    let mut data = json!({
        "title": args.title,
        "body": body,
        "impl": args.implementer,
        "review": args.review,
        "pane": args.pane,
    });
    if !args.avoid.is_empty() {
        data["avoid"] = json!(args.avoid);
    }
    if let Some(original) = args.verify_of {
        data["verify_of"] = json!(original);
    }
    let today = chrono::Local::now().date_naive();
    let mut reused: Option<u64> = None;
    let result = log.append_with(|events| {
        if let Some(first) = events.iter().find(|e| e.key.as_ref() == Some(&key)) {
            // The same key is the same card, or it is a mistake.
            if first.kind != "dispatched" || first.unit.as_ref() != Some(target) {
                return Err(format!(
                    "键 {key} 已绑定 #{},与本次派卡不符",
                    first.card.unwrap_or(0)
                ));
            }
            // A voided card does not come back: saying "already there"
            // would leave the outer loop waiting on a card nobody will do.
            let voided = events
                .iter()
                .any(|e| e.kind == "voided" && e.card == first.card);
            if voided {
                return Err(format!(
                    "键 {key} 的 #{} 已作废,不会重新生效;要重派请改标题或正文,或给一个新的 --key",
                    first.card.unwrap_or(0)
                ));
            }
            reused = first.card;
            return Ok(Vec::new());
        }
        // A card that is only in the archive keeps its number and its key.
        let floor = log.floor().map_err(|e| format!("读不了日志的基线: {e}"))?;
        if let Some(archived) = floor.card_of_key(&key) {
            reused = Some(archived);
            return Ok(Vec::new());
        }
        if !args.skip_health {
            let Some(health) = unit::health(target, args.stale_after) else {
                return Err(format!("单元 {target} 现在不能接卡:单元未登记"));
            };
            if health.state != "healthy" {
                return Err(format!(
                    "单元 {target} 现在不能接卡:{},{}",
                    unit::state_zh(health.state),
                    health.reasons.join(";")
                ));
            }
            if !args.allow_reviewers {
                let missing = health
                    .beat
                    .as_ref()
                    .map(|beat| missing_reviewers(beat, &args.review))
                    .unwrap_or_default();
                if !missing.is_empty() {
                    return Err(format!(
                        "单元 {target} 跑不了这张卡的复验方:{}。换复验方,或加 --allow-reviewers",
                        missing.join("、")
                    ));
                }
            }
        }
        let mut scan = super::cards::scan(events);
        if let Some(original) = args.verify_of {
            let done_by = match scan.entries.get(&original) {
                Some(entry) => entry.unit.as_deref(),
                None => match floor.gone.get(&original) {
                    Some(gone) => gone.unit.as_deref(),
                    None => return Err(format!("没有 #{original},无法派它的回测卡")),
                },
            };
            if done_by == Some(target) {
                return Err(format!(
                    "#{original} 就是 {target} 做的,回测卡要派给另一台单元"
                ));
            }
        }
        if !args.allow_busy {
            let held: Vec<String> = items(&mut scan, today, |e| e.unit.as_deref() == Some(target))
                .into_iter()
                .filter(|i| i.partition != Partition::Settled)
                .map(|i| format!("#{}", i.number))
                .collect();
            if !held.is_empty() {
                return Err(format!(
                    "单元 {target} 正忙:还占着 {}。等它了结,或加 --allow-busy",
                    held.join("、")
                ));
            }
        }
        let next = events
            .iter()
            .filter(|e| e.kind == "dispatched")
            .filter_map(|e| e.card)
            .max()
            .unwrap_or(0)
            .max(floor.max_card)
            + 1;
        Ok(vec![Draft {
            kind: "dispatched".to_string(),
            card: Some(next),
            actor: format!("外环({})", args.by.as_deref().unwrap_or("hub")),
            unit: Some(target.clone()),
            key: Some(key.clone()),
            data: data.clone(),
        }])
    });
    let written = match result {
        Ok(written) => written,
        Err(AppendError::Refused(why)) => {
            eprintln!("{prog}: {why},未写入");
            return 1;
        }
        Err(AppendError::Log(e)) => {
            eprintln!("{prog}: {e}");
            return 4;
        }
    };
    let created = !written.is_empty();
    let Some(number) = written.first().and_then(|e| e.card).or(reused) else {
        eprintln!("{prog}: 没有得到卡号");
        return 4;
    };
    if args.json {
        println!(
            "{}",
            json!({
                "number": number,
                "unit": target,
                "created": created,
                "key": key,
                "pane": args.pane,
            })
        );
        return 0;
    }
    if created {
        println!("已派出 #{number} → {target}");
        if let Some(pane) = &args.pane {
            println!("已进 {target} 的收件箱(窗格 {pane}),由单元值守唤醒");
        }
    } else {
        println!("#{number} 已在日志里(同一个键),没有重复派卡");
    }
    println!("{number}");
    0
}
