//! `octoscode olp repo status|promote|push` — local pull requests: the
//! state of every task branch in the hub repository, the gate before
//! GitHub, and the unit's push.
//!
//! A unit's push of `task/<N>` to the hub is a local pull request. Its
//! state is worked out, never stored, from the repository's refs and the
//! blackboard (a Markdown board or an event log): for one card, the last of
//! "unit pushed", "outer accepted", "outer returned" decides.
//!
//! | state     | when                                            | waits on |
//! |-----------|-------------------------------------------------|----------|
//! | pending   | last is a push record, or nothing is recorded   | outer    |
//! | returned  | last is a return                                | unit     |
//! | accepted  | last is an acceptance, not yet on the remote    | outer    |
//! | submitted | accepted and the remote's tracking ref is head  | a person |
//!
//! `promote <N>` pushes the branch to the GitHub remote only when the card
//! is accepted and the head is the commit the last verified push recorded.
//! `push <N> --verified-by <signer>` is the unit's side: it pushes the
//! local `task/<N>` head with the verification record the hub asks for.
//!
//! Exit codes: 0; 1 `promote` refused or the push failed; 2 usage, or no
//! such repository or board; 6 the outer-duty lease is invalid. `push`
//! returns git's own code after its checks.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::board::strip_quote;
use super::log::Log;

struct Fail(i32, String);

#[derive(Clone, PartialEq)]
enum Kind {
    Pushed { unit: String, sha: String },
    Accepted,
    Returned,
}

fn git(repo: &str, args: &[&str]) -> (bool, String, String) {
    match Command::new("git")
        .args(["--git-dir", repo])
        .args(args)
        .output()
    {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(e) => (false, String::new(), e.to_string()),
    }
}

fn digits(s: &str) -> Option<(u64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// `<who>(<name>)·` at the start: the name and the rest.
fn signed<'a>(line: &'a str, who: &str) -> Option<(&'a str, &'a str)> {
    let rest = line.strip_prefix(who)?.strip_prefix('(')?;
    let (name, rest) = rest.split_once(')')?;
    (!name.is_empty()).then_some((name, rest.strip_prefix('·')?))
}

/// The commit in `task/<n> @ <sha>…`.
fn pushed_sha(text: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix("task/")?;
    let (_, rest) = digits(rest)?;
    let rest = rest.trim_start().strip_prefix('@')?.trim_start();
    let sha: String = rest
        .chars()
        .take_while(|c| matches!(c, '0'..='9' | 'a'..='f'))
        .take(64)
        .collect();
    (sha.len() >= 7).then_some(sha)
}

/// What a Markdown board records per card, in line order. Only line-start
/// forms outside code fences count.
fn board_events(text: &str) -> BTreeMap<u64, Vec<Kind>> {
    let mut events: BTreeMap<u64, Vec<Kind>> = BTreeMap::new();
    let mut in_fence = false;
    for line in text.lines() {
        let stripped = line.trim_start();
        if stripped.starts_with("```") || stripped.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let line = strip_quote(line.trim_end_matches('\r'));
        if let Some((unit, rest)) = signed(line, "单元") {
            let pushed = rest
                .strip_prefix("本地复验通过(#")
                .and_then(digits)
                .and_then(|(n, rest)| {
                    let rest = rest.strip_prefix(')')?;
                    let rest = rest.strip_prefix([':', '：']).unwrap_or(rest);
                    Some((n, pushed_sha(rest)?))
                });
            if let Some((n, sha)) = pushed {
                events.entry(n).or_default().push(Kind::Pushed {
                    unit: unit.to_string(),
                    sha,
                });
            }
            continue;
        }
        if let Some((_, rest)) = signed(line, "外环") {
            for (word, kind) in [("采认", Kind::Accepted), ("打回", Kind::Returned)] {
                let card = rest
                    .strip_prefix(word)
                    .and_then(|r| r.strip_prefix("(#"))
                    .and_then(digits)
                    .filter(|(_, rest)| rest.starts_with(')'));
                if let Some((n, _)) = card {
                    events.entry(n).or_default().push(kind);
                    break;
                }
            }
        }
    }
    events
}

fn log_events(dir: &Path) -> Result<BTreeMap<u64, Vec<Kind>>, String> {
    let mut events: BTreeMap<u64, Vec<Kind>> = BTreeMap::new();
    for event in Log::at(dir).read().map_err(|e| e.to_string())? {
        let Some(card) = event.card else { continue };
        let kind = match event.kind.as_str() {
            "accepted" => Kind::Accepted,
            "returned" => Kind::Returned,
            "pushed" => {
                let text = event.data.get("text").and_then(Value::as_str).unwrap_or("");
                match pushed_sha(text) {
                    Some(sha) => Kind::Pushed {
                        unit: event.unit.clone().unwrap_or_default(),
                        sha,
                    },
                    None => continue,
                }
            }
            _ => continue,
        };
        events.entry(card).or_default().push(kind);
    }
    Ok(events)
}

