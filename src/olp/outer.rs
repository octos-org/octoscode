//! `octoscode olp outer supervise|resume|install` — bring the outer loop
//! back when it dies.
//!
//! The outer loop runs in a herdr pane and holds the project's duty lock
//! through `octoscode outer-duty hold`. When it dies the lock goes VACANT.
//! `supervise` checks once: a HELD lock is left alone — so it never makes a
//! second outer loop and never takes over a live lock — and a VACANT one is
//! taken again by running `hold … -- <outer command>` in the outer's pane,
//! unless a program is still running there. Restarting too often pauses it.
//! After a restart the agent is handed a prompt to run `resume` first.
//!
//! `resume` is the read-only briefing for whoever takes over: the lock and
//! lease, recent restarts, the inbox, questions, alerts, units, and a
//! checklist. `install` registers `supervise` as a keepalive job.
//!
//! Exit codes: 0 the lock is HELD (already, or after a restart; `resume`:
//! this session holds it); 1 error; 2 usage; 3 a program still runs in the
//! outer's pane (`resume`: this session is not the holder); 4 paused after
//! too many restarts.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PROG: &str = "olp-outer";
const SHELLS: [&str; 9] = [
    "zsh", "bash", "sh", "fish", "dash", "ksh", "tcsh", "csh", "nu",
];
/// Short-lived helpers a shell runs while it starts.
const SHELL_HELPERS: [&str; 10] = [
    "git",
    "sed",
    "awk",
    "uname",
    "tput",
    "stty",
    "dircolors",
    "locale",
    "basename",
    "dirname",
];

pub(super) struct Fail(pub(super) i32, pub(super) String);

pub(super) fn fail<T>(code: i32, why: impl Into<String>) -> Result<T, Fail> {
    Err(Fail(code, why.into()))
}

pub(super) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn absolute(path: &str) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

pub(super) fn quote(word: &str) -> String {
    shlex::try_quote(word)
        .map(|q| q.into_owned())
        .unwrap_or_else(|_| word.to_string())
}

fn herdr_bin() -> String {
    std::env::var("OLP_HERDR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "herdr".to_string())
}

/// Run herdr and return its `result`.
pub(super) fn herdr(args: &[&str], session: Option<&str>) -> Result<Value, Fail> {
    let mut cmd = Command::new(herdr_bin());
    if let Some(session) = session {
        cmd.args(["--session", session]);
    }
    let out = cmd
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Fail(1, format!("herdr 不可用: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let data: Value = serde_json::from_str(text.trim()).unwrap_or_else(|_| json!({}));
    if !out.status.success() || data.get("error").is_some() {
        let said = data
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if err.is_empty() {
                    text.trim().to_string()
                } else {
                    err
                }
            });
        let what: Vec<&str> = args.iter().take(2).copied().collect();
        return fail(1, format!("herdr {} 失败: {said}", what.join(" ")));
    }
    Ok(data.get("result").cloned().unwrap_or_else(|| json!({})))
}

fn this_exe() -> Result<PathBuf, Fail> {
    std::env::current_exe().map_err(|e| Fail(1, format!("找不到 octoscode 自身: {e}")))
}

/// `HELD`, `VACANT`, or whatever `outer-duty check` said.
fn duty_state(project: &str) -> String {
    let said = this_exe().ok().and_then(|exe| {
        Command::new(exe)
            .args(["outer-duty", "check", "--project", project])
            .output()
            .ok()
    });
    said.map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ERROR".to_string())
}

fn state_path(project: &str) -> PathBuf {
    let real = std::fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
    let digest = Sha256::digest(real.to_string_lossy().as_bytes());
    let name: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    std::env::var_os("OLP_OUTER_STATE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".octos/outer/supervise")
        })
        .join(format!("{name}.json"))
}

