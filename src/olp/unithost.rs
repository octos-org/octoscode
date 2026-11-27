//! `octoscode olp unit host` — keep the unit's host agent in its herdr pane.
//!
//! The host is the agent that claims a unit's cards and organises the inner
//! loops. When its program is gone — herdr itself restarted, the agent
//! crashed, the machine rebooted — nothing on the unit brings it back: the
//! watcher (`olp remote watch`) only reports that the pane takes no card.
//! Run from a keepalive job, this starts the host again in the same pane and
//! hands it a prompt to take up the cards it holds.
//!
//! It looks at the pane's foreground program only: a pane with a program in
//! it is left alone, whatever that program is doing. It reads no card and
//! judges nothing. Inner loops are the host's to bring back; restarts pause
//! after `--max-restarts` within `--window` seconds.
//!
//! Exit codes: 0 the host is running (already, or started now); 1 herdr
//! failed; 2 usage; 4 restarts are paused.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::outer::{
    Fail, busy_programs, fail, herdr, herdr_bin, now, pane_for_label, quote, submit_again,
};

const PROG: &str = "olp unit host";
const USAGE: &str = "用法: octoscode olp unit host --pane <窗格> | --label <工作区标签> [--cwd 目录] [--session 会话] [--ready 90] [--max-restarts 3] [--window 1800] [--prompt 话 | --no-prompt] [--reset] -- <主持 agent 的启动命令…>

单元主持的保活:窗格里没有程序在跑时(herdr 重启、agent 崩溃、机器重启之后),在原窗格重新启动主持,
并送一句接班提示,让它先看在手的卡。窗格里有程序在跑就不动。内环由主持自己恢复。
--pane 与 --label 给且只给一个;--label 指的工作区不存在时新建(用 --cwd)。
一般由常驻任务每分钟跑一次:octoscode olp keepalive install unit-host --interval 60 -- octoscode olp unit host …
退出码:0 主持在跑(原本就在,或刚拉起);1 herdr 出错;2 用法;4 重启过于频繁,已暂停";

struct Args {
    pane: Option<String>,
    label: Option<String>,
    cwd: Option<String>,
    session: Option<String>,
    ready: u64,
    max_restarts: usize,
    window: u64,
    prompt: Option<String>,
    no_prompt: bool,
    reset: bool,
    command: Vec<String>,
}

fn parse(argv: &[String]) -> Result<Args, Fail> {
    let (argv, command) = match argv.iter().position(|w| w == "--") {
        Some(at) => (&argv[..at], argv[at + 1..].to_vec()),
        None => (argv, Vec::new()),
    };
    let mut a = Args {
        pane: None,
        label: None,
        cwd: None,
        session: None,
        ready: 90,
        max_restarts: 3,
        window: 1800,
        prompt: None,
        no_prompt: false,
        reset: false,
        command,
    };
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--no-prompt" => {
                a.no_prompt = true;
                continue;
            }
            "--reset" => {
                a.reset = true;
                continue;
            }
            _ => {}
        }
        let mut value = || {
            words
                .next()
                .cloned()
                .ok_or_else(|| Fail(2, format!("{word} 需要一个值")))
        };
        let whole = |text: String| {
            text.parse::<u64>()
                .map_err(|_| Fail(2, format!("{word} 必须是整数")))
        };
        match word.as_str() {
            "--pane" => a.pane = Some(value()?),
            "--label" => a.label = Some(value()?),
            "--cwd" => a.cwd = Some(value()?),
            "--session" => a.session = Some(value()?),
            "--prompt" => a.prompt = Some(value()?),
            "--ready" => a.ready = whole(value()?)?,
            "--max-restarts" => a.max_restarts = whole(value()?)? as usize,
            "--window" => a.window = whole(value()?)?,
            other => return fail(2, format!("不认识的参数: {other}")),
        }
    }
    if a.pane.is_some() == a.label.is_some() {
        return fail(2, "--pane 与 --label 给且只给一个");
    }
    if a.pane.as_deref().is_some_and(|p| !super::card::is_pane(p))
        || a.label.as_deref().is_some_and(str::is_empty)
    {
        return fail(2, "窗格标识或标签不合法");
    }
    if a.command.is_empty() {
        return fail(2, "缺少主持 agent 的启动命令(写在 -- 之后)");
    }
    Ok(a)
}