fn read_events(board: &str) -> Result<BTreeMap<u64, Vec<Kind>>, Fail> {
    let path = Path::new(board);
    if path.is_dir() {
        return log_events(path).map_err(|why| Fail(2, format!("读不了事件日志 {board}: {why}")));
    }
    let bytes = std::fs::read(path).map_err(|e| Fail(2, format!("读不了黑板 {board}: {e}")))?;
    Ok(board_events(&String::from_utf8_lossy(&bytes)))
}

fn task_branches(repo: &str) -> Result<BTreeMap<u64, String>, Fail> {
    let (ok, out, err) = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads/task/",
        ],
    );
    if !ok {
        return Err(Fail(2, format!("读不了仓库 {repo}: {err}")));
    }
    Ok(out
        .lines()
        .filter_map(|line| {
            let (name, sha) = line.split_once(' ')?;
            let n = name.strip_prefix("refs/heads/task/")?;
            (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .then(|| Some((n.parse().ok()?, sha.to_string())))?
        })
        .collect())
}

fn remote_head(repo: &str, remote: &str, number: u64) -> Option<String> {
    let reference = format!("refs/remotes/{remote}/task/{number}");
    let (ok, out, _) = git(repo, &["rev-parse", "--verify", "--quiet", &reference]);
    ok.then_some(out)
}

fn short(sha: &str) -> &str {
    &sha[..12.min(sha.len())]
}

struct Status {
    state: &'static str,
    registered: Option<String>,
    record: Value,
    warnings: Vec<String>,
}

fn waiting_on(state: &str) -> &'static str {
    match state {
        "returned" => "unit",
        "submitted" => "human",
        _ => "outer",
    }
}

fn evaluate(number: u64, head: &str, events: &[Kind], tracked: Option<&str>) -> Status {
    let mut warnings = Vec::new();
    let last_push = events.iter().rev().find_map(|e| match e {
        Kind::Pushed { unit, sha } => Some((unit.clone(), sha.clone())),
        _ => None,
    });
    let state = match events.last() {
        None => {
            warnings.push(format!("#{number}:板上没有这个分支的推送登记或结论"));
            "pending"
        }
        Some(Kind::Pushed { .. }) => "pending",
        Some(Kind::Returned) => "returned",
        Some(Kind::Accepted) => {
            if let Some(old) = tracked.filter(|t| *t != head) {
                warnings.push(format!(
                    "#{number}:GitHub 上是旧提交 {},分支头是 {}",
                    short(old),
                    short(head)
                ));
            }
            if tracked == Some(head) {
                "submitted"
            } else {
                "accepted"
            }
        }
    };
    if let Some((_, sha)) = last_push
        .as_ref()
        .filter(|(_, sha)| !head.starts_with(sha.as_str()))
    {
        warnings.push(format!(
            "#{number}:分支头 {} 与最近一次推送登记的 {} 不符",
            short(head),
            short(sha)
        ));
    }
    let pushes = events
        .iter()
        .filter(|e| matches!(e, Kind::Pushed { .. }))
        .count();
    let registered = last_push.as_ref().map(|(_, sha)| sha.clone());
    Status {
        state,
        record: json!({
            "number": number,
            "branch": format!("task/{number}"),
            "head": head,
            // The commit the last verified push recorded (a prefix), or null.
            "registered": registered,
            "unit": last_push.map(|(unit, _)| unit),
            "state": state,
            "waiting_on": waiting_on(state),
            "partition": if state == "returned" { "working" } else { "inbox" },
            "pushes": pushes,
        }),
        registered,
        warnings,
    }
}

struct Args {
    repo: String,
    board: String,
    remote: String,
    json: bool,
    dry_run: bool,
    rest: Vec<String>,
}

