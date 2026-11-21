//! `octoscode olp card open|ack|accept|return|void|snooze` — the writes of
//! a card's life, as events in the log. Each is decided and appended under
//! the log's lock: a card number is the next free one, a key is written
//! once, and an event about a card that does not exist or was voided is
//! refused there and then.
//!
//! Exit codes: 0 written (or already there, same key); 1 refused by a rule;
//! 2 usage; 4 the log could not be read or written; 6 the outer-duty lease
//! is no longer valid (the outer loop's writes only).

use std::io::Read;

use serde_json::json;

use super::log::{AppendError, Draft, Event, Log};

struct Args {
    log: String,
    by: Option<String>,
    key: Option<String>,
    title: Option<String>,
    unit: Option<String>,
    implementer: Option<String>,
    review: Vec<String>,
    until: Option<String>,
    json: bool,
    rest: Vec<String>,
}

fn usage(action: &str) -> &'static str {
    match action {
        "open" => {
            "用法: octoscode olp card open --log <目录> --title <标题> [--unit U] [--impl A] [--review A,B] [--key K] [--by 署名] [--json]  (正文从 stdin)"
        }
        "ack" => {
            "用法: octoscode olp card ack --log <目录> <N> <done|wontdo|blocked> <说明…> [--by 署名] [--key K]"
        }
        "snooze" => {
            "用法: octoscode olp card snooze --log <目录> <N> --until YYYY-MM-DD <等什么…> [--by 署名]"
        }
        "void" => "用法: octoscode olp card void --log <目录> <N> [说明…] [--by 署名]",
        _ => {
            "用法: octoscode olp card accept|return --log <目录> <N> <说明…> [--by 署名] [--key K]"
        }
    }
}

fn parse(argv: &[String]) -> Result<Option<Args>, String> {
    let mut args = Args {
        log: String::new(),
        by: None,
        key: None,
        title: None,
        unit: None,
        implementer: None,
        review: Vec::new(),
        until: None,
        json: false,
        rest: Vec::new(),
    };
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
            "--log" => args.log = value()?,
            "--by" => args.by = Some(value()?),
            "--key" => args.key = Some(value()?),
            "--title" => args.title = Some(value()?),
            "--unit" => args.unit = Some(value()?),
            "--impl" => args.implementer = Some(value()?),
            "--until" => args.until = Some(value()?),
            "--review" => {
                args.review = value()?
                    .split(',')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect();
            }
            other if other.starts_with("--") => return Err(format!("不认识的参数: {other}")),
            other => args.rest.push(other.to_string()),
        }
    }
    if args.log.is_empty() {
        return Err("缺少 --log <日志目录>".to_string());
    }
    Ok(Some(args))
}

/// Lowercase letter first, then letters, digits, hyphens; 32 at most —
/// the rule unit ids and agent kinds already follow.
fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn single_line(s: &str) -> bool {
    !s.trim().is_empty() && !s.contains(['\n', '\r'])
}

/// A signature goes inside `外环(…)` / `内环(…)`: one line, no brackets.
fn signature(role: &str, by: Option<&str>, default: &str) -> Result<String, String> {
    let by = by.unwrap_or(default).trim();
    if !single_line(by) || by.contains(['(', ')', '(', ')']) {
        return Err("--by 必须是非空的单行,不含括号".to_string());
    }
    Ok(format!("{role}({by})"))
}

enum Refusal {
    /// Exit 1: a rule said no.
    Rule(String),
}

/// The card `n` as the log has it: `Err` when there is no such card or it
/// was voided — nothing more is written about either.
fn open_card(events: &[Event], n: u64) -> Result<(), Refusal> {
    if !events
        .iter()
        .any(|e| e.kind == "dispatched" && e.card == Some(n))
    {
        return Err(Refusal::Rule(format!("没有 #{n} 这张卡")));
    }
    if events
        .iter()
        .any(|e| e.kind == "voided" && e.card == Some(n))
    {
        return Err(Refusal::Rule(format!("#{n} 已被改判作废,不再接受写入")));
    }
    Ok(())
}

fn finish(prog: &str, result: Result<Vec<Event>, AppendError<Refusal>>) -> Result<Vec<Event>, i32> {
    result.map_err(|e| match e {
        AppendError::Refused(Refusal::Rule(why)) => {
            eprintln!("{prog}: {why},未写入");
            1
        }
        AppendError::Log(e) => {
            eprintln!("{prog}: {e}");
            4
        }
    })
}

