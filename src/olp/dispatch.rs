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

Markdown 板:把 --log 换成 --board <板>,由 olp-dispatch.py 处理(它自己的 --help 另有说明)。
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
    if args.log.is_empty() {
        return Err("缺少 --log <日志目录>".to_string());
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
            reused = first.card;
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
            match scan.entries.get(&original) {
                None => return Err(format!("没有 #{original},无法派它的回测卡")),
                Some(entry) if entry.unit.as_deref() == Some(target) => {
                    return Err(format!(
                        "#{original} 就是 {target} 做的,回测卡要派给另一台单元"
                    ));
                }
                Some(_) => {}
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
