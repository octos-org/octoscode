//! `octoscode olp repo init|serve|connect-local|local-pack|hook` — the
//! hub's local git repository: a unit may push only its own task branch,
//! and only with a local verification record.
//!
//! Units do not push to GitHub. They push `task/<N>` to a bare repository
//! on the hub; the outer loop reviews from there. `serve` is the ssh forced
//! command that pins the unit id; the repository's `pre-receive` hook then
//! holds every push that carries a unit id to the rules, and `post-receive`
//! records the push on the blackboard.
//!
//! Rules for a push with a unit id:
//! 1. only `refs/heads/task/<N>`; no other branch, no tag, no deletion;
//! 2. card N exists and its last `unit:` names this unit;
//! 3. a push option `olp-verified=<the new head's full sha>:<signer>`;
//! 4. when the card has a `review:` line, the signer covers every name on
//!    its last one (`m07-codex+claude`); an empty line asks for none.
//!
//! A push without a unit id (the outer loop, on the hub itself) is not
//! restricted — unless it carries a verification record, which means a
//! unit's clone is not connected through `serve`: that is refused.
//!
//! The blackboard is a Markdown file or an event log directory
//! (`git config olp.board`); the card is read from whichever it is, and
//! the push is recorded there.
//!
//! Exit codes: 0; 1 push refused (hooks); 2 usage; 3 `serve` refused.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;

use super::log::{Draft, Log};
use super::unit::is_unit;

const SIGNER_MAX: usize = 4096;

fn die(code: i32, why: &str) -> i32 {
    eprintln!("olp-hub: {why}");
    code
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        Ok(out) => Err(format!(
            "git {} 失败: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => Err(format!("git {} 失败: {e}", args.join(" "))),
    }
}

fn absolute(path: &str) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn real(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| absolute(path))
}

/// This binary as a later command should call it: the `octoscode` on PATH
/// when that is the same file (a package manager's link survives upgrades),
/// else the running file.
fn self_exe() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("octoscode"));
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("octoscode"))
            .find(|p| p.is_file())
    });
    match on_path {
        Some(p) if std::fs::canonicalize(&p).ok() == std::fs::canonicalize(&exe).ok() => {
            std::path::absolute(&p).unwrap_or(p)
        }
        _ => exe,
    }
    .to_string_lossy()
    .into_owned()
}

fn shell(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| {
            shlex::try_quote(w)
                .map(|q| q.into_owned())
                .unwrap_or_else(|_| w.to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `--name value` options and positionals.
fn options<'a>(
    argv: &'a [String],
    names: &[&str],
) -> Result<(Vec<Option<&'a str>>, Vec<&'a str>), String> {
    let mut values = vec![None; names.len()];
    let mut rest = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        if let Some(at) = names.iter().position(|n| n == word) {
            values[at] = Some(
                words
                    .next()
                    .map(String::as_str)
                    .ok_or_else(|| format!("{word} 需要一个值"))?,
            );
        } else if let Some((name, value)) = word.split_once('=').filter(|(n, _)| names.contains(n))
        {
            let at = names.iter().position(|n| *n == name).unwrap_or(0);
            values[at] = Some(value);
        } else if word.starts_with("--") {
            return Err(format!("不认识的参数: {word}"));
        } else {
            rest.push(word.as_str());
        }
    }
    Ok((values, rest))
}

// --- init ---------------------------------------------------------------

fn init(argv: &[String]) -> i32 {
    let (values, rest) = match options(argv, &["--board"]) {
        Ok(parsed) => parsed,
        Err(why) => return die(2, &why),
    };
    let ([repo], Some(board)) = (rest.as_slice(), values[0]) else {
        return die(
            2,
            "用法: octoscode olp repo init <裸仓库路径> --board <黑板路径>",
        );
    };
    let repo = absolute(repo);
    let board = absolute(board);
    if !board.is_file() && !board.is_dir() {
        return die(2, &format!("黑板不存在: {}", board.display()));
    }
    if repo.exists() {
        if git(&["rev-parse", "--is-bare-repository"], Some(&repo)).as_deref() != Ok("true") {
            return die(2, &format!("{} 已存在且不是裸仓库", repo.display()));
        }
    } else if let Err(why) = git(
        &["init", "--bare", "--quiet", &repo.to_string_lossy()],
        None,
    ) {
        return die(2, &why);
    }
    let board_text = board.to_string_lossy();
    for args in [
        vec!["config", "olp.board", &*board_text],
        // The verification record arrives as a push option.
        vec!["config", "receive.advertisePushOptions", "true"],
    ] {
        if let Err(why) = git(&args, Some(&repo)) {
            return die(2, &why);
        }
    }
    // Older versions recorded a scripts directory.
    let _ = git(&["config", "--unset", "olp.scripts"], Some(&repo));
    let hooks = repo.join("hooks");
    let exe = self_exe();
    let installed = std::fs::create_dir_all(&hooks).and_then(|()| {
        for name in ["pre-receive", "post-receive"] {
            let path = hooks.join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\n# Installed by olp repo init; re-run init to refresh.\nexec {}\n",
                    shell(&[&exe, "olp", "repo", "hook", name])
                ),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut mode = std::fs::metadata(&path)?.permissions();
                mode.set_mode(mode.mode() | 0o111);
                std::fs::set_permissions(&path, mode)?;
            }
        }
        Ok(())
    });
    if let Err(e) = installed {
        return die(2, &format!("装不了钩子: {e}"));
    }
    println!(
        "hub repo ready: {} (board: {})",
        repo.display(),
        board.display()
    );
    0
}