/// `action` is the word after `olp card`; `argv` what follows it.
pub fn run(action: &str, argv: &[String]) -> i32 {
    let prog = format!("olp card {action}");
    let args = match parse(argv) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{}", usage(action));
            return 0;
        }
        Err(why) => {
            eprintln!("{prog}: {why}\n{}", usage(action));
            return 2;
        }
    };
    let fail = |why: &str| {
        eprintln!("{prog}: {why}\n{}", usage(action));
        2
    };
    let log = Log::at(&args.log);
    if action == "open" {
        return open(&prog, &log, args);
    }
    let Some(n) = args
        .rest
        .first()
        .and_then(|w| w.parse::<u64>().ok())
        .filter(|n| *n > 0)
    else {
        return fail("缺少卡号");
    };
    let outer = action != "ack";
    let actor = match signature(
        if outer { "外环" } else { "内环" },
        args.by.as_deref(),
        if outer { "hub" } else { "本机" },
    ) {
        Ok(actor) => actor,
        Err(why) => return fail(&why),
    };
    let (kind, words, mut data) = match action {
        "ack" => {
            let Some(result) = args
                .rest
                .get(1)
                .filter(|r| matches!(r.as_str(), "done" | "wontdo" | "blocked"))
            else {
                return fail("结果只能是 done、wontdo 或 blocked");
            };
            ("ack", &args.rest[2..], json!({ "result": result }))
        }
        "accept" => ("accepted", &args.rest[1..], json!({})),
        "return" => ("returned", &args.rest[1..], json!({})),
        "void" => ("voided", &args.rest[1..], json!({})),
        "snooze" => {
            let Some(until) = &args.until else {
                return fail("缺少 --until YYYY-MM-DD");
            };
            if chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d").is_err() || until.len() != 10 {
                return fail("--until 不是合法日期(YYYY-MM-DD)");
            }
            ("snoozed", &args.rest[1..], json!({ "until": until }))
        }
        _ => return fail("没有这个动作"),
    };
    let said = words.join(" ");
    if kind != "voided" && said.trim().is_empty() {
        return fail("缺少说明(done 附证据,wontdo 附理由,blocked、打回、搁置附原因)");
    }
    if said.contains(['\n', '\r']) {
        return fail("说明必须是单行");
    }
    // The inbox reads a snooze's date from the start of its text.
    data["text"] = json!(match (&args.until, kind) {
        (Some(until), "snoozed") => format!(" 至 {until}:{}", said.trim()),
        _ => said.trim().to_string(),
    });
    if outer {
        if let Err(code) = super::duty::gate(&prog) {
            return code;
        }
    }
    let draft = Draft {
        kind: kind.to_string(),
        card: Some(n),
        actor,
        unit: None,
        key: args.key.clone(),
        data,
    };
    let written = finish(
        &prog,
        log.append_with(|events| {
            if let Some(key) = &draft.key {
                if events.iter().any(|e| e.key.as_ref() == Some(key)) {
                    return Ok(Vec::new()); // said before: nothing to add
                }
            }
            open_card(events, n)?;
            Ok(vec![draft])
        }),
    );
    match written {
        Ok(events) if events.is_empty() => {
            println!("#{n}:同一个键已经写过,没有重复写入");
            0
        }
        Ok(_) => {
            println!("#{n}:已记录 {kind}");
            0
        }
        Err(code) => code,
    }
}

fn open(prog: &str, log: &Log, args: Args) -> i32 {
    let fail = |why: &str| {
        eprintln!("{prog}: {why}\n{}", usage("open"));
        2
    };
    let Some(title) = args
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| single_line(t))
    else {
        return fail("--title 必须是非空的单行");
    };
    for name in args
        .unit
        .iter()
        .chain(&args.implementer)
        .chain(&args.review)
    {
        if !is_name(name) {
            return fail(&format!(
                "名字不合法: {name}(小写字母开头,小写字母、数字、连字符,最长 32)"
            ));
        }
    }
    let mut unique = args.review.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != args.review.len() {
        return fail("--review 里有重复的名字");
    }
    if !args.rest.is_empty() {
        return fail(&format!("多余的参数: {}", args.rest.join(" ")));
    }
    let actor = match signature("外环", args.by.as_deref(), "hub") {
        Ok(actor) => actor,
        Err(why) => return fail(&why),
    };
    let mut body = String::new();
    if std::io::stdin().read_to_string(&mut body).is_err() || body.trim().is_empty() {
        return fail("正文为空(从 stdin 喂)");
    }
    let body = format!("{}\n", body.trim_end_matches(['\n', '\r']));
    if let Err(code) = super::duty::gate(prog) {
        return code;
    }
    let data = json!({
        "title": title,
        "body": body,
        "impl": args.implementer,
        "review": args.review,
    });
    let mut reused: Option<u64> = None;
    let result = log.append_with(|events| {
        if let Some(key) = &args.key {
            if let Some(first) = events.iter().find(|e| e.key.as_ref() == Some(key)) {
                // The same key is the same card, or it is a mistake.
                if first.kind != "dispatched" || first.unit != args.unit || first.data != data {
                    return Err(Refusal::Rule(format!(
                        "键 {key} 已绑定 #{},与本次立卡的内容不符",
                        first.card.unwrap_or(0)
                    )));
                }
                reused = first.card;
                return Ok(Vec::new());
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
            actor,
            unit: args.unit.clone(),
            key: args.key.clone(),
            data: data.clone(),
        }])
    });
    let written = match finish(prog, result) {
        Ok(written) => written,
        Err(code) => return code,
    };
    let created = !written.is_empty();
    let Some(number) = written.first().and_then(|e| e.card).or(reused) else {
        eprintln!("{prog}: 没有得到卡号");
        return 4;
    };
    if args.json {
        println!(
            "{}",
            json!({ "number": number, "created": created, "unit": args.unit, "key": args.key })
        );
    } else {
        if created {
            println!("已立卡 #{number}");
        } else {
            println!("#{number} 已在日志里(同一个键),没有重复立卡");
        }
        println!("{number}");
    }
    0
}
