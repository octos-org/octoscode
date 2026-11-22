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

fn conf() -> BTreeMap<String, String> {
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

fn setting(flag: Option<&String>, env: &str, conf: &BTreeMap<String, String>, key: &str) -> String {
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
const PROGRESS_USAGE: &str =
    "用法: octoscode olp remote progress <N> claimed | review <轮次> | rework <轮次> [说明…]";
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
        [n, node, text @ ..] if node == "claimed" => {
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

const INBOX_USAGE: &str = "用法: octoscode olp remote inbox [<窗格>]";
const WATCH_USAGE: &str = "用法: octoscode olp remote watch <窗格> [--once] [--interval 秒] [--renudge 秒] [--idle-wait 秒]";

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
        item["next"] = next;
    }
    report["how"] = serde_json::json!(
        "每张卡:先执行 next.claim 领取,按 body(卡的全文)办理,做完执行 next.ack 回执——done 附 commit/测试证据,wontdo 附理由,blocked 附原因"
    );
}

/// The one sentence a pane is woken with. It carries no card content: the
/// inbox is where the card is, this only says to look.
pub fn wake_sentence(pane: &str) -> String {
    format!("有新派卡,请执行:octoscode olp remote inbox {pane},再按输出里的 next 领取、办理、回执")
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
pub fn watch(argv: &[String]) -> i32 {
    let prog = "olp remote watch";
    let (mut once, mut interval, mut renudge, mut idle_wait) = (false, 15u64, 900u64, 5u64);
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
                    "{WATCH_USAGE}\n\n单元值守:有新派卡且窗格里的 agent 空闲时,往窗格发一句固定短句。不读卡的内容。"
                );
                return 0;
            }
            "--once" => {
                once = true;
                true
            }
            "--interval" => seconds(&mut interval),
            "--renudge" => seconds(&mut renudge),
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
    // When each card was last nudged, kept across restarts.
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
        let mut nudged: BTreeMap<String, u64> = std::fs::read_to_string(&state_path)
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
                nudged.retain(|card, _| waiting.contains(card));
                let due = waiting
                    .iter()
                    .any(|card| nudged.get(card).is_none_or(|at| now() >= at + renudge));
                let timeout = (idle_wait * 1000).to_string();
                if due
                    && herdr(&[
                        "agent",
                        "wait",
                        pane,
                        "--until",
                        "idle",
                        "--until",
                        "done",
                        "--timeout",
                        &timeout,
                    ])
                    && herdr(&["agent", "prompt", pane, &wake_sentence(pane)])
                {
                    // One sentence covers everything that waits.
                    for card in &waiting {
                        nudged.insert(card.clone(), now());
                    }
                    println!("已提醒窗格 {pane}:{} 张卡待领取", waiting.len());
                }
                let text = serde_json::to_string(&nudged).unwrap_or_else(|_| "{}".to_string());
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