// --- serve (ssh forced command) -----------------------------------------

fn serve(unit: &str, repo: &str, original: &str) -> i32 {
    if !is_unit(unit) {
        return die(2, &format!("单元号不合法: {unit}"));
    }
    let repo = real(repo);
    let mut words = shlex::split(original).unwrap_or_default();
    if words.len() == 3
        && words[0] == "git"
        && matches!(words[1].as_str(), "receive-pack" | "upload-pack")
    {
        words = vec![format!("git-{}", words[1]), words[2].clone()];
    }
    let [pack, path] = words.as_slice() else {
        return die(3, "这个密钥只能用于对 hub 仓库的 git 推送与拉取");
    };
    if !matches!(pack.as_str(), "git-receive-pack" | "git-upload-pack") {
        return die(3, "这个密钥只能用于对 hub 仓库的 git 推送与拉取");
    }
    if real(path) != repo {
        return die(3, &format!("这个密钥只能访问 {}", repo.display()));
    }
    let mut cmd = Command::new("git");
    cmd.arg(&pack[4..]).arg(&repo).env("OLP_UNIT", unit);
    #[cfg(unix)]
    {
        let failed = std::os::unix::process::CommandExt::exec(&mut cmd);
        die(3, &format!("无法运行 git: {failed}"))
    }
    #[cfg(not(unix))]
    {
        match cmd.status() {
            Ok(status) => status.code().unwrap_or(3),
            Err(e) => die(3, &format!("无法运行 git: {e}")),
        }
    }
}

fn serve_args(argv: &[String]) -> Result<(String, String, Vec<String>), String> {
    let (values, rest) = options(argv, &["--unit", "--repo"])?;
    match (values[0], values[1]) {
        (Some(unit), Some(repo)) => Ok((
            unit.to_string(),
            repo.to_string(),
            rest.iter().map(|w| w.to_string()).collect(),
        )),
        _ => Err("需要 --unit <单元号> 与 --repo <裸仓库路径>".to_string()),
    }
}

