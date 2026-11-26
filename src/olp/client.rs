//! The unit's side of the event log: `octoscode olp remote ack | progress |
//! mirror` when the unit's configuration says its hub keeps an event log
//! (`store = log` in `unit.conf`, or `OLP_STORE=log`). Each sends one
//! request to the hub's forced command (`remote serve-log`), which decides
//! what this unit may do; `hub = local` runs that same command here.
//!
//! Configuration, as for the Markdown channel: `--hub` / `OLP_HUB` /
//! `hub=`, `--unit` / `OLP_UNIT_ID` / `unit=`, in
//! `${OLP_UNIT_CONF:-~/.octos/outer/unit.conf}`; with `hub = local` also
//! `OLP_LOG` / `log=` (the log directory); for `mirror`, `--to` /
//! `OLP_MIRROR` / `mirror=` (the Markdown file agents read).
//!
//! Exit codes: 0 done; 2 usage or configuration; 3 refused by the hub;
//! 4 the hub could not read or write its log; 75 the hub was not reached
//! (the same command again is safe: ACKs and progress carry a key).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use super::log::Event;

const EX_TEMPFAIL: i32 = 75;
const LOCAL: &str = "local";

pub(super) fn conf() -> BTreeMap<String, String> {
    let path = std::env::var_os("OLP_UNIT_CONF")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| Path::new(&home).join(".octos/outer/unit.conf"))
        });
    let mut out = BTreeMap::new();
    let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            out.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    out
}

pub(super) fn setting(
    flag: Option<&String>,
    env: &str,
    conf: &BTreeMap<String, String>,
    key: &str,
) -> String {
    flag.cloned()
        .or_else(|| std::env::var(env).ok().filter(|v| !v.is_empty()))
        .or_else(|| conf.get(key).cloned())
        .unwrap_or_default()
}

/// Does this unit's hub keep an event log? Otherwise the Markdown channel
/// (`scripts/olp-board-remote.py`) handles the command.
pub fn log_mode() -> bool {
    setting(None, "OLP_STORE", &conf(), "store") == "log"
}

struct Link {
    hub: String,
    unit: String,
    /// The log directory, when the hub is this machine.
    log: String,
}

fn is_unit(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_hub(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || "._@-".contains(c))
}

fn link(hub: Option<&String>, unit: Option<&String>) -> Result<Link, String> {
    let conf = conf();
    let hub = setting(hub, "OLP_HUB", &conf, "hub");
    let unit = setting(unit, "OLP_UNIT_ID", &conf, "unit");
    if !is_hub(&hub) {
        return Err("没有配置 hub 的 ssh 目标(--hub、OLP_HUB 或 unit.conf 里的 hub=)".to_string());
    }
    if !is_unit(&unit) {
        return Err("没有配置本单元号(--unit、OLP_UNIT_ID 或 unit.conf 里的 unit=)".to_string());
    }
    let log = setting(None, "OLP_LOG", &conf, "log");
    if hub == LOCAL && !Path::new(&log).is_absolute() {
        return Err(
            "hub = local 时要给日志目录的绝对路径(OLP_LOG 或 unit.conf 里的 log=)".to_string(),
        );
    }
    Ok(Link { hub, unit, log })
}

struct Reply {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

/// One request to the hub side. `None`: the hub was not reached.
fn send(link: &Link, request: &[String], stdin: &str) -> Option<Reply> {
    let words: Vec<&str> = request.iter().map(String::as_str).collect();
    let line = shlex::try_join(words).ok()?;
    let mut cmd = if link.hub == LOCAL {
        // The forced command, run here: the same handler, the same rules.
        let mut cmd = Command::new(std::env::current_exe().ok()?);
        cmd.args([
            "olp",
            "remote",
            "serve-log",
            "--unit",
            &link.unit,
            "--log",
            &link.log,
        ])
        .env("SSH_ORIGINAL_COMMAND", &line);
        cmd
    } else {
        let ssh = std::env::var("OLP_SSH").ok().filter(|v| !v.is_empty());
        let mut cmd = Command::new(ssh.as_deref().unwrap_or("ssh"));
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            &link.hub,
            &line,
        ]);
        cmd
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    // A request refused before stdin is read closes the pipe first.
    let _ = child.stdin.take()?.write_all(stdin.as_bytes());
    let out = child.wait_with_output().ok()?;
    let code = out.status.code()?;
    // ssh's own failures (no route, refused, bad key) are 255.
    if link.hub != LOCAL && code == 255 {
        return None;
    }
    Some(Reply {
        code,
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    })
}