fn load_state(project: &str) -> Value {
    std::fs::read_to_string(state_path(project))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn save_state(project: &str, data: &Value) {
    let path = state_path(project);
    let tmp = path.with_extension("json.tmp");
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| std::fs::write(&tmp, data.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        eprintln!("{PROG}: 记不下重启记录 {}: {e}", path.display());
    }
}

/// One line on the blackboard. The supervisor is not the outer loop: its
/// write does not carry the outer's lease.
fn board_note(board: Option<&str>, text: &str) {
    let Some(board) = board else { return };
    if Path::new(board).is_dir() {
        // An event log: the note is an event of its own, tied to no card.
        let note = text.trim().trim_start_matches('>').trim();
        let written = super::log::Log::at(board).append(super::log::Draft {
            kind: "note".to_string(),
            actor: "外环监督".to_string(),
            data: json!({ "text": note }),
            ..Default::default()
        });
        if let Err(e) = written {
            eprintln!("{PROG}: 提醒:没写进事件日志: {e}");
        }
        return;
    }
    let Ok(exe) = this_exe() else { return };
    let mut cmd = Command::new(exe);
    cmd.args(["olp", "board", "append", board]);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("OLP_DUTY_") {
            cmd.env_remove(name);
        }
    }
    let ran = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut pipe) = child.stdin.take() {
                let _ = std::io::Write::write_all(&mut pipe, text.as_bytes());
            }
            child.wait_with_output()
        });
    match ran {
        Ok(out) if out.status.success() => {}
        Ok(out) => eprintln!(
            "{PROG}: 提醒:没写进黑板: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => eprintln!("{PROG}: 提醒:没写进黑板: {e}"),
    }
}

/// Everything `supervise` and `install` take.
struct Common {
    project: String,
    signature: String,
    duties: String,
    label: String,
    cwd: Option<String>,
    session: Option<String>,
    board: Option<String>,
    max_restarts: usize,
    window: u64,
    wait: u64,
    prompt: Option<String>,
    no_prompt: bool,
    ready: f64,
    settle: f64,
    reset: bool,
    // install only
    interval: u64,
    name: String,
    no_load: bool,
    dir: Option<String>,
    // resume only
    repo: Option<String>,
    json: bool,
    command: Vec<String>,
}

fn parse(action: &str, argv: &[String]) -> Result<Common, Fail> {
    let (argv, command) = match argv.iter().position(|w| w == "--") {
        Some(at) => (&argv[..at], argv[at + 1..].to_vec()),
        None => (argv, Vec::new()),
    };
    let mut c = Common {
        project: String::new(),
        signature: String::new(),
        duties: String::new(),
        label: "olp-outer".to_string(),
        cwd: None,
        session: None,
        board: None,
        max_restarts: 3,
        window: 1800,
        wait: 20,
        prompt: None,
        no_prompt: false,
        ready: 90.0,
        settle: 2.0,
        reset: false,
        interval: 60,
        name: "outer".to_string(),
        no_load: false,
        dir: None,
        repo: None,
        json: false,
        command,
    };
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        match name {
            "--no-prompt" => {
                c.no_prompt = true;
                continue;
            }
            "--reset" if action == "supervise" => {
                c.reset = true;
                continue;
            }
            "--no-load" if action == "install" => {
                c.no_load = true;
                continue;
            }
            "--json" if action == "resume" => {
                c.json = true;
                continue;
            }
            _ => {}
        }
        let mut value = || {
            inline
                .clone()
                .or_else(|| words.next().cloned())
                .ok_or_else(|| Fail(2, format!("{name} 需要一个值")))
        };
        let whole = |text: String| {
            text.parse::<u64>()
                .map_err(|_| Fail(2, format!("{name} 必须是整数")))
        };
        let real = |text: String| {
            text.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| Fail(2, format!("{name} 必须是数字")))
        };
        match name {
            "--project" => c.project = value()?,
            "--signature" => c.signature = value()?,
            "--duties" => c.duties = value()?,
            "--label" => c.label = value()?,
            "--cwd" => c.cwd = Some(value()?),
            "--session" => c.session = Some(value()?),
            "--board" => c.board = Some(value()?),
            "--prompt" => c.prompt = Some(value()?),
            "--max-restarts" => c.max_restarts = whole(value()?)? as usize,
            "--window" => c.window = whole(value()?)?,
            "--wait" => c.wait = whole(value()?)?,
            "--ready" => c.ready = real(value()?)?,
            "--settle" => c.settle = real(value()?)?,
            "--interval" if action == "install" => c.interval = whole(value()?)?,
            "--name" if action == "install" => c.name = value()?,
            "--dir" if action == "install" => c.dir = Some(value()?),
            "--repo" if action == "resume" => c.repo = Some(value()?),
            other => return fail(2, format!("不认识的参数: {other}")),
        }
    }
    if c.project.is_empty() {
        return fail(2, "缺少 --project");
    }
    if action == "resume" {
        if c.board.is_none() {
            return fail(2, "缺少 --board");
        }
    } else if c.signature.is_empty() || c.duties.is_empty() {
        return fail(2, "缺少 --signature 或 --duties");
    }
    Ok(c)
}