/// Single machine: point this repository's hub remote at the bare repo on
/// the same disk, so pushes still go through `serve` with the unit id.
fn connect_local(argv: &[String]) -> i32 {
    let (values, rest) = match options(argv, &["--unit", "--repo", "--remote"]) {
        Ok(parsed) => parsed,
        Err(why) => return die(2, &why),
    };
    let (Some(unit), Some(repo), true) = (values[0], values[1], rest.is_empty()) else {
        return die(
            2,
            "用法: octoscode olp repo connect-local --unit <单元号> --repo <裸仓库路径> [--remote hub]",
        );
    };
    let remote = values[2].unwrap_or("hub");
    if !is_unit(unit) {
        return die(2, &format!("单元号不合法: {unit}"));
    }
    let repo = real(repo);
    if !repo.is_dir() {
        return die(2, &format!("hub 仓库不存在: {}", repo.display()));
    }
    let repo = repo.to_string_lossy().into_owned();
    // git runs remote.<name>.receivepack through the shell with the
    // repository path appended: `serve` sees what an ssh push would send.
    let exe = self_exe();
    let base = [
        exe.as_str(),
        "olp",
        "repo",
        "local-pack",
        "--unit",
        unit,
        "--repo",
        repo.as_str(),
    ];
    let with = |pack: &str| {
        let mut words = base.to_vec();
        words.push(pack);
        shell(&words)
    };
    let verb = if git(&["remote", "get-url", remote], None).is_ok() {
        "set-url"
    } else {
        "add"
    };
    let receive = (
        format!("remote.{remote}.receivepack"),
        with("git-receive-pack"),
    );
    let upload = (
        format!("remote.{remote}.uploadpack"),
        with("git-upload-pack"),
    );
    for args in [
        ["remote", verb, remote, repo.as_str()].as_slice(),
        ["config", &receive.0, &receive.1].as_slice(),
        ["config", &upload.0, &upload.1].as_slice(),
    ] {
        if let Err(why) = git(args, None) {
            return die(2, &why);
        }
    }
    println!("remote {remote} -> {repo}(本机;推送经 serve,单元号 {unit})");
    0
}

// --- hooks --------------------------------------------------------------

/// What the blackboard says about card `number`: `None` when there is no
/// such card; else its unit and, when it has a `review:` line, the names on
/// the last one.
type Card = Option<(Option<String>, Option<Vec<String>>)>;

/// A code fence line: up to three spaces, three or more of one of `` ` `` or
/// `~`, then the rest of the line.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let mark = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = rest.len() - rest.trim_start_matches(mark).len();
    (run >= 3).then(|| (mark, run, &rest[run..]))
}

fn card_on_board(text: &str, number: u64) -> Card {
    let (mut found, mut unit, mut reviewers) = (false, None, None);
    let mut current: Option<u64> = None;
    let mut open: Option<(char, usize)> = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let here = fence(line);
        if let Some((mark, length)) = open {
            // Only a fence of the same character, at least as long and with
            // nothing after it, closes the open one.
            if here.is_some_and(|(m, n, rest)| m == mark && n >= length && rest.trim().is_empty()) {
                open = None;
            }
            continue;
        }
        if let Some((mark, length, rest)) = here {
            if !(mark == '`' && rest.contains('`')) {
                open = Some((mark, length));
                continue;
            }
        }
        if line.starts_with("### ") || line.starts_with("## ") {
            current = line.strip_prefix("### ").and_then(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok()
            });
            found |= current == Some(number);
            continue;
        }
        if current != Some(number) {
            continue;
        }
        if let Some(value) = line.strip_prefix("unit:") {
            let value = value.trim();
            // The last one wins: a reassignment appends a new unit line.
            if !value.is_empty() && !value.contains(char::is_whitespace) {
                unit = Some(value.to_string());
            }
        }
        if let Some(value) = line.strip_prefix("review:") {
            reviewers = Some(
                value
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|r| !r.is_empty())
                    .map(str::to_string)
                    .collect(),
            );
        }
    }
    found.then_some((unit, reviewers))
}

/// One read of the blackboard for a card. `Err` when it cannot be read:
/// the caller refuses rather than guess.
fn read_card(board: &Path, number: u64) -> Result<Card, String> {
    if board.is_dir() {
        let events = Log::at(board).read().map_err(|e| e.to_string())?;
        let scan = super::cards::scan(&events);
        return Ok(scan.entries.get(&number).map(|entry| {
            let review = (!entry.review.is_empty()).then(|| entry.review.clone());
            (entry.unit.clone(), review)
        }));
    }
    let bytes = std::fs::read(board).map_err(|e| e.to_string())?;
    Ok(card_on_board(&String::from_utf8_lossy(&bytes), number))
}