fn parse(argv: &[String]) -> Result<Args, Fail> {
    let (mut repo, mut board, mut remote) = (None, None, "github".to_string());
    let (mut as_json, mut dry_run, mut rest) = (false, false, Vec::new());
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| words.next().cloned())
                .ok_or_else(|| Fail(2, format!("{name} 需要一个值")))
        };
        match name {
            "--json" => as_json = true,
            "--dry-run" => dry_run = true,
            "--repo" => repo = Some(value()?),
            "--board" => board = Some(value()?),
            "--remote" => remote = value()?,
            other if other.starts_with("--") => {
                return Err(Fail(2, format!("不认识的参数: {other}")));
            }
            other => rest.push(other.to_string()),
        }
    }
    let (Some(repo), Some(board)) = (repo, board) else {
        return Err(Fail(
            2,
            "需要 --repo <hub.git> 与 --board <黑板>".to_string(),
        ));
    };
    if !Path::new(&repo).is_dir() || !git(&repo, &["rev-parse", "--git-dir"]).0 {
        return Err(Fail(2, format!("不是 git 仓库: {repo}")));
    }
    if !Path::new(&board).is_file() && !Path::new(&board).is_dir() {
        return Err(Fail(2, format!("黑板不存在: {board}")));
    }
    let mut chars = remote.chars();
    let remote_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && remote.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !remote_ok {
        return Err(Fail(2, format!("远程名不合法: {remote}")));
    }
    Ok(Args {
        repo,
        board,
        remote,
        json: as_json,
        dry_run,
        rest,
    })
}

const STATES: [(&str, &str); 4] = [
    ("pending", "待审核"),
    ("accepted", "已采认,待推 GitHub"),
    ("submitted", "已提交 GitHub,待人合并"),
    ("returned", "被退回"),
];

fn status(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv)?;
    if !args.rest.is_empty() || args.dry_run {
        return Err(Fail(2, format!("多余的参数: {}", args.rest.join(" "))));
    }
    let events = read_events(&args.board)?;
    let (mut items, mut warnings) = (Vec::new(), Vec::new());
    for (number, head) in task_branches(&args.repo)? {
        let tracked = remote_head(&args.repo, &args.remote, number);
        let status = evaluate(
            number,
            &head,
            events.get(&number).map(Vec::as_slice).unwrap_or(&[]),
            tracked.as_deref(),
        );
        items.push(status.record);
        warnings.extend(status.warnings);
    }
    let count = |state: &str| items.iter().filter(|i| i["state"] == state).count();
    if args.json {
        let doc = json!({
            "counts": {
                "pending": count("pending"), "returned": count("returned"),
                "accepted": count("accepted"), "submitted": count("submitted"),
            },
            "items": items,
            "warnings": warnings,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
        );
        return Ok(0);
    }
    if items.is_empty() {
        println!("仓库里没有 task/<N> 分支");
    }
    for (state, zh) in STATES {
        let rows: Vec<&Value> = items.iter().filter(|i| i["state"] == state).collect();
        if rows.is_empty() {
            continue;
        }
        let who = match waiting_on(state) {
            "unit" => "单元",
            "human" => "人",
            _ => "大外环",
        };
        println!("== {zh}({})· 等{who}", rows.len());
        for i in rows {
            let unit = i["unit"]
                .as_str()
                .filter(|u| !u.is_empty())
                .map(|u| format!(" [{u}]"))
                .unwrap_or_default();
            println!(
                "{}{unit} @ {}  推送 {} 次",
                i["branch"].as_str().unwrap_or_default(),
                short(i["head"].as_str().unwrap_or_default()),
                i["pushes"]
            );
        }
    }
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok(0)
}

fn promote(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv)?;
    let number = match args.rest.as_slice() {
        [n] => n
            .parse::<u64>()
            .map_err(|_| Fail(2, format!("任务号必须是数字: {n}")))?,
        _ => return Err(Fail(2, "promote 需要一个任务号".to_string())),
    };
    if args.json {
        return Err(Fail(2, "promote 不接受 --json".to_string()));
    }
    super::duty::gate("olp-local-pr").map_err(|code| Fail(code, String::new()))?;
    let Some(head) = task_branches(&args.repo)?.remove(&number) else {
        return Err(Fail(1, format!("仓库里没有分支 task/{number},未推送")));
    };
    if !git(&args.repo, &["remote", "get-url", &args.remote]).0 {
        return Err(Fail(2, format!("仓库里没有名为 {} 的远程", args.remote)));
    }
    let events = read_events(&args.board)?;
    let tracked = remote_head(&args.repo, &args.remote, number);
    let status = evaluate(
        number,
        &head,
        events.get(&number).map(Vec::as_slice).unwrap_or(&[]),
        tracked.as_deref(),
    );
    match status.state {
        "pending" => {
            return Err(Fail(
                1,
                format!(
                    "#{number} 还没有采认:板上这张卡的最新一行不是采认(单元在采认之后又推送过,也要重新审核)。未推送"
                ),
            ));
        }
        "returned" => return Err(Fail(1, format!("#{number} 已被打回,未推送"))),
        "submitted" => {
            println!(
                "task/{number} @ {} 已在 {} 上,无需再推",
                short(&head),
                args.remote
            );
            return Ok(0);
        }
        _ => {}
    }
    // Only the commit a unit pushed with a verification record may go out:
    // the hub's own pushes are unrestricted and leave no record.
    if !status
        .registered
        .as_deref()
        .is_some_and(|sha| head.starts_with(sha))
    {
        return Err(Fail(
            1,
            format!(
                "#{number} 的分支头 {} 不是登记过的复验提交(板上最近一次推送登记:{}),未推送",
                short(&head),
                status.registered.as_deref().unwrap_or("无")
            ),
        ));
    }
    let reference = format!("refs/heads/task/{number}");
    if args.dry_run {
        println!(
            "dry-run:会把 task/{number} @ {} 推到 {}",
            short(&head),
            args.remote
        );
        return Ok(0);
    }
    let (ok, _, err) = git(
        &args.repo,
        &["push", &args.remote, &format!("{reference}:{reference}")],
    );
    if !ok {
        return Err(Fail(1, format!("推送到 {} 失败: {err}", args.remote)));
    }
    // A bare repository does not always move the tracking ref on push;
    // record it so `status` can tell without going online.
    let tracking = format!("refs/remotes/{}/task/{number}", args.remote);
    git(&args.repo, &["update-ref", &tracking, &head]);
    println!(
        "已把 task/{number} @ {} 推到 {};接下来由大外环开 PR",
        short(&head),
        args.remote
    );
    Ok(0)
}