/// The outer's pane: the first pane of the workspace labelled `--label`,
/// created when there is none. `true` when it was just created.
fn find_pane(c: &Common) -> Result<(String, bool), Fail> {
    let cwd = c.cwd.as_deref().unwrap_or(&c.project);
    pane_for_label(&c.label, cwd, c.session.as_deref())
}

/// The first pane of the workspace labelled `label`; the workspace is
/// created in `cwd` when there is none (`true`).
pub(super) fn pane_for_label(
    label: &str,
    cwd: &str,
    session: Option<&str>,
) -> Result<(String, bool), Fail> {
    let workspaces = herdr(&["workspace", "list"], session)?;
    let found = workspaces["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|w| w["label"].as_str() == Some(label))
        .and_then(|w| w["workspace_id"].as_str().map(str::to_string));
    let Some(workspace) = found else {
        let created = herdr(
            &[
                "workspace",
                "create",
                "--cwd",
                cwd,
                "--label",
                label,
                "--no-focus",
            ],
            session,
        )?;
        return match created
            .pointer("/root_pane/pane_id")
            .and_then(Value::as_str)
        {
            Some(pane) => Ok((pane.to_string(), true)),
            None => fail(1, "herdr 没有给出新工作区的窗格"),
        };
    };
    let panes = herdr(&["pane", "list"], session)?;
    panes["panes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["workspace_id"].as_str() == Some(workspace.as_str()))
        .and_then(|p| p["pane_id"].as_str())
        .map(|pane| (pane.to_string(), false))
        .ok_or_else(|| Fail(1, format!("工作区 {label} 里没有窗格")))
}

/// The first pane of the workspace labelled `label`, if there is one.
pub(super) fn pane_of_label(label: &str, session: Option<&str>) -> Option<String> {
    let workspaces = herdr(&["workspace", "list"], session).ok()?;
    let workspace = workspaces["workspaces"]
        .as_array()?
        .iter()
        .find(|w| w["label"].as_str() == Some(label))?["workspace_id"]
        .as_str()?
        .to_string();
    let panes = herdr(&["pane", "list"], session).ok()?;
    panes["panes"]
        .as_array()?
        .iter()
        .find(|p| p["workspace_id"].as_str() == Some(workspace.as_str()))?["pane_id"]
        .as_str()
        .map(str::to_string)
}

/// Programs in the pane's foreground other than its shell and the helpers a
/// shell runs while starting.
pub(super) fn busy_programs(pane: &str, session: Option<&str>) -> Result<Vec<String>, Fail> {
    let info = herdr(&["pane", "process-info", "--pane", pane], session)?;
    Ok(info
        .pointer("/process_info/foreground_processes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|proc| {
            let raw = ["name", "argv0"]
                .iter()
                .find_map(|k| proc[*k].as_str().filter(|s| !s.is_empty()))?;
            let name = raw
                .rsplit('/')
                .next()
                .unwrap_or(raw)
                .trim_start_matches('-');
            (!name.is_empty() && !SHELLS.contains(&name) && !SHELL_HELPERS.contains(&name))
                .then(|| name.to_string())
        })
        .collect())
}

/// HELD for `settle` seconds: an outer loop that dies right after taking
/// the lock does not count as restarted.
fn stays_held(project: &str, settle: f64) -> bool {
    let end = Instant::now() + Duration::from_secs_f64(settle.max(0.0));
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(250));
        if duty_state(project) != "HELD" {
            return false;
        }
    }
    true
}