/// One file per host: the restarts within the window.
fn state_path(a: &Args) -> PathBuf {
    let who = format!(
        "{}\n{}\n{}",
        a.session.as_deref().unwrap_or_default(),
        a.pane.as_deref().unwrap_or_default(),
        a.label.as_deref().unwrap_or_default()
    );
    let digest = Sha256::digest(who.as_bytes());
    let name: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    std::env::var_os("OLP_UNIT_HOST_STATE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".octos/outer/unithost")
        })
        .join(format!("{name}.json"))
}

fn save(path: &PathBuf, data: &Value) {
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| std::fs::write(path, data.to_string()));
    if let Err(e) = written {
        eprintln!("{PROG}: 记不下重启记录 {}: {e}", path.display());
    }
}

fn takeover(a: &Args, pane: &str) -> Option<String> {
    if a.no_prompt {
        return None;
    }
    if let Some(prompt) = a.prompt.as_ref().filter(|p| !p.is_empty()) {
        return Some(prompt.clone());
    }
    Some(format!(
        "单元主持刚被自动重启(窗格里的程序没了,由保活任务拉起)。接班:先运行 `octoscode olp remote inbox {pane}` 看待领取和在手的卡;在手的卡接着办(内环的窗格或进程不在了就按规程重新组织),办完照常回执。"
    ))
}