/// The unit's side: push `task/<N>` with its verification record.
fn push(argv: &[String]) -> i32 {
    let die = |why: &str| {
        eprintln!("olp-task-push: {why}");
        2
    };
    let (mut number, mut signer, mut remote, mut force) = (None, None, "hub".to_string(), false);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--verified-by" => match words.next() {
                Some(value) => signer = Some(value.clone()),
                None => return die("--verified-by 需要复验署名"),
            },
            "--remote" => match words.next() {
                Some(value) => remote = value.clone(),
                None => return die("--remote 需要远程名"),
            },
            "--force" => force = true,
            other if other.starts_with('-') => return die(&format!("未知参数: {other}")),
            other if number.is_none() => number = Some(other.to_string()),
            other => return die(&format!("多余的参数: {other}")),
        }
    }
    let Some(number) = number else {
        return die(
            "用法: octoscode olp repo push <N> --verified-by <复验署名> [--remote hub] [--force]",
        );
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return die(&format!("任务号必须是数字: {number}"));
    }
    let Some(signer) = signer.filter(|s| !s.is_empty()) else {
        return die("缺少 --verified-by:本地复验通过后由复验方推送");
    };
    let signer_ok = signer
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && signer
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._@+-".contains(c));
    if !signer_ok {
        return die(
            "复验署名只允许 [A-Za-z0-9._@+-],且以字母或数字开头(多个复验方用 + 连接,如 m07-codex+claude)",
        );
    }
    let branch = format!("task/{number}");
    let run = |args: &[&str]| Command::new("git").args(args).output();
    let sha = match run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}^{{commit}}"),
    ]) {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => return die(&format!("本地没有分支 {branch}")),
    };
    if !run(&["remote", "get-url", &remote]).is_ok_and(|out| out.status.success()) {
        return die(&format!("没有名为 {remote} 的远程"));
    }
    let mut cmd = Command::new("git");
    cmd.args(["push", "-o", &format!("olp-verified={sha}:{signer}")]);
    if force {
        cmd.arg("--force");
    }
    cmd.arg(&remote)
        .arg(format!("refs/heads/{branch}:refs/heads/{branch}"));
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => die(&format!("无法运行 git: {e}")),
    }
}

/// `action` is the word after `olp repo`; `argv` what follows it.
pub fn run(action: &str, argv: &[String]) -> i32 {
    if argv.iter().any(|w| w == "-h" || w == "--help") {
        println!(
            "用法: octoscode olp repo status --repo <hub.git> --board <黑板或日志目录> [--remote github] [--json]\n      octoscode olp repo promote <N> --repo <hub.git> --board <黑板或日志目录> [--remote github] [--dry-run]\n      octoscode olp repo push <N> --verified-by <复验署名> [--remote hub] [--force]   (单元上)\n退出码:0 成功;1 promote 被闸门拒绝或推送失败;2 用法错误;6 主审权已失效"
        );
        return 0;
    }
    let result = match action {
        "status" => status(argv),
        "promote" => promote(argv),
        "push" => return push(argv),
        other => Err(Fail(2, format!("没有这个动作: {other}"))),
    };
    match result {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            if !why.is_empty() {
                eprintln!("olp-local-pr: {why}");
            }
            code
        }
    }
}