fn resume_prompt(c: &Common) -> Option<String> {
    if c.no_prompt {
        return None;
    }
    if let Some(prompt) = c.prompt.as_ref().filter(|p| !p.is_empty()) {
        return Some(prompt.clone());
    }
    let board = c.board.as_ref()?;
    Some(format!(
        "外环刚被自动重启(锁空出后由监督任务拉起)。接班:先运行 `octoscode olp outer resume --board {} --project {} --signature {}`,按它的检查单逐项恢复,再继续工作。",
        quote(board),
        quote(&c.project),
        quote(&c.signature)
    ))
}

/// Tell herdr how to bring this pane's program back after herdr itself
/// restarts: the same `hold … -- <outer command>`, so what comes back holds
/// the lock. Only the command is reported (`report-agent-session`), never
/// the agent's state: herdr keeps reading that from the screen. Without
/// this, an agent's own herdr integration would have herdr resume the bare
/// agent, which holds nothing. A failure is a reminder, not an error.
fn register_resume(c: &Common, pane: &str, hold: &[&str]) {
    let session = c.session.as_deref();
    // herdr takes a resume command only for a pane it sees an agent in.
    let deadline = Instant::now() + Duration::from_secs_f64(c.ready.max(0.0));
    let agent = loop {
        let seen = herdr(&["pane", "get", pane], session).ok().and_then(|got| {
            got.pointer("/pane/agent")
                .and_then(Value::as_str)
                .filter(|a| !a.is_empty())
                .map(str::to_string)
        });
        match seen {
            Some(agent) => break agent,
            None if Instant::now() >= deadline => {
                eprintln!(
                    "{PROG}: 提醒:herdr 没认出窗格 {pane} 里的 agent,没有登记恢复命令(herdr 重启后要靠监督任务拉起)"
                );
                return;
            }
            None => std::thread::sleep(Duration::from_secs(1)),
        }
    };
    let seq = (now() as u64).to_string();
    // `hold` starts with `env`: herdr wants a plain command name first.
    let mut args = vec![
        "pane",
        "report-agent-session",
        pane,
        "--source",
        "olp-outer",
        "--agent",
        &agent,
        "--seq",
        &seq,
        "--",
    ];
    args.extend(hold);
    match herdr(&args, session) {
        Ok(_) => println!("resume command registered for pane {pane}"),
        Err(Fail(_, why)) => {
            eprintln!("{PROG}: 提醒:没能登记恢复命令({why});herdr 重启后要靠监督任务拉起")
        }
    }
}

/// The prompt was typed but the agent did not start on it: an agent that
/// has only just started can lose the Enter. While the agent is idle — never
/// while it waits on a dialog of its own — press Enter again, a few times.
pub(super) fn submit_again(pane: &str, session: Option<&str>) -> Option<Value> {
    for _ in 0..4 {
        std::thread::sleep(Duration::from_secs(3));
        herdr(
            &[
                "agent",
                "wait",
                pane,
                "--until",
                "idle",
                "--timeout",
                "1000",
            ],
            session,
        )
        .ok()?;
        herdr(&["pane", "send-keys", pane, "Enter"], session).ok()?;
        let taken = herdr(
            &[
                "agent",
                "wait",
                pane,
                "--until",
                "working",
                "--until",
                "blocked",
                "--timeout",
                "10000",
            ],
            session,
        );
        if let Ok(taken) = taken {
            return Some(taken);
        }
    }
    None
}

/// Wait until herdr sees the restarted agent ready, then hand it the resume
/// prompt. A failure is noted, not fatal: the outer loop is running.
fn deliver_prompt(c: &Common, pane: &str) {
    let Some(text) = resume_prompt(c) else { return };
    let session = c.session.as_deref();
    let timeout = ((c.ready * 1000.0) as i64).to_string();
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
        Err(Fail(_, why)) => {
            eprintln!("{PROG}: 提醒:接班提示没送到窗格 {pane}: {why}");
            board_note(
                c.board.as_deref(),
                &format!(
                    "> 外环已重启但接班提示未送达({}):窗格 {pane},{why}。需要人看一眼。{}\n",
                    c.signature,
                    stamp()
                ),
            );
        }
    }
}