/// Wait until herdr sees the agent ready, then hand it the prompt. A
/// failure is said, not fatal: the host is running, and the watcher's
/// follow-up reaches a host that sits idle on a card it holds.
fn deliver(a: &Args, pane: &str) {
    let Some(text) = takeover(a, pane) else {
        return;
    };
    let session = a.session.as_deref();
    // A program that has only just started is not an agent to herdr yet,
    // and `agent wait` fails at once on a pane without one.
    let deadline = Instant::now() + Duration::from_secs(a.ready);
    loop {
        let seen = herdr(&["pane", "get", pane], session).is_ok_and(|got| {
            got.pointer("/pane/agent")
                .and_then(Value::as_str)
                .is_some_and(|agent| !agent.is_empty())
        });
        if seen {
            break;
        }
        if Instant::now() >= deadline {
            // herdr does not know every agent (Kimi Code shows up as
            // `kimi-co` and stays unknown): the words still go in.
            typed(pane, &text, session, "herdr 没认出这里的 agent");
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let timeout = (a.ready * 1000).to_string();
    let sent = herdr(
        &[
            "agent",
            "wait",
            pane,
            "--until",
            "idle",
            "--until",
            "blocked",
            "--timeout",
            &timeout,
        ],
        session,
    )
    .and_then(|_| {
        herdr(
            &[
                "agent",
                "prompt",
                pane,
                &text,
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
    })
    .or_else(|first| submit_again(pane, session).ok_or(first));
    match sent {
        Ok(_) => println!("prompted pane {pane}"),
        // Still on the screen: it was typed and the agent has not taken it;
        // typing it again would say it twice.
        Err(Fail(_, why)) if screen(pane, session).is_some_and(|now| now.contains(head(&text))) => {
            eprintln!("{PROG}: 提醒:接班提示没送到窗格 {pane}: {why}")
        }
        Err(Fail(_, why)) => typed(pane, &text, session, &why),
    }
}

/// What the pane shows now, as plain text.
fn screen(pane: &str, session: Option<&str>) -> Option<String> {
    let mut cmd = std::process::Command::new(herdr_bin());
    if let Some(session) = session {
        cmd.args(["--session", session]);
    }
    let out = cmd
        .args(["pane", "read", pane])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The opening of a prompt, enough to find it on a screen.
fn head(text: &str) -> &str {
    let end = text.char_indices().nth(12).map_or(text.len(), |(at, _)| at);
    &text[..end]
}

/// The pause between the words and the Enter. An Enter that follows at
/// once is taken by some agents as part of the paste and submits nothing.
fn enter_pause() -> Duration {
    let ms = std::env::var("OLP_UNIT_HOST_ENTER_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    Duration::from_millis(ms)
}

/// The prompt, typed into the pane as a person would: for an agent herdr
/// cannot follow. Nothing confirms the agent took it, and this says so;
/// the watcher's follow-up reaches a host that sits idle on a card.
fn typed(pane: &str, text: &str, session: Option<&str>, why: &str) {
    let pause = enter_pause();
    // A program still drawing its first screen loses what is typed at it.
    let mut last = screen(pane, session);
    for _ in 0..15 {
        std::thread::sleep(pause.min(Duration::from_secs(1)));
        let now = screen(pane, session);
        if now == last {
            break;
        }
        last = now;
    }
    let sent = herdr(&["pane", "send-text", pane, text], session).and_then(|_| {
        std::thread::sleep(pause);
        herdr(&["pane", "send-keys", pane, "Enter"], session)
    });
    match sent {
        Ok(_) => println!("typed into pane {pane}({why};直接敲进了窗格,没法确认 agent 接住)"),
        Err(Fail(_, failed)) => {
            eprintln!("{PROG}: 提醒:接班提示没送到窗格 {pane}: {why};直接敲入也失败: {failed}")
        }
    }
}

fn host(a: Args) -> Result<i32, Fail> {
    let session = a.session.as_deref();
    let state = state_path(&a);
    if a.reset {
        save(&state, &json!({}));
    }
    let (pane, created) = match (&a.pane, &a.label) {
        (Some(pane), _) => (pane.clone(), false),
        (None, Some(label)) => {
            let home = std::env::var("HOME").unwrap_or_default();
            pane_for_label(label, a.cwd.as_deref().unwrap_or(&home), session)?
        }
        (None, None) => return fail(2, USAGE),
    };
    let busy = busy_programs(&pane, session)?;
    if !busy.is_empty() && !created {
        println!("running in pane {pane} ({})", busy.join(", "));
        return Ok(0);
    }
    let started = now();
    let record: Value = std::fs::read_to_string(&state)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    let mut recent: Vec<f64> = record["restarts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .filter(|t| started - t < a.window as f64)
        .collect();
    if recent.len() >= a.max_restarts {
        eprintln!(
            "{PROG}: 已暂停:{} 秒内重启了 {} 次,主持一起来就退出,需要人看(看过后带 --reset 跑一次)",
            a.window,
            recent.len()
        );
        return Ok(4);
    }
    let line = a
        .command
        .iter()
        .map(|w| quote(w))
        .collect::<Vec<_>>()
        .join(" ");
    herdr(&["pane", "run", &pane, &line], session)?;
    recent.push(started);
    save(&state, &json!({"restarts": recent, "last_pane": pane}));
    // Started means a program is in the pane's foreground and stays there.
    let deadline = Instant::now() + Duration::from_secs(a.ready.min(20));
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if !busy_programs(&pane, session)?.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            return fail(
                1,
                format!("已在窗格 {pane} 执行启动命令,但里面没有程序在跑"),
            );
        }
    }
    println!("restarted in pane {pane}");
    deliver(&a, &pane);
    Ok(0)
}

/// `argv` is what follows `olp unit host`.
pub fn run(argv: &[String]) -> i32 {
    let before_command = argv.iter().take_while(|w| *w != "--");
    if before_command.clone().any(|w| w == "-h" || w == "--help") {
        println!("{USAGE}");
        return 0;
    }
    match parse(argv).and_then(host) {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            eprintln!("{PROG}: {why}");
            if code == 2 {
                eprintln!("{USAGE}");
            }
            code
        }
    }
}