/// Flags every command here takes, and the words left over.
struct Words {
    hub: Option<String>,
    unit: Option<String>,
    to: Option<String>,
    rest: Vec<String>,
    help: bool,
}

fn words(argv: &[String]) -> Result<Words, String> {
    let mut out = Words {
        hub: None,
        unit: None,
        to: None,
        rest: Vec::new(),
        help: false,
    };
    let mut it = argv.iter();
    while let Some(word) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{word} 需要一个值"))
        };
        match word.as_str() {
            "-h" | "--help" => out.help = true,
            "--hub" => out.hub = Some(value()?),
            "--unit" => out.unit = Some(value()?),
            "--to" => out.to = Some(value()?),
            other if other.starts_with("--") => return Err(format!("不认识的参数: {other}")),
            other => out.rest.push(other.to_string()),
        }
    }
    Ok(out)
}

fn deliver(prog: &str, link: &Link, request: &[String], text: &str) -> i32 {
    match send(link, request, text) {
        None => {
            eprintln!("{prog}: 没有连上 hub,未送达;原样重跑本命令即可(不会重复记录)");
            EX_TEMPFAIL
        }
        Some(reply) => {
            print!("{}", String::from_utf8_lossy(&reply.stdout));
            if !reply.stderr.is_empty() {
                eprintln!("{}", reply.stderr);
            }
            reply.code
        }
    }
}

const ACK_USAGE: &str = "用法: octoscode olp remote ack <N> <done|wontdo|blocked> <说明…>";
const PROGRESS_USAGE: &str = "用法: octoscode olp remote progress <N> claimed | review <轮次> | rework <轮次> [说明…]\n      (另有 undelivered:值守进程报告窗格没有接住卡,一般不用手工发)";
const MIRROR_USAGE: &str = "用法: octoscode olp remote mirror [--to <Markdown 文件>]";