fn push_options() -> Vec<String> {
    let count = std::env::var("GIT_PUSH_OPTION_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    (0..count)
        .map(|i| std::env::var(format!("GIT_PUSH_OPTION_{i}")).unwrap_or_default())
        .collect()
}

fn is_signer(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.len() <= SIGNER_MAX
        && chars.all(|c| c.is_ascii_alphanumeric() || "._@+-".contains(c))
}

/// The signer, when a push option vouches for exactly `new`.
fn verified_by(new: &str) -> Option<String> {
    push_options().iter().find_map(|option| {
        let (sha, signer) = option.strip_prefix("olp-verified=")?.split_once(':')?;
        (sha == new && is_signer(signer)).then(|| signer.to_string())
    })
}

/// `m07-codex+claude` → codex, claude (the unit prefix is optional).
fn signed_reviewers(signer: &str, unit: &str) -> Vec<String> {
    let lower = signer.to_lowercase();
    let body = lower
        .strip_prefix(&format!("{}-", unit.to_lowercase()))
        .unwrap_or(&lower);
    body.split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn updates() -> Vec<(String, String)> {
    std::io::stdin()
        .lock()
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            (parts.len() == 3).then(|| (parts[1].to_string(), parts[2].to_string()))
        })
        .collect()
}

fn task_number(reference: &str) -> Option<&str> {
    reference
        .strip_prefix("refs/heads/task/")
        .filter(|n| (1..=9).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()))
}

fn is_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn board_of_repo() -> PathBuf {
    PathBuf::from(git(&["config", "olp.board"], None).unwrap_or_default())
}

fn pre_receive() -> i32 {
    let unit = std::env::var("OLP_UNIT").unwrap_or_default();
    let updates = updates();
    if unit.is_empty() {
        // The outer loop's own pushes are not held to the unit's rules. One
        // that carries a verification record is a unit handing in work
        // without going through `serve`: accepting it would skip the rules
        // and leave the blackboard without a record.
        if push_options()
            .iter()
            .any(|o| o.starts_with("olp-verified="))
        {
            eprintln!(
                "olp-hub: 拒收:这是单元带复验记录的推送,却没有经过 serve(没有单元号)。单机部署时先在这个克隆里执行 octoscode olp repo connect-local --unit <单元号> --repo <hub 仓库>,再推送"
            );
            return 1;
        }
        return 0;
    }
    if !is_unit(&unit) {
        eprintln!("olp-hub: 拒收:单元号不合法 {unit}");
        return 1;
    }
    let board = board_of_repo();
    let mut errors = Vec::new();
    for (new, reference) in &updates {
        let Some(number) = task_number(reference) else {
            errors.push(format!("{reference}:单元只能推 refs/heads/task/<N>"));
            continue;
        };
        if new.bytes().all(|b| b == b'0') {
            errors.push(format!("{reference}:单元不能删除分支"));
            continue;
        }
        if !is_sha(new) {
            errors.push(format!("{reference}:提交号不合法"));
            continue;
        }
        let n: u64 = number.parse().unwrap_or(0);
        let (assigned, required) = match read_card(&board, n) {
            Err(why) => {
                errors.push(format!(
                    "{reference}:读不了黑板 {}({why}),无法核对派单与复验方",
                    board.display()
                ));
                continue;
            }
            Ok(None) => {
                errors.push(format!("{reference}:黑板上没有条目 #{n}"));
                continue;
            }
            Ok(Some(card)) => card,
        };
        if assigned.as_deref() != Some(unit.as_str()) {
            let who = assigned.unwrap_or_else(|| "未指定单元(条目里没有 unit: 行)".to_string());
            errors.push(format!("{reference}:#{n} 派给的是 {who},不是 {unit}"));
            continue;
        }
        let Some(signer) = verified_by(new) else {
            errors.push(format!(
                "{reference}:缺少本地复验记录。本地复验通过后用 scripts/olp-task-push.sh {n} --verified-by <复验署名> 推送(push option olp-verified 的 sha 必须等于分支头)"
            ));
            continue;
        };
        // The card names the reviewers; every one of them must have signed.
        let required = required.unwrap_or_default();
        let signed = signed_reviewers(&signer, &unit);
        let missing: Vec<&str> = required
            .iter()
            .filter(|r| !signed.contains(&r.to_lowercase()))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            errors.push(format!(
                "{reference}:#{n} 卡上指定的复验方是 {},复验署名 {signer} 缺少 {}(署名写成 {unit}-{};改了复验方就在卡里追加新的 review: 行)",
                required.join("、"),
                missing.join("、"),
                required.join("+")
            ));
        }
    }
    for e in &errors {
        eprintln!("olp-hub: 拒收 {e}");
    }
    i32::from(!errors.is_empty())
}