fn supervise(mut c: Common) -> Result<i32, Fail> {
    if c.command.is_empty() {
        return fail(2, "缺少外环命令(写在 -- 之后)");
    }
    c.project = absolute(&c.project);
    if !Path::new(&c.project).is_dir() {
        return fail(2, format!("项目目录不存在: {}", c.project));
    }
    if c.reset {
        save_state(&c.project, &json!({}));
    }
    match duty_state(&c.project).as_str() {
        "HELD" => {
            let mut record = load_state(&c.project);
            if record.get("busy_seen").is_some() {
                if let Some(fields) = record.as_object_mut() {
                    fields.remove("busy_seen");
                }
                save_state(&c.project, &record);
            }
            println!("held");
            return Ok(0);
        }
        "VACANT" => {}
        other => return fail(1, format!("读不出锁的状态({other})")),
    }
    let mut record = load_state(&c.project);
    let started = now();
    let recent: Vec<f64> = record["restarts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .filter(|t| started - t < c.window as f64)
        .collect();
    if recent.len() >= c.max_restarts {
        if record["paused_noted"] != json!(true) {
            board_note(
                c.board.as_deref(),
                &format!(
                    "> 外环自动重启已暂停({}):{} 分钟内重启了 {} 次,外环一起来就挂,需要人看。{}\n",
                    c.signature,
                    c.window / 60,
                    recent.len(),
                    stamp()
                ),
            );
            record["paused_noted"] = json!(true);
            record["restarts"] = json!(recent);
            save_state(&c.project, &record);
        }
        eprintln!("paused: {} restarts in {}s", recent.len(), c.window);
        return Ok(4);
    }
    let (pane, created) = find_pane(&c)?;
    let busy = busy_programs(&pane, c.session.as_deref())?;
    if !busy.is_empty() && !created {
        eprintln!(
            "{PROG}: 外环窗格 {pane} 里还有程序在跑({}),锁却空着;未重启",
            busy.join(", ")
        );
        // Seen twice in a row it is no passing moment: something runs in the
        // outer's pane that did not come up through `hold` — an agent herdr
        // resumed by itself, or one started by hand. Said once.
        let seen = record["busy_seen"].as_u64().unwrap_or(0) + 1;
        if seen == 2 {
            board_note(
                c.board.as_deref(),
                &format!(
                    "> 外环没有持锁({}):锁空着,窗格 {pane} 里却有程序在跑({}),它不是经 outer-duty hold 启动的;监督任务不会重启它,需要人看。{}\n",
                    c.signature,
                    busy.join(", "),
                    stamp()
                ),
            );
        }
        record["busy_seen"] = json!(seen);
        save_state(&c.project, &record);
        return Ok(3);
    }
    let exe = super::repo::self_exe();
    // Without HERDR_ENV the agent's own herdr integration stays silent in
    // this pane. An integration that reports the agent's state holds the
    // pane in herdr's eyes; herdr then takes that integration's resume
    // command — the bare agent, which holds no lock — and refuses ours.
    let mut hold = vec![
        "env",
        "-u",
        "HERDR_ENV",
        exe.as_str(),
        "outer-duty",
        "hold",
        "--project",
        &c.project,
        "--signature",
        &c.signature,
        "--duties",
        &c.duties,
        "--",
    ];
    hold.extend(c.command.iter().map(String::as_str));
    let line = hold.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
    herdr(&["pane", "run", &pane, &line], c.session.as_deref())?;
    let mut restarts = recent;
    restarts.push(started);
    save_state(
        &c.project,
        &json!({"restarts": restarts, "last_pane": pane}),
    );

    let deadline = Instant::now() + Duration::from_secs(c.wait);
    while Instant::now() < deadline {
        if duty_state(&c.project) == "HELD" && stays_held(&c.project, c.settle) {
            board_note(
                c.board.as_deref(),
                &format!(
                    "> 外环已自动重启({}):锁空出,已在 herdr 窗格 {pane} 重新 hold。{}\n",
                    c.signature,
                    stamp()
                ),
            );
            println!("restarted in pane {pane}");
            register_resume(&c, &pane, &hold);
            deliver_prompt(&c, &pane);
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    fail(
        1,
        format!(
            "已在窗格 {pane} 执行 hold,{} 秒内锁仍是空的(或拿到后又放掉了)",
            c.wait
        ),
    )
}

fn render_brief(b: &Value) -> String {
    let word = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let mut out = vec![
        format!("外环接班简报({})", word(&b["time"])),
        format!("  黑板 {}", word(&b["board"])),
        format!("  项目 {}", word(&b["project"])),
    ];
    let who = if b["holder"] == json!(true) {
        "本会话持锁,可以接班"
    } else {
        "本会话不是持锁者:只能批注,不要接班"
    };
    out.push(format!(
        "锁:{};{who}({})",
        word(&b["lock"]),
        word(&b["holder_detail"])
    ));
    let restarts: Vec<String> = b["restarts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(word)
        .collect();
    if !restarts.is_empty() {
        out.push(format!("最近的自动重启:{}", restarts.join(", ")));
    }
    let notes: Vec<String> = b["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(word)
        .collect();
    if !notes.is_empty() {
        out.push("监督任务的留言:".to_string());
        out.extend(notes.iter().map(|n| format!("  {n}")));
    }
    let s = &b["summary"];
    if let Some(why) = s.get("unavailable").and_then(Value::as_str) {
        out.push(format!("汇总读不到:{why}"));
    } else {
        let list = |key: &str| s[key].as_array().cloned().unwrap_or_default();
        let inbox: Vec<Value> = list("cards")
            .into_iter()
            .filter(|c| c["partition"] == "inbox")
            .collect();
        out.push(format!(
            "收件箱 {} 张(停机期间到的回执与待办):",
            inbox.len()
        ));
        for c in inbox.iter().take(20) {
            out.push(format!(
                "  #{} {} — {}",
                c["number"],
                word(&c["title"]),
                word(&c["reason"])
            ));
        }
        let questions = list("questions");
        if !questions.is_empty() {
            out.push(format!("待回答的提问 {} 个:", questions.len()));
            for q in questions.iter().take(10) {
                let text: String = word(&q["question"]).chars().take(100).collect();
                out.push(format!("  [{}] {text}", word(&q["unit"])));
            }
        }
        let alerts = list("alerts");
        if !alerts.is_empty() {
            out.push(format!("负哨 {} 条(最近):", alerts.len()));
            for a in alerts.iter().skip(alerts.len().saturating_sub(5)) {
                out.push(format!(
                    "  #{} [{}] {}",
                    a["card"],
                    word(&a["unit"]),
                    word(&a["alert"])
                ));
            }
        }
        let units: Vec<String> = list("units")
            .iter()
            .map(|u| format!("{} {}", word(&u["unit"]), word(&u["state"])))
            .collect();
        if !units.is_empty() {
            out.push(format!("单元:{}", units.join(";")));
        }
    }
    out.push("接班检查单(逐项做完再干活):".to_string());
    for (i, item) in b["checklist"].as_array().into_iter().flatten().enumerate() {
        out.push(format!("  {}. {}", i + 1, word(item)));
    }
    out.join("\n")
}

fn resume(c: Common) -> Result<i32, Fail> {
    let project = absolute(&c.project);
    let board = c.board.clone().unwrap_or_default();
    if !Path::new(&board).is_file() && !Path::new(&board).is_dir() {
        return fail(2, format!("黑板不存在: {board}"));
    }
    let board = absolute(&board);
    let lock = duty_state(&project);
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let (holder, detail) = if env("OLP_DUTY_LEASE").is_some() {
        let said = Command::new(this_exe()?)
            .args(["outer-duty", "lease", "--project", &project])
            .output()
            .map_err(|e| Fail(1, e.to_string()))?;
        let text = format!(
            "{} {}",
            String::from_utf8_lossy(&said.stdout).trim(),
            String::from_utf8_lossy(&said.stderr).trim()
        );
        (said.status.success(), text.trim().to_string())
    } else {
        (
            lock == "HELD" && env("OLP_DUTY_PROJECT").is_some(),
            "环境里没有 OLP_DUTY_LEASE(不是经 hold 启动,或版本较旧)".to_string(),
        )
    };
    let record = load_state(&project);
    let restarts: Vec<String> = record["restarts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .filter_map(|t| {
            chrono::DateTime::from_timestamp(t as i64, 0).map(|at| {
                at.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
        })
        .collect();
    let restarts = &restarts[restarts.len().saturating_sub(5)..];
    let mut summary_cmd = Command::new(this_exe()?);
    summary_cmd.args(["olp", "board", "summary", "--board", &board, "--json"]);
    if let Some(repo) = &c.repo {
        summary_cmd.args(["--repo", repo]);
    }
    let summary = match summary_cmd.output() {
        Ok(out) => serde_json::from_slice::<Value>(&out.stdout).unwrap_or_else(|_| {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            json!({"unavailable": if err.is_empty() {
                format!("olp summary 退出码 {}", out.status.code().unwrap_or(-1))
            } else { err }})
        }),
        Err(e) => json!({"unavailable": e.to_string()}),
    };
    let sig = if c.signature.is_empty() {
        "<署名>"
    } else {
        &c.signature
    };
    let quoted = quote(&board);
    let is_log = Path::new(&board).is_dir();
    // What the supervisor left on an event log, newest last.
    let notes: Vec<String> = if is_log {
        let all = super::log::Log::at(&board).read().unwrap_or_default();
        let notes: Vec<String> = all
            .iter()
            .filter(|e| e.kind == "note")
            .map(|e| {
                format!(
                    "{} {}",
                    e.ts,
                    e.data.get("text").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect();
        notes[notes.len().saturating_sub(5)..].to_vec()
    } else {
        Vec::new()
    };
    let (watch, sign) = if is_log {
        (
            format!(
                "确认外环值守已注册(日志有新事件时它往本窗格发一句短句):octoscode olp keepalive list 里应有 …-watch 任务(由 olp outer install 注册),有就不要自己再启动;没有时告诉 operator,临时可跑 octoscode olp outer watch {quoted} --pane <本窗格>"
            ),
            "接班不用另记一行:采认、打回、搁置各自写进事件日志".to_string(),
        )
    } else {
        (
            format!(
                "重新挂正哨(基线从现在起):octoscode olp board watch {quoted} 'ACK(' --skip-signature '{sig}'"
            ),
            format!("在黑板上署名记一行接班:> 外环({sig})·接班:<时间>,收件箱 N 张已处理"),
        )
    };
    let brief = json!({
        "time": stamp(),
        "board": board,
        "project": project,
        "lock": lock,
        "holder": holder,
        "holder_detail": detail,
        "restarts": restarts,
        "summary": summary,
        "checklist": [
            format!("处理收件箱(停机期间到的回执都在这里):octoscode olp inbox {quoted};逐张写采认/打回/搁置"),
            "回答积压的提问:octoscode olp ask list <单元> → octoscode olp ask answer <单元> <id>",
            watch,
            "负哨:单元上的守护各自在跑;hub 上看本简报的负哨与 octoscode olp summary",
            "恢复循环:/loop list 取 id,再 /loop resume <id>(用 /loop 时)",
            sign,
        ],
        "notes": notes,
    });
    if c.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&brief).unwrap_or_else(|_| brief.to_string())
        );
    } else {
        println!("{}", render_brief(&brief));
    }
    Ok(if holder { 0 } else { 3 })
}

fn install(c: Common) -> Result<i32, Fail> {
    if c.command.is_empty() {
        return fail(2, "缺少外环命令(写在 -- 之后)");
    }
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return fail(2, "常驻任务只支持 macOS(launchd)与 Linux(systemd)");
    }
    let exe = super::repo::self_exe();
    let mut job: Vec<String> = [
        exe.as_str(),
        "olp",
        "outer",
        "supervise",
        "--project",
        &absolute(&c.project),
        "--signature",
        &c.signature,
        "--duties",
        &c.duties,
        "--label",
        &c.label,
    ]
    .iter()
    .map(|w| w.to_string())
    .collect();
    for (flag, value, is_path) in [
        ("--cwd", &c.cwd, true),
        ("--session", &c.session, false),
        ("--board", &c.board, true),
    ] {
        if let Some(value) = value {
            job.push(flag.to_string());
            job.push(if is_path {
                absolute(value)
            } else {
                value.clone()
            });
        }
    }
    if c.no_prompt {
        job.push("--no-prompt".to_string());
    } else if let Some(prompt) = &c.prompt {
        job.extend(["--prompt".to_string(), prompt.clone()]);
    }
    for (flag, value) in [
        ("--ready", c.ready.to_string()),
        ("--max-restarts", c.max_restarts.to_string()),
        ("--window", c.window.to_string()),
        ("--wait", c.wait.to_string()),
        ("--settle", c.settle.to_string()),
    ] {
        job.extend([flag.to_string(), value]);
    }
    job.push("--".to_string());
    job.extend(c.command.iter().cloned());

    // A job's PATH is short: record where herdr is.
    let herdr_path = std::env::var_os("OLP_HERDR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|dir| dir.join("herdr"))
                .find(|p| p.is_file())
        })
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path));
    if herdr_path.is_none() {
        eprintln!("{PROG}: 提醒:PATH 上找不到 herdr;装好后重跑,或设 OLP_HERDR");
    }
    let register = |name: &str, job: &[String]| -> Result<i32, Fail> {
        let mut tool = Command::new(this_exe()?);
        tool.args([
            "olp",
            "keepalive",
            "install",
            name,
            "--interval",
            &c.interval.to_string(),
        ]);
        if let Some(path) = &herdr_path {
            tool.args(["--env", &format!("OLP_HERDR={}", path.display())]);
        }
        if c.no_load {
            tool.arg("--no-load");
        }
        if let Some(dir) = &c.dir {
            tool.args(["--dir", dir]);
        }
        let status = tool
            .arg("--")
            .args(job)
            .status()
            .map_err(|e| Fail(1, format!("无法运行保活工具: {e}")))?;
        Ok(status.code().unwrap_or(1))
    };
    let code = register(&c.name, &job)?;
    // On an event log the outer's watcher is a job too: one the outer loop
    // starts itself outlives it, and the next outer loop would start another.
    let log = c.board.as_deref().filter(|b| Path::new(b).is_dir());
    let (Some(log), 0) = (log, code) else {
        return Ok(code);
    };
    let mut watch: Vec<String> = [
        exe.as_str(),
        "olp",
        "outer",
        "watch",
        &absolute(log),
        "--label",
        &c.label,
        "--once",
    ]
    .iter()
    .map(|w| w.to_string())
    .collect();
    if let Some(session) = &c.session {
        watch.extend(["--session".to_string(), session.clone()]);
    }
    register(&format!("{}-watch", c.name), &watch)
}

/// `argv` is what follows `olp outer`.
pub fn run(argv: &[String]) -> i32 {
    let usage = "用法: octoscode olp outer supervise --project P --signature S --duties D [--label olp-outer] [--cwd 目录] [--session 会话] [--board 黑板] [--max-restarts 3] [--window 1800] [--wait 20] [--settle 2] [--ready 90] [--prompt 话 | --no-prompt] [--reset] -- <外环命令…>\n      octoscode olp outer resume --board B --project P [--repo R] [--signature S] [--json]\n      octoscode olp outer watch <日志目录> --pane <窗格> | --label <工作区标签> [--once]   (外环值守,另见它的 --help)\n      octoscode olp outer install [--interval 60] [--name outer] [--no-load] [--dir 目录] <supervise 的参数> -- <外环命令…>   (--board 是事件日志时另注册 <name>-watch:外环值守)\n退出码:0 锁 HELD(resume:本会话持锁);1 出错;2 用法;3 外环窗格里还有程序在跑(resume:不是持锁者);4 重启过于频繁,已暂停";
    let (action, rest) = match argv.split_first() {
        Some((a, rest)) if !matches!(a.as_str(), "-h" | "--help") => (a.as_str(), rest),
        Some(_) => {
            println!("{usage}");
            return 0;
        }
        None => {
            eprintln!("{usage}");
            return 2;
        }
    };
    let before_command = rest.iter().take_while(|w| *w != "--");
    if action != "watch" && before_command.clone().any(|w| w == "-h" || w == "--help") {
        println!("{usage}");
        return 0;
    }
    let result = match action {
        "supervise" => parse(action, rest).and_then(supervise),
        "resume" => parse(action, rest).and_then(resume),
        "install" => parse(action, rest).and_then(install),
        "watch" => return super::outerwatch::run(rest),
        other => fail(2, format!("没有这个动作: {other}\n{usage}")),
    };
    match result {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            eprintln!("{PROG}: {why}");
            code
        }
    }
}