/// `olp remote ack` on the event-log channel.
pub fn ack(argv: &[String]) -> i32 {
    let prog = "olp remote ack";
    let w = match words(argv) {
        Ok(w) if w.help => {
            println!("{ACK_USAGE}");
            return 0;
        }
        Ok(w) => w,
        Err(why) => {
            eprintln!("{prog}: {why}\n{ACK_USAGE}");
            return 2;
        }
    };
    let [n, result, text @ ..] = w.rest.as_slice() else {
        eprintln!("{ACK_USAGE}");
        return 2;
    };
    let text = text.join(" ");
    if text.trim().is_empty() {
        eprintln!("{prog}: ACK 需要说明(done 附 commit/测试证据,wontdo 附理由,blocked 附原因)");
        return 2;
    }
    let link = match link(w.hub.as_ref(), w.unit.as_ref()) {
        Ok(link) => link,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    // The same ACK sent twice (a retry after a lost reply) is recorded once.
    let digest = Sha256::digest(format!("{n}\n{result}\n{}", text.trim()).as_bytes());
    let tag: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let key = format!("{}:ack:{n}:{tag}", link.unit);
    deliver(
        prog,
        &link,
        &["ack".into(), n.clone(), result.clone(), key],
        &text,
    )
}

/// `olp remote progress`: a progress node of one of this unit's cards.
pub fn progress(argv: &[String]) -> i32 {
    let prog = "olp remote progress";
    let w = match words(argv) {
        Ok(w) if w.help => {
            println!("{PROGRESS_USAGE}");
            return 0;
        }
        Ok(w) => w,
        Err(why) => {
            eprintln!("{prog}: {why}\n{PROGRESS_USAGE}");
            return 2;
        }
    };
    if !log_mode() {
        eprintln!(
            "{prog}: 进度上报需要事件日志(unit.conf 里 store = log);本单元的 hub 还在用 Markdown 黑板"
        );
        return 2;
    }
    let (request, text) = match w.rest.as_slice() {
        [n, node, text @ ..] if node == "claimed" || node == "undelivered" => {
            (vec![n.clone(), node.clone()], text.join(" "))
        }
        [n, node, round, text @ ..] if node == "review" || node == "rework" => {
            (vec![n.clone(), node.clone(), round.clone()], text.join(" "))
        }
        _ => {
            eprintln!("{PROGRESS_USAGE}");
            return 2;
        }
    };
    let link = match link(w.hub.as_ref(), w.unit.as_ref()) {
        Ok(link) => link,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    let mut full = vec!["progress".to_string()];
    full.extend(request);
    deliver(prog, &link, &full, &text)
}

const QUESTION_USAGE: &str = "用法: octoscode olp remote question ask <提问id> <问题…> | answer <提问id> | timeout <提问id>\n      (内环用 ask_outer 提问时由提问服务自动上报,一般不用手工发)";

/// `olp remote question`: tell the hub's log that an inner loop asked the
/// outer loop something, or how that question ended.
pub fn question(argv: &[String]) -> i32 {
    let prog = "olp remote question";
    let w = match words(argv) {
        Ok(w) if w.help => {
            println!("{QUESTION_USAGE}");
            return 0;
        }
        Ok(w) => w,
        Err(why) => {
            eprintln!("{prog}: {why}\n{QUESTION_USAGE}");
            return 2;
        }
    };
    if !needs_log(prog) {
        return 2;
    }
    let (request, text) = match w.rest.as_slice() {
        [kind, id, text @ ..] if kind == "ask" && !text.is_empty() => {
            (vec![kind.clone(), id.clone()], text.join(" "))
        }
        [kind, id] if kind == "answer" || kind == "timeout" => {
            (vec![kind.clone(), id.clone()], String::new())
        }
        _ => {
            eprintln!("{QUESTION_USAGE}");
            return 2;
        }
    };
    let link = match link(w.hub.as_ref(), w.unit.as_ref()) {
        Ok(link) => link,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    let mut full = vec!["question".to_string()];
    full.extend(request);
    // One line travels: a question's line breaks become spaces.
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    deliver(prog, &link, &full, &text)
}

/// `olp remote mirror` on the event-log channel: fetch what is new about
/// this unit's cards and rewrite the Markdown file agents read. The events
/// are kept beside it (`<file>.events.jsonl`), so each run asks only for
/// what came after the last one.
pub fn mirror(argv: &[String]) -> i32 {
    let prog = "olp remote mirror";
    let w = match words(argv) {
        Ok(w) if w.help => {
            println!("{MIRROR_USAGE}");
            return 0;
        }
        Ok(w) => w,
        Err(why) => {
            eprintln!("{prog}: {why}\n{MIRROR_USAGE}");
            return 2;
        }
    };
    let target = setting(w.to.as_ref(), "OLP_MIRROR", &conf(), "mirror");
    if target.is_empty() {
        eprintln!("{prog}: 没有镜像路径(--to、OLP_MIRROR 或 unit.conf 里的 mirror=)");
        return 2;
    }
    let link = match link(w.hub.as_ref(), w.unit.as_ref()) {
        Ok(link) => link,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    let target = PathBuf::from(target);
    let cache = PathBuf::from(format!("{}.events.jsonl", target.display()));
    let parse = |text: &str| -> Result<Vec<Event>, String> {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(|e| e.to_string()))
            .collect()
    };
    let mut events = match std::fs::read_to_string(&cache) {
        Ok(text) => match parse(&text) {
            Ok(events) => events,
            Err(why) => {
                eprintln!(
                    "{prog}: 本机的事件缓存 {} 读不了({why});删掉它再同步",
                    cache.display()
                );
                return 2;
            }
        },
        Err(_) => Vec::new(),
    };
    let since = events.last().map_or(0, |e| e.seq);
    let Some(reply) = send(&link, &["events".to_string(), since.to_string()], "") else {
        eprintln!("{prog}: 没有连上 hub,镜像没有更新(还是上次同步的内容)");
        return EX_TEMPFAIL;
    };
    if reply.code != 0 {
        eprintln!("{}", reply.stderr);
        return reply.code;
    }
    let fresh = match parse(&String::from_utf8_lossy(&reply.stdout)) {
        Ok(fresh) => fresh,
        Err(why) => {
            eprintln!("{prog}: hub 返回的事件无法解析: {why}");
            return 4;
        }
    };
    let added = fresh.len();
    events.extend(fresh);
    let lines: String = events
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|line| line + "\n")
        .collect();
    let view = super::render::render(&events);
    // Each file is replaced whole: a reader never sees half of one.
    for (path, content) in [(&cache, lines), (&target, view)] {
        if let Err(e) = replace(path, &content) {
            eprintln!("{prog}: 写不了 {}: {e}", path.display());
            return 4;
        }
    }
    println!(
        "已同步:新增 {added} 条事件,共 {} 条 → {}",
        events.len(),
        target.display()
    );
    0
}

fn replace(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// What the watcher remembers between rounds and across restarts.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct WatchState {
    /// When each waiting card was last nudged; negative when the agent did
    /// not pick that nudge up.
    nudged: BTreeMap<String, i64>,
    /// Nudges in a row that were not picked up.
    misses: u64,
    /// Since when the pane has not been able to take a card, and why.
    stuck: Option<(u64, String)>,
    /// Cards already reported to the hub as undelivered.
    reported: std::collections::BTreeSet<String>,
    /// How many nudges the agent started working on, per card still unclaimed.
    picked: BTreeMap<String, u32>,
    /// Follow-ups on claimed cards without a receipt: the card's last event
    /// when it was followed up, how many times since, and when.
    followed: BTreeMap<String, (u64, u32, u64)>,
}

/// What became of the wake sentence.
enum Prompted {
    /// herdr typed it and saw the agent start (working, or blocked on an
    /// approval of its own).
    Started,
    /// herdr typed it and the agent did not start.
    Stalled,
    /// herdr did not type it (the agent is blocked or gone, no server).
    Refused,
}

/// Types the wake sentence and waits, as herdr does it, for the agent to
/// start: `agent prompt --wait` fails with `agent_prompt_stalled` when no
/// working or blocked state follows the submission, and with
/// `agent_blocked` — before typing anything — when the agent is blocked.
fn prompt(pane: &str, sentence: &str) -> Prompted {
    let exe = std::env::var("OLP_HERDR").ok().filter(|v| !v.is_empty());
    let out = Command::new(exe.as_deref().unwrap_or("herdr"))
        .args(["agent", "prompt", pane, sentence])
        .args(["--wait", "--until", "working", "--until", "blocked"])
        .args(["--timeout", "10000"])
        .stdin(Stdio::null())
        .output();
    let Ok(out) = out else {
        return Prompted::Refused;
    };
    if out.status.success() {
        return Prompted::Started;
    }
    // herdr writes its error object to stderr.
    let code = [&out.stderr, &out.stdout]
        .into_iter()
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .find_map(|v| v["error"]["code"].as_str().map(str::to_string))
        .unwrap_or_default();
    match code.as_str() {
        // Refused before a key was sent.
        "agent_blocked" | "agent_not_ready" | "agent_not_found" | "server_not_running" => {
            Prompted::Refused
        }
        // Anything else may have typed: never assume the prompt is clean.
        _ => Prompted::Stalled,
    }
}

/// herdr's word for the agent in a pane: `idle`, `working`, `blocked`, …;
/// `unknown` when herdr cannot say (no agent there, no server).
fn agent_status(pane: &str) -> String {
    let exe = std::env::var("OLP_HERDR").ok().filter(|v| !v.is_empty());
    Command::new(exe.as_deref().unwrap_or("herdr"))
        .args(["agent", "get", pane])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
        .and_then(|v| {
            v["result"]["agent"]["agent_status"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}
const INBOX_USAGE: &str = "用法: octoscode olp remote inbox [<窗格>]";
const WATCH_USAGE: &str = "用法: octoscode olp remote watch <窗格> [--once] [--interval 秒] [--renudge 秒] [--retry 秒] [--follow-up 秒] [--idle-wait 秒]";

fn needs_log(prog: &str) -> bool {
    if log_mode() {
        return true;
    }
    eprintln!("{prog}: 需要事件日志(unit.conf 里 store = log);本单元的 hub 还在用 Markdown 黑板");
    false
}

/// The cards waiting for this unit, as the hub lists them.
fn pending(link: &Link, pane: Option<&str>) -> Result<serde_json::Value, (i32, String)> {
    let mut request = vec!["inbox".to_string()];
    request.extend(pane.map(str::to_string));
    let reply = send(link, &request, "").ok_or((EX_TEMPFAIL, "没有连上 hub".to_string()))?;
    if reply.code != 0 {
        return Err((reply.code, reply.stderr));
    }
    serde_json::from_slice(&reply.stdout).map_err(|e| (4, format!("hub 的回答无法解析: {e}")))
}

/// `olp remote inbox [<窗格>]`: the cards dispatched to this unit that it
/// has not claimed, as JSON. Claim one with `remote progress <N> claimed`.
pub fn inbox(argv: &[String]) -> i32 {
    let prog = "olp remote inbox";
    let w = match words(argv) {
        Ok(w) if w.help => {
            println!(
                "{INBOX_USAGE}\n\n列出派给本单元、还没领取的卡(JSON):body 是卡的全文,next 是接下来的命令(领取、回执)"
            );
            return 0;
        }
        Ok(w) if w.rest.len() <= 1 => w,
        Ok(_) => {
            eprintln!("{INBOX_USAGE}");
            return 2;
        }
        Err(why) => {
            eprintln!("{prog}: {why}\n{INBOX_USAGE}");
            return 2;
        }
    };
    if !needs_log(prog) {
        return 2;
    }
    let link = match link(w.hub.as_ref(), w.unit.as_ref()) {
        Ok(link) => link,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    match pending(&link, w.rest.first().map(String::as_str)) {
        Ok(mut report) => {
            with_next_steps(&mut report);
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string())
            );
            0
        }
        Err((code, why)) => {
            eprintln!("{prog}: {why}");
            code
        }
    }
}

/// Says, on each waiting card, the commands that come next, so an agent
/// that was only told to look here can go on without guessing: claim, do
/// what `body` says, then ACK. A hub too old to send the body is read
/// through the mirror instead.
fn with_next_steps(report: &mut serde_json::Value) {
    let Some(items) = report.get_mut("items").and_then(|v| v.as_array_mut()) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    let mut hosted = false;
    for item in items.iter_mut() {
        let Some(n) = item.get("number").and_then(|v| v.as_u64()) else {
            continue;
        };
        let mut next = serde_json::json!({
            "claim": format!("octoscode olp remote progress {n} claimed"),
            "ack": format!("octoscode olp remote ack {n} done|wontdo|blocked <说明>"),
        });
        if item.get("body").is_none() {
            next["read"] = serde_json::json!("octoscode olp remote mirror");
        }
        // A card that names who implements and who reviews is not for the
        // woken agent to do itself: it hosts the unit's inner loops.
        let implementer = item.get("impl").and_then(|v| v.as_str()).unwrap_or("");
        let review: Vec<&str> = item
            .get("review")
            .and_then(|v| v.as_array())
            .map(|names| names.iter().filter_map(|n| n.as_str()).collect())
            .unwrap_or_default();
        if !implementer.is_empty() || !review.is_empty() {
            let named = |who: &str| {
                if who.is_empty() {
                    "由你定".to_string()
                } else {
                    who.to_string()
                }
            };
            next["host"] = serde_json::json!(format!(
                "这张卡指定了实现方 {}、复验方 {}:你是单元主持,不要自己实现。按 $(cat ~/.octos/outer/octoscode-home)/skills/octoloop/AGENTS.md 的「模式 unit」组织实现与对抗复验;全部复验方通过后执行 octoscode olp repo push {n} --verified-by <单元号>-<复验方>+<复验方>,再回执",
                named(implementer),
                named(&review.join("、")),
            ));
            hosted = true;
        }
        item["next"] = next;
    }
    let host = if hosted {
        ";带 next.host 的卡由你主持而不是由你实现,照 next.host 办"
    } else {
        ""
    };
    let avoid = if items.iter().any(|item| item.get("avoid").is_some()) {
        ";avoid 列出的东西不要碰,非碰不可就回执 blocked 说明"
    } else {
        ""
    };
    report["how"] = serde_json::json!(format!(
        "每张卡:先执行 next.claim 领取,按 body(卡的全文)办理,做完执行 next.ack 回执——done 附 commit/测试证据,wontdo 附理由,blocked 附原因{avoid}{host}"
    ));
}

/// The one sentence a pane is woken with. It carries no card content: the
/// inbox is where the card is, this only says to look.
pub fn wake_sentence(pane: &str) -> String {
    format!("有新派卡,请执行:octoscode olp remote inbox {pane},再按输出里的 next 领取、办理、回执")
}

/// What the watcher types into a pane that sits idle on claimed cards.
/// Fixed but for the numbers: nothing of the cards' text.
pub fn follow_up_sentence(pane: &str, cards: &[String]) -> String {
    format!(
        "已领取的卡 #{} 还没有回执,而这个窗格已经空闲:请接着办理(卡的全文在 octoscode olp remote inbox {pane} 输出的 held 里);办完或办不了都要回执(octoscode olp remote ack <N> done|blocked <说明>)",
        cards.join("、#")
    )
}

fn herdr(args: &[&str]) -> bool {
    let exe = std::env::var("OLP_HERDR").ok().filter(|v| !v.is_empty());
    Command::new(exe.as_deref().unwrap_or("herdr"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `olp remote watch <窗格>`: the unit's watcher. Not an agent: it asks the
/// hub what waits for this pane and, when something new does and the agent
/// in the pane is idle, types the one fixed sentence. It never reads a
/// card. A card is nudged once, and again after `--renudge` seconds if it
/// is still unclaimed; a busy agent is left alone until the next round.
/// A nudge counts only if the agent then starts working: herdr also calls
/// an agent idle while a dialog covers its prompt, and a sentence typed
/// there is lost. Such a nudge is tried once more after `--retry` seconds;
/// after two in a row the pane is not taking input, and typing into it
/// again only piles sentences up in its prompt, so the watcher falls back
/// to the `--renudge` pace. A pane that cannot take its cards for `--retry`
/// seconds — blocked on a dialog or an approval, no agent in it, or not
/// picking nudges up — is reported to the hub (`progress <N> undelivered`),
/// where the outer loop's inbox shows it: a card is claimed or known to be
/// stuck, never silently waiting.
pub fn watch(argv: &[String]) -> i32 {
    let prog = "olp remote watch";
    let (mut once, mut interval, mut renudge, mut idle_wait) = (false, 15u64, 900u64, 5u64);
    let (mut retry, mut follow_up) = (120u64, 900u64);
    let mut rest = Vec::new();
    let mut it = argv.iter();
    while let Some(word) = it.next() {
        let mut seconds = |slot: &mut u64| match it.next().and_then(|v| v.parse::<u64>().ok()) {
            Some(value) => {
                *slot = value;
                true
            }
            None => false,
        };
        let ok = match word.as_str() {
            "-h" | "--help" => {
                println!(
                    "{WATCH_USAGE}\n\n单元值守:有新派卡且窗格里的 agent 空闲时,往窗格发一句固定短句。不读卡的内容。\n发话后 agent 没有开始工作(例如有对话框挡着)的,--retry 秒后再发一次;连续两次都没接住就不再追着发,\n改按 --renudge 的间隔。\n窗格持续 --retry 秒接不了卡(被对话框或审批挡住、里面没有 agent、发话没人接)时向 hub 报告送达失败,外环的收件箱里会看到。已开始工作而卡仍未领取的,--renudge 秒后再发。\n已领取的卡 --follow-up 秒没有新事件、窗格又空闲(主持停在半路)时,发一句请它接着办并回执;同一张卡在没有新事件期间最多发两次,\n之后由 hub 的停滞判定接手。"
                );
                return 0;
            }
            "--once" => {
                once = true;
                true
            }
            "--interval" => seconds(&mut interval),
            "--renudge" => seconds(&mut renudge),
            "--retry" => seconds(&mut retry),
            "--follow-up" => seconds(&mut follow_up),
            "--idle-wait" => seconds(&mut idle_wait),
            other if other.starts_with("--") => false,
            other => {
                rest.push(other.to_string());
                true
            }
        };
        if !ok {
            eprintln!("{prog}: 参数不对: {word}\n{WATCH_USAGE}");
            return 2;
        }
    }
    let [pane] = rest.as_slice() else {
        eprintln!("{WATCH_USAGE}");
        return 2;
    };
    if !super::card::is_pane(pane) || !needs_log(prog) {
        eprintln!("{WATCH_USAGE}");
        return 2;
    }
    let link = match link(None, None) {
        Ok(link) => link,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    let state_path = std::env::var_os("OLP_WATCH_STATE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                Path::new(&home).join(format!(
                    ".octos/outer/watch/{}-{}.json",
                    link.unit,
                    pane.replace(':', "_")
                ))
            })
        });
    let Some(state_path) = state_path else {
        eprintln!("{prog}: 找不到 HOME,无处记录已提醒过的卡");
        return 2;
    };
    loop {
        let mut st: WatchState = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        match pending(&link, Some(pane)) {
            Err((_, why)) => eprintln!("{prog}: {why}(下一轮再试)"),
            Ok(report) => {
                let waiting: Vec<String> = report["items"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|i| i["number"].as_u64())
                            .map(|n| n.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                // Forget cards that are no longer waiting.
                st.nudged.retain(|card, _| waiting.contains(card));
                st.reported.retain(|card| waiting.contains(card));
                st.picked.retain(|card, _| waiting.contains(card));
                // The agent started on two nudges and the card is still not
                // claimed when the next one is due: it works and gets
                // nowhere (an error, a quota). The hub is told once.
                let abandoned: Vec<(String, u32)> = waiting
                    .iter()
                    .filter(|card| !st.reported.contains(*card))
                    .filter_map(|card| {
                        let times = *st.picked.get(card).filter(|n| **n >= 2)?;
                        let at = st.nudged.get(card).filter(|at| **at > 0)?;
                        (now() >= at.unsigned_abs() + renudge).then(|| (card.clone(), times))
                    })
                    .collect();
                for (card, times) in abandoned {
                    let request = ["progress", card.as_str(), "undelivered"].map(str::to_string);
                    let text = format!(
                        "窗格 {pane}:agent 已 {times} 次开始工作,卡仍未领取(可能出错或额度用完)"
                    );
                    match send(&link, &request, &text) {
                        Some(reply) => {
                            if reply.code == 0 {
                                println!("已向 hub 报告:#{card} 提醒了 {times} 次仍未领取");
                            } else {
                                eprintln!(
                                    "{prog}: hub 没有收下 #{card} 的送达失败报告: {}",
                                    reply.stderr
                                );
                            }
                            st.reported.insert(card);
                        }
                        None => eprintln!("{prog}: 没有连上 hub,送达失败的报告下一轮再发"),
                    }
                }
                if waiting.is_empty() {
                    st.misses = 0;
                    st.stuck = None;
                }
                let due = waiting.iter().any(|card| {
                    st.nudged.get(card).is_none_or(|at| {
                        let after = if *at < 0 && st.misses < 2 {
                            retry
                        } else {
                            renudge
                        };
                        now() >= at.unsigned_abs() + after
                    })
                });
                let timeout = (idle_wait * 1000).to_string();
                // Not idle, or herdr would not type: a working agent is
                // simply busy; anything else cannot take a card.
                let not_ready = |st: &mut WatchState| {
                    let status = agent_status(pane);
                    if status == "working" {
                        if st.misses == 0 {
                            st.stuck = None;
                        }
                    } else if st.stuck.is_none() {
                        println!("窗格 {pane} 现在接不了提醒(agent 状态:{status}),没有往里打字");
                        st.stuck = Some((now(), format!("agent 状态 {status}")));
                    }
                };
                if due {
                    let idle = herdr(&[
                        "agent",
                        "wait",
                        pane,
                        "--until",
                        "idle",
                        "--until",
                        "done",
                        "--timeout",
                        &timeout,
                    ]);
                    match idle.then(|| prompt(pane, &wake_sentence(pane))) {
                        Some(Prompted::Started) => {
                            let at = i64::try_from(now()).unwrap_or(i64::MAX);
                            // One sentence covers everything that waits.
                            for card in &waiting {
                                st.nudged.insert(card.clone(), at);
                                *st.picked.entry(card.clone()).or_default() += 1;
                            }
                            st.misses = 0;
                            st.stuck = None;
                            println!("已提醒窗格 {pane}:{} 张卡待领取", waiting.len());
                        }
                        Some(Prompted::Stalled) => {
                            let at = i64::try_from(now()).unwrap_or(i64::MAX);
                            for card in &waiting {
                                st.nudged.insert(card.clone(), -at);
                            }
                            st.misses += 1;
                            st.stuck
                                .get_or_insert((now(), "发话后 agent 没有开始工作".to_string()));
                            if st.misses < 2 {
                                println!(
                                    "已往窗格 {pane} 发话,但 agent 没有开始工作(可能有对话框挡着输入);{retry} 秒后再发一次"
                                );
                            } else {
                                println!(
                                    "窗格 {pane} 连续 {} 次没有接住提醒,不再追着发(改为每 {renudge} 秒一次)",
                                    st.misses
                                );
                            }
                        }
                        Some(Prompted::Refused) | None => not_ready(&mut st),
                    }
                } else if !waiting.is_empty() {
                    // Nudged and not claimed yet: a pane that got stuck since
                    // (an approval, a dialog, the agent gone) is still the
                    // hub's to know about.
                    let status = agent_status(pane);
                    if matches!(status.as_str(), "idle" | "done" | "working") {
                        if st.misses == 0 {
                            st.stuck = None;
                        }
                    } else {
                        not_ready(&mut st);
                    }
                }
                // A pane that has not taken its cards for `--retry` seconds
                // is the hub's to know about: said once per card.
                if let Some((since, why)) = st.stuck.clone() {
                    if now() >= since + retry {
                        for card in &waiting {
                            if st.reported.contains(card) {
                                continue;
                            }
                            let request =
                                ["progress", card.as_str(), "undelivered"].map(str::to_string);
                            let text = format!("窗格 {pane}:{why}");
                            match send(&link, &request, &text) {
                                Some(reply) if reply.code == 0 => {
                                    println!("已向 hub 报告:#{card} 没有送达窗格 {pane}({why})");
                                    st.reported.insert(card.clone());
                                }
                                Some(reply) => {
                                    eprintln!(
                                        "{prog}: hub 没有收下 #{card} 的送达失败报告: {}",
                                        reply.stderr
                                    );
                                    st.reported.insert(card.clone());
                                }
                                None => eprintln!("{prog}: 没有连上 hub,送达失败的报告下一轮再发"),
                            }
                        }
                    }
                }
                // Claimed, no receipt, nothing new for `--follow-up` seconds
                // and the pane idle: the host stopped half-way. Asked to go
                // on, at most twice while the card stays silent.
                let held: Vec<(String, u64)> = report["held"]
                    .as_array()
                    .map(|cards| {
                        cards
                            .iter()
                            .filter(|c| {
                                c["quiet_seconds"].as_i64().unwrap_or(0)
                                    >= i64::try_from(follow_up).unwrap_or(i64::MAX)
                            })
                            .filter_map(|c| {
                                Some((c["number"].as_u64()?.to_string(), c["last_seq"].as_u64()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                st.followed.retain(|card, (seq, _, _)| {
                    report["held"].as_array().is_some_and(|cards| {
                        cards.iter().any(|c| {
                            c["number"].as_u64().map(|n| n.to_string()).as_ref() == Some(card)
                                && c["last_seq"].as_u64() == Some(*seq)
                        })
                    })
                });
                let behind: Vec<(String, u64)> = held
                    .into_iter()
                    .filter(|(card, _)| {
                        st.followed
                            .get(card)
                            .is_none_or(|(_, times, at)| *times < 2 && now() >= at + follow_up)
                    })
                    .collect();
                if waiting.is_empty()
                    && !behind.is_empty()
                    && matches!(agent_status(pane).as_str(), "idle" | "done")
                {
                    let numbers: Vec<String> =
                        behind.iter().map(|(card, _)| card.clone()).collect();
                    if !matches!(
                        prompt(pane, &follow_up_sentence(pane, &numbers)),
                        Prompted::Refused
                    ) {
                        for (card, seq) in behind {
                            let times = st.followed.get(&card).map_or(0, |(_, times, _)| *times);
                            st.followed.insert(card, (seq, times + 1, now()));
                        }
                        println!(
                            "窗格 {pane} 空闲,已领取的卡 #{} 还没有回执:已请它接着办",
                            numbers.join("、#")
                        );
                    }
                }
                let text = serde_json::to_string(&st).unwrap_or_else(|_| "{}".to_string());
                if let Err(e) = replace(&state_path, &text) {
                    eprintln!("{prog}: 记不下已提醒的卡 {}: {e}", state_path.display());
                }
            }
        }
        if once {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1)));
    }
}