/// Record a received push on the blackboard.
fn record(board: &Path, unit: &str, number: &str, new: &str, signer: &str) -> Result<(), String> {
    let said = format!(
        "task/{number} @ {} 复验 {signer}",
        &new[..12.min(new.len())]
    );
    if board.is_dir() {
        let draft = Draft {
            kind: "pushed".to_string(),
            card: number.parse().ok(),
            actor: format!("单元({unit})"),
            unit: Some(unit.to_string()),
            key: Some(format!("{unit}:pushed:{number}:{new}")),
            data: json!({ "text": said }),
        };
        return Log::at(board)
            .append_with(|events| {
                let seen = events.iter().any(|e| e.key == draft.key);
                Ok::<_, String>(if seen { Vec::new() } else { vec![draft] })
            })
            .map(|_| ())
            .map_err(|e| format!("{e:?}"));
    }
    let line = format!("> 单元({unit})·本地复验通过(#{number}): {said}\n");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(["olp", "board", "append"]).arg(board);
    // The hook's GIT_DIR and friends are not for the board writer.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(name);
        }
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(line.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn post_receive() -> i32 {
    let unit = std::env::var("OLP_UNIT").unwrap_or_default();
    let updates = updates();
    if unit.is_empty() {
        return 0;
    }
    let board = board_of_repo();
    for (new, reference) in &updates {
        let Some(number) = task_number(reference) else {
            continue;
        };
        if new.bytes().all(|b| b == b'0') {
            continue;
        }
        let signer = verified_by(new).unwrap_or_else(|| "?".to_string());
        let short = &new[..12.min(new.len())];
        // A failed record does not undo the push; the pusher is told.
        match record(&board, &unit, number, new, &signer) {
            Ok(()) => eprintln!("olp-hub: 已收 {reference} @ {short},已在黑板登记"),
            Err(why) => eprintln!(
                "olp-hub: 已收 {reference} @ {short},但黑板登记失败({why});请告知大外环这次推送"
            ),
        }
    }
    0
}

/// `action` is the word after `olp repo`; `argv` what follows it.
pub fn run(action: &str, argv: &[String]) -> i32 {
    if argv.iter().any(|w| w == "-h" || w == "--help") {
        println!(
            "用法: octoscode olp repo init <裸仓库路径> --board <黑板路径>\n      octoscode olp repo serve --unit <单元号> --repo <裸仓库路径>   (ssh 强制命令)\n      octoscode olp repo connect-local --unit <单元号> --repo <裸仓库路径> [--remote hub]\n      octoscode olp repo hook pre-receive|post-receive            (由钩子调用)\n      octoscode olp repo local-pack …                             (由 git 调用)\n退出码:0 成功;1 推送被拒;2 用法错误;3 serve 拒绝的命令"
        );
        return 0;
    }
    match action {
        "init" => init(argv),
        "connect-local" => connect_local(argv),
        "serve" => match serve_args(argv) {
            Ok((unit, repo, rest)) if rest.is_empty() => serve(
                &unit,
                &repo,
                &std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default(),
            ),
            Ok(_) => die(2, "serve 不接受位置参数"),
            Err(why) => die(2, &why),
        },
        "local-pack" => match serve_args(argv) {
            // What git runs for a push or fetch on a connected remote.
            Ok((unit, repo, rest))
                if rest.first().is_some_and(|p| {
                    matches!(p.as_str(), "git-receive-pack" | "git-upload-pack")
                }) =>
            {
                let words: Vec<&str> = rest.iter().map(String::as_str).collect();
                serve(&unit, &repo, &shell(&words))
            }
            Ok(_) => die(2, "local-pack 需要 git-receive-pack 或 git-upload-pack"),
            Err(why) => die(2, &why),
        },
        "hook" => match argv {
            [name] if name == "pre-receive" => pre_receive(),
            [name] if name == "post-receive" => post_receive(),
            _ => die(2, "hook 需要 pre-receive 或 post-receive"),
        },
        other => die(2, &format!("没有这个动作: {other}")),
    }
}
