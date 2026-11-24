//! `octoscode olp unit …` — which machines are units, and whether each can
//! take a card now.
//!
//! On the hub: `register`, `set-state`, `receive`, `status`, `check`,
//! `where`. On a unit: `heartbeat`, which inspects the machine and prints
//! one JSON line for the caller to send (`… heartbeat | ssh hub … receive`).
//!
//! Files (hub), under `${OLP_UNITS_DIR:-~/.octos/outer/units}`:
//! `registry.tsv` (unit, hostname, mac, hostkey, serial, enabled, note) and
//! `heartbeat/<unit>.json` (the last heartbeat plus the hub's `received_at`).
//! Both formats are the ones `scripts/olp-unit.py` wrote, so a hub can
//! switch binaries without re-registering.
//!
//! Exit codes: 0 done; 1 `check` did not pass; 2 usage or input; 3 the
//! heartbeat was refused (unit not registered, or not the unit this
//! connection is allowed to speak for).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const FIELDS: [&str; 7] = [
    "unit", "hostname", "mac", "hostkey", "serial", "enabled", "note",
];
const UNIQUE: [&str; 3] = ["hostname", "mac", "hostkey"];
pub const DEFAULT_STALE: u64 = 180;
const MAX_HEARTBEAT_BYTES: usize = 256 * 1024;
const USAGE_CODE: i32 = 2;
const REJECTED: i32 = 3;
const PERMISSIONS: [&str; 5] = ["full", "sandboxed", "mixed", "none", "unknown"];

const USAGE: &str = "用法: octoscode olp unit <动作>
  hub 上:
    register <单元号> --hostname H --mac M [--hostkey FP] [--serial S] [--note T]
    set-state <单元号> online|offline
    receive [--unit <单元号>]            从 stdin 读一条心跳并落盘
    status [--json] [--stale-after 秒]   汇总每个单元的状态
    check <单元号> [--stale-after 秒]    派卡前的检查:只有“正常”退出 0
    where <单元号>                       输出 local 或 remote
  单元上:
    heartbeat --unit <单元号> [--require agent,disk,full] [--disk-min-free 百分比]
退出码:0 成功;1 check 未通过;2 用法或输入错误;3 心跳被拒绝";

struct Fail(i32, String);

type Row = BTreeMap<&'static str, String>;

fn usage_err<T>(why: impl Into<String>) -> Result<T, Fail> {
    Err(Fail(USAGE_CODE, why.into()))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn units_dir() -> PathBuf {
    env("OLP_UNITS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".octos/outer/units"))
}

fn registry_path() -> PathBuf {
    units_dir().join("registry.tsv")
}

fn heartbeat_path(unit: &str) -> PathBuf {
    units_dir().join("heartbeat").join(format!("{unit}.json"))
}

/// Lowercase letter first, then lowercase letters, digits, hyphens; 32 at most.
pub fn is_unit(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_mac(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts.iter().all(|p| {
            p.len() == 2
                && p.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        })
}

fn norm_host(name: &str) -> String {
    name.trim()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// This machine's name as `hostname` prints it, first label, lowercase.
fn this_host() -> String {
    let named = run_for(Command::new("hostname"), Duration::from_secs(5))
        .ok()
        .map(|ran| ran.stdout)
        .filter(|name| !name.trim().is_empty());
    norm_host(
        &named
            .or_else(|| env("HOSTNAME"))
            .or_else(|| env("COMPUTERNAME"))
            .unwrap_or_default(),
    )
}

fn check_unit_name(unit: &str) -> Result<&str, Fail> {
    if is_unit(unit) {
        Ok(unit)
    } else {
        usage_err(format!(
            "单元号不合法: {unit}(小写字母开头,只含 [a-z0-9-],例如 m07)"
        ))
    }
}

fn load_registry() -> BTreeMap<String, Row> {
    let mut rows = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(registry_path()) else {
        return rows;
    };
    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let row: Row = FIELDS
            .iter()
            .map(|k| (*k, parts.next().unwrap_or_default().to_string()))
            .collect();
        if is_unit(&row["unit"]) {
            rows.insert(row["unit"].clone(), row);
        }
    }
    rows
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".olp-unit-")
        .tempfile_in(dir)?;
    tmp.write_all(text.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn io_fail(what: &str, e: std::io::Error) -> Fail {
    Fail(USAGE_CODE, format!("{what}: {e}"))
}

fn save_registry(rows: &BTreeMap<String, Row>) -> Result<(), Fail> {
    let mut text = String::from("# unit\thostname\tmac\thostkey\tserial\tenabled\tnote\n");
    for row in rows.values() {
        let line: Vec<&str> = FIELDS.iter().map(|k| row[k].as_str()).collect();
        text.push_str(&line.join("\t"));
        text.push('\n');
    }
    write_atomic(&registry_path(), &text).map_err(|e| io_fail("写登记文件失败", e))
}

/// Held while the registry is read, changed and written back.
fn registry_lock() -> Result<std::fs::File, Fail> {
    let dir = units_dir();
    std::fs::create_dir_all(&dir).map_err(|e| io_fail("建不了登记目录", e))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("registry.tsv.lock"))
        .map_err(|e| io_fail("打不开登记锁", e))?;
    fs2::FileExt::lock_exclusive(&file).map_err(|e| io_fail("拿不到登记锁", e))?;
    Ok(file)
}

// --- arguments -----------------------------------------------------------

/// `--name value`, `--name=value`, bare flags and positionals, in any order.
struct Parsed {
    values: BTreeMap<String, String>,
    flags: Vec<String>,
    rest: Vec<String>,
}

fn parse(argv: &[String], valued: &[&str], flags: &[&str]) -> Result<Parsed, Fail> {
    let mut out = Parsed {
        values: BTreeMap::new(),
        flags: Vec::new(),
        rest: Vec::new(),
    };
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        if valued.contains(&name) {
            let value = match inline {
                Some(v) => v,
                None => words
                    .next()
                    .cloned()
                    .ok_or_else(|| Fail(USAGE_CODE, format!("{name} 需要一个值")))?,
            };
            out.values.insert(name.to_string(), value);
        } else if flags.contains(&word.as_str()) {
            out.flags.push(word.clone());
        } else if word.starts_with("--") {
            return usage_err(format!("不认识的参数: {word}"));
        } else {
            out.rest.push(word.clone());
        }
    }
    Ok(out)
}

impl Parsed {
    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn positionals<const N: usize>(&self, names: [&str; N]) -> Result<[&str; N], Fail> {
        if self.rest.len() != N {
            return usage_err(format!("需要 {} 个位置参数:{}", N, names.join(" ")));
        }
        Ok(std::array::from_fn(|i| self.rest[i].as_str()))
    }

    fn stale_after(&self) -> Result<u64, Fail> {
        match self.get("--stale-after") {
            None => Ok(DEFAULT_STALE),
            Some(text) => text
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| Fail(USAGE_CODE, "--stale-after 必须是正整数".to_string())),
        }
    }
}

// --- hub: register / set-state -------------------------------------------

fn clean_field(name: &str, value: &str) -> Result<String, Fail> {
    if value.contains(['\t', '\r', '\n']) {
        return usage_err(format!("{name} 不能含制表符或换行"));
    }
    Ok(value.trim().to_string())
}

fn register(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(
        argv,
        &["--hostname", "--mac", "--hostkey", "--serial", "--note"],
        &[],
    )?;
    let [unit] = args.positionals(["<单元号>"])?;
    let unit = check_unit_name(unit)?.to_string();
    let (Some(hostname), Some(mac)) = (args.get("--hostname"), args.get("--mac")) else {
        return usage_err("register 需要 --hostname 与 --mac");
    };
    let mac_norm = mac.trim().to_lowercase().replace('-', ":");
    if !is_mac(&mac_norm) {
        return usage_err(format!("网卡地址格式不对: {mac}(应为 aa:bb:cc:dd:ee:ff)"));
    }
    let mut row: Row = BTreeMap::new();
    row.insert("unit", unit.clone());
    row.insert("hostname", norm_host(&clean_field("--hostname", hostname)?));
    row.insert("mac", mac_norm);
    for (key, flag) in [
        ("hostkey", "--hostkey"),
        ("serial", "--serial"),
        ("note", "--note"),
    ] {
        row.insert(key, clean_field(flag, args.get(flag).unwrap_or_default())?);
    }
    row.insert("enabled", "yes".to_string());
    if row["hostname"].is_empty() {
        return usage_err("--hostname 不能为空");
    }
    let _lock = registry_lock()?;
    let mut rows = load_registry();
    for (other, existing) in &rows {
        if *other == unit {
            continue;
        }
        for key in UNIQUE {
            if !row[key].is_empty() && row[key] == existing[key] {
                return usage_err(format!(
                    "{key} {} 已登记给单元 {other},一个值只能属于一个单元",
                    row[key]
                ));
            }
        }
    }
    if let Some(existing) = rows.get(&unit) {
        // Registering again keeps an operator's offline mark.
        if !existing["enabled"].is_empty() {
            row.insert("enabled", existing["enabled"].clone());
        }
    }
    println!(
        "registered {unit} hostname={} mac={}",
        row["hostname"], row["mac"]
    );
    rows.insert(unit, row);
    save_registry(&rows)?;
    Ok(0)
}

fn set_state(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv, &[], &[])?;
    let [unit, state] = args.positionals(["<单元号>", "online|offline"])?;
    let unit = check_unit_name(unit)?;
    if !matches!(state, "online" | "offline") {
        return usage_err("状态只能是 online 或 offline");
    }
    let _lock = registry_lock()?;
    let mut rows = load_registry();
    let Some(row) = rows.get_mut(unit) else {
        return usage_err(format!("单元未登记: {unit}"));
    };
    row.insert(
        "enabled",
        if state == "online" { "yes" } else { "no" }.to_string(),
    );
    save_registry(&rows)?;
    println!("{unit} {state}");
    Ok(0)
}

// --- unit: heartbeat -----------------------------------------------------

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

enum RunError {
    Spawn(std::io::Error),
    Timeout,
}

/// Run a command to its end, or kill it at the deadline.
fn run_for(mut cmd: Command, limit: Duration) -> Result<Ran, RunError> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
        }
    };
    Ok(Ran {
        code: status.code(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

fn local_macs() -> Vec<String> {
    let mut macs = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            if entry.file_name() == "lo" {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path().join("address")) else {
                continue;
            };
            let mac = text.trim().to_lowercase();
            if is_mac(&mac) && mac != "00:00:00:00:00:00" {
                macs.insert(mac);
            }
        }
    }
    if macs.is_empty() {
        let ifconfig = which("ifconfig").unwrap_or_else(|| PathBuf::from("/sbin/ifconfig"));
        if ifconfig.exists() {
            let mut cmd = Command::new(ifconfig);
            cmd.arg("-a");
            let out = run_for(cmd, Duration::from_secs(10))
                .map(|r| r.stdout)
                .unwrap_or_default();
            let mut words = out.split_whitespace();
            while let Some(word) = words.next() {
                if word == "ether" {
                    if let Some(mac) = words.next().map(str::to_lowercase) {
                        if is_mac(&mac) {
                            macs.insert(mac);
                        }
                    }
                }
            }
        }
    }
    macs.into_iter().collect()
}

type Check = (bool, String);

fn check_serve() -> Check {
    let mut cmd = Command::new("pgrep");
    cmd.args(["-f", "octos serve"]);
    match run_for(cmd, Duration::from_secs(10)) {
        Err(RunError::Spawn(e)) => (false, format!("pgrep 不可用: {e}")),
        Err(RunError::Timeout) => (false, "pgrep 不可用: 超时".to_string()),
        Ok(ran) => {
            let me = std::process::id().to_string();
            let pids = ran
                .stdout
                .split_whitespace()
                .filter(|p| p.chars().all(|c| c.is_ascii_digit()) && *p != me)
                .count();
            if pids > 0 {
                (true, format!("{pids} 个 octos serve 进程"))
            } else {
                (false, "没有 octos serve 进程".to_string())
            }
        }
    }
}

fn herdr_bin() -> String {
    env("OLP_HERDR").unwrap_or_else(|| "herdr".to_string())
}

/// Where reviewer CLIs are usually installed: a keepalive job's PATH is short.
const TOOL_DIRS: [&str; 5] = [
    ".local/bin",
    "/opt/homebrew/bin",
    "/usr/local/bin",
    ".npm-global/bin",
    ".claude/local",
];
/// kind, env override, status command. Only the yes/no answer is kept:
/// `claude auth status` also prints the account, which never leaves the unit.
const REVIEWERS: [(&str, &str, [&str; 2]); 2] = [
    ("claude", "OLP_CLAUDE", ["auth", "status"]),
    ("codex", "OLP_CODEX", ["login", "status"]),
];

fn find_tool(kind: &str, override_env: &str) -> Option<PathBuf> {
    if let Some(path) = env(override_env) {
        let path = PathBuf::from(path);
        return is_executable(&path).then_some(path);
    }
    which(kind).or_else(|| {
        TOOL_DIRS
            .iter()
            .map(|d| {
                if d.starts_with('/') {
                    PathBuf::from(d).join(kind)
                } else {
                    home().join(d).join(kind)
                }
            })
            .find(|p| is_executable(p))
    })
}

/// Whether each reviewer CLI is installed and logged in. Reported, never a
/// health check: the outer loop reads it when it picks a card's reviewers.
fn reviewer_status() -> Vec<Value> {
    REVIEWERS
        .iter()
        .map(|(kind, override_env, args)| {
            let Some(exe) = find_tool(kind, override_env) else {
                return json!({"kind": kind, "installed": false, "logged_in": null});
            };
            let mut cmd = Command::new(exe);
            cmd.args(args);
            let Ok(ran) = run_for(cmd, Duration::from_secs(20)) else {
                return json!({"kind": kind, "installed": true, "logged_in": null});
            };
            let mut logged_in = ran.code == Some(0);
            if *kind == "claude" {
                if let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(&ran.stdout) {
                    logged_in = doc.get("loggedIn") == Some(&Value::Bool(true));
                }
            }
            json!({"kind": kind, "installed": true, "logged_in": logged_in})
        })
        .collect()
}

/// Run a herdr command that prints JSON: its `result`, or why not.
fn herdr_json(args: &[&str]) -> Result<Value, String> {
    let mut cmd = Command::new(herdr_bin());
    cmd.args(args);
    let ran = match run_for(cmd, Duration::from_secs(10)) {
        Ok(ran) => ran,
        Err(RunError::Spawn(e)) => return Err(format!("无法运行 herdr: {e}")),
        Err(RunError::Timeout) => return Err("无法运行 herdr: 超时".to_string()),
    };
    let text = if ran.stdout.trim().is_empty() {
        "{}"
    } else {
        &ran.stdout
    };
    let or = |fallback: String| {
        let said = ran.stderr.trim();
        cut(if said.is_empty() { &fallback } else { said }, 200)
    };
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return Err(or("herdr 的输出不是 JSON".to_string()));
    };
    let Value::Object(map) = &doc else {
        return Err(or(format!("herdr 退出码 {}", ran.code.unwrap_or(-1))));
    };
    if let Some(err) = map.get("error").filter(|e| !e.is_null()) {
        let said = ["message", "code"]
            .iter()
            .find_map(|k| err.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        return Err(cut(&said, 200));
    }
    if ran.code != Some(0) {
        return Err(or(format!("herdr 退出码 {}", ran.code.unwrap_or(-1))));
    }
    Ok(map.get("result").cloned().unwrap_or_else(|| json!({})))
}

struct Agent {
    kind: String,
    status: String,
    pane: String,
}

/// The inner loops herdr knows on this machine, or why it could not say.
fn inner_agents() -> Result<Vec<Agent>, String> {
    let result = herdr_json(&["agent", "list"])?;
    let word = |a: &Value, key: &str, default: &str| {
        a.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(default)
            .to_string()
    };
    Ok(result
        .get("agents")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|a| a.is_object())
                .map(|a| Agent {
                    kind: word(a, "agent", "unknown"),
                    status: word(a, "agent_status", "unknown"),
                    pane: word(a, "pane_id", ""),
                })
                .collect()
        })
        .unwrap_or_default())
}

fn check_agent(agents: &Result<Vec<Agent>, String>) -> Check {
    match agents {
        Err(why) => (false, format!("读不到 herdr 里的内环:{why}")),
        Ok(list) if list.is_empty() => (false, "herdr 里没有内环".to_string()),
        Ok(list) => {
            let shown: Vec<String> = list
                .iter()
                .map(|a| format!("{}({})", a.kind, a.status))
                .collect();
            (true, format!("{} 个内环:{}", list.len(), shown.join("、")))
        }
    }
}

/// From the launch flags of a Claude Code or Codex process; `None` when the
/// flags say nothing (the agent's config file decides).
fn full_access_flags(kind: &str, argv: &[String]) -> Option<bool> {
    let has = |flag: &str| argv.iter().any(|a| a == flag);
    let value_of = |names: &[&str], long: &str| {
        argv.iter().enumerate().find_map(|(i, a)| {
            if names.contains(&a.as_str()) && i + 1 < argv.len() {
                Some(argv[i + 1].clone())
            } else {
                a.strip_prefix(&format!("{long}=")).map(str::to_string)
            }
        })
    };
    match kind {
        "claude" => {
            if has("--dangerously-skip-permissions") {
                return Some(true);
            }
            value_of(&["--permission-mode"], "--permission-mode").map(|v| v == "bypassPermissions")
        }
        "codex" => {
            if has("--dangerously-bypass-approvals-and-sandbox") || has("--yolo") {
                return Some(true);
            }
            value_of(&["-s", "--sandbox"], "--sandbox").map(|v| v == "danger-full-access")
        }
        _ => None,
    }
}

/// Launch permission of each Claude Code / Codex inner loop, read from the
/// argv of the pane's foreground process.
fn agent_permissions(agents: &[Agent]) -> Vec<(String, String, Option<bool>)> {
    agents
        .iter()
        .filter(|a| matches!(a.kind.as_str(), "claude" | "codex") && !a.pane.is_empty())
        .map(|a| {
            let result = herdr_json(&["pane", "process-info", "--pane", &a.pane]).ok();
            let argv = result
                .as_ref()
                .and_then(|r| r.pointer("/process_info/foreground_processes"))
                .and_then(Value::as_array)
                .and_then(|procs| {
                    procs.iter().find(|p| {
                        ["name", "argv0"]
                            .iter()
                            .any(|k| p.get(k).and_then(Value::as_str) == Some(a.kind.as_str()))
                    })
                })
                .map(|p| {
                    p.get("argv")
                        .and_then(Value::as_array)
                        .map(|argv| {
                            argv.iter()
                                .filter_map(|w| w.as_str().map(str::to_string))
                                .collect::<Vec<String>>()
                        })
                        .unwrap_or_default()
                });
            let full = argv.and_then(|argv| full_access_flags(&a.kind, &argv));
            (a.kind.clone(), a.pane.clone(), full)
        })
        .collect()
}

fn fold(values: &[bool]) -> &'static str {
    if values.is_empty() {
        "none"
    } else if values.iter().all(|v| *v) {
        "full"
    } else if values.iter().any(|v| *v) {
        "mixed"
    } else {
        "sandboxed"
    }
}

/// Fold the octos backends' permission and the other inner loops' into one.
fn combine_permission(octos: &str, others: &[(String, String, Option<bool>)]) -> &'static str {
    if octos == "unknown" || others.iter().any(|o| o.2.is_none()) {
        return "unknown";
    }
    let mut values: Vec<bool> = others.iter().filter_map(|o| o.2).collect();
    match octos {
        "full" => values.push(true),
        "sandboxed" => values.push(false),
        "mixed" => values.extend([true, false]),
        _ => {}
    }
    fold(&values)
}

fn is_serve_argv0(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.strip_prefix("octos") {
        Some("") => true,
        Some(rest) => rest.strip_prefix('-').is_some_and(|tag| {
            !tag.is_empty()
                && tag
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        }),
        None => false,
    }
}

/// How the octos backends on this machine were launched. Only two launch
/// flags are read; the rest of the command line never leaves the machine.
/// Every serve is listed, but only the solo ones decide the permission.
fn serve_permission(ps_output: Option<&str>) -> (&'static str, Vec<Value>) {
    let text = match ps_output {
        Some(file) => match std::fs::read(file) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(_) => return ("unknown", Vec::new()),
        },
        None => {
            let mut cmd = Command::new("ps");
            cmd.args(["-axo", "pid=,command="]);
            match run_for(cmd, Duration::from_secs(10)) {
                Ok(ran) if ran.code == Some(0) => ran.stdout,
                _ => return ("unknown", Vec::new()),
            }
        }
    };
    let mut backends = Vec::new();
    let mut full = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 || !parts[0].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = parts[0].parse::<u64>() else {
            continue;
        };
        let argv = &parts[1..];
        if !is_serve_argv0(argv[0]) || argv[1] != "serve" {
            continue;
        }
        let full_access = argv.contains(&"--danger-full-access");
        let solo = argv.contains(&"--solo");
        // Only a solo serve can back an inner loop: without `--solo` octos
        // refuses a permission profile, so such a serve is somebody's own
        // service and says nothing about how the inner loops were launched.
        if solo {
            full.push(full_access);
        }
        backends.push(json!({
            "pid": pid,
            "full_access": full_access,
            "solo": solo,
        }));
    }
    (fold(&full), backends)
}

/// olp/v3: inner loops run without a sandbox.
fn check_full(permission: &str) -> Check {
    match permission {
        "full" => (true, "内环都以完全访问启动".to_string()),
        "sandboxed" => (false, "内环以沙箱启动;olp/v3 要求免沙箱".to_string()),
        "mixed" => (false, "有内环以沙箱启动;olp/v3 要求全部免沙箱".to_string()),
        "none" => (false, "没有内环在跑".to_string()),
        _ => (
            false,
            "看不出内环是否免沙箱(启动参数里没有完全访问的开关)".to_string(),
        ),
    }
}

fn check_disk(min_free: f64) -> Check {
    let at = home();
    let total = fs2::total_space(&at).unwrap_or(0);
    let free = if total == 0 {
        0.0
    } else {
        fs2::available_space(&at).unwrap_or(0) as f64 * 100.0 / total as f64
    };
    (
        free >= min_free,
        format!("空闲 {free:.1}%(阈值 {min_free}%)"),
    )
}

/// Each executable in the checks directory is one check: exit 0 passes, the
/// first line of stdout is its detail, 10 seconds at most.
fn run_plugin_checks() -> Vec<Value> {
    let dir = env("OLP_UNIT_CHECKS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".octos/outer/unit-checks.d"));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.') && is_executable(&dir.join(n)))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let (ok, detail) = match run_for(Command::new(dir.join(&name)), Duration::from_secs(10))
            {
                Ok(ran) => {
                    let ok = ran.code == Some(0);
                    let mut detail = cut(ran.stdout.trim().lines().next().unwrap_or_default(), 200);
                    if !ok && detail.is_empty() {
                        detail = match ran.code {
                            Some(code) => format!("exit {code}"),
                            None => "被信号终止".to_string(),
                        };
                    }
                    (ok, detail)
                }
                Err(RunError::Timeout) => (false, "超时(10 秒)".to_string()),
                Err(RunError::Spawn(e)) => (false, format!("无法执行: {e}")),
            };
            json!({"name": name, "ok": ok, "detail": detail})
        })
        .collect()
}

fn heartbeat(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(
        argv,
        &["--unit", "--require", "--disk-min-free", "--ps-output"],
        &[],
    )?;
    if !args.rest.is_empty() {
        return usage_err(format!("多余的参数: {}", args.rest.join(" ")));
    }
    let Some(unit) = args.get("--unit") else {
        return usage_err("heartbeat 需要 --unit <单元号>");
    };
    let unit = check_unit_name(unit)?;
    let min_free = match args.get("--disk-min-free") {
        None => 10.0,
        Some(text) => text
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| Fail(USAGE_CODE, "--disk-min-free 必须是数字".to_string()))?,
    };
    let required: Vec<&str> = args
        .get("--require")
        .unwrap_or("agent,disk,full")
        .split(',')
        .filter(|r| !r.is_empty())
        .collect();
    if let Some(bad) = required
        .iter()
        .find(|r| !matches!(**r, "agent" | "serve" | "disk" | "full"))
    {
        return usage_err(format!(
            "未知的内置巡检项: {bad}(可选 agent、serve、disk、full)"
        ));
    }
    let agents = inner_agents();
    let (octos_permission, backends) = serve_permission(args.get("--ps-output"));
    let others = agent_permissions(agents.as_deref().unwrap_or_default());
    let mut permission = combine_permission(octos_permission, &others);
    if agents.is_err() && permission == "none" {
        // No octos backend, and herdr could not say whether another kind of
        // inner loop runs: that is not "none".
        permission = "unknown";
    }
    let mut checks: Vec<Value> = required
        .iter()
        .map(|name| {
            let (ok, detail) = match *name {
                "agent" => check_agent(&agents),
                "serve" => check_serve(),
                "disk" => check_disk(min_free),
                _ => check_full(permission),
            };
            json!({"name": name, "ok": ok, "detail": detail})
        })
        .collect();
    checks.extend(run_plugin_checks());
    let beat = json!({
        "v": 1,
        "unit": unit,
        "hostname": this_host(),
        "macs": local_macs(),
        "sent_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "checks": checks,
        "permission": permission,
        "backends": backends,
        "agents": agents.as_deref().unwrap_or_default().iter().map(|a| {
            json!({"kind": a.kind, "status": a.status, "pane": a.pane})
        }).collect::<Vec<Value>>(),
        "agent_permissions": others.iter().map(|(kind, pane, full)| {
            json!({"kind": kind, "pane": pane, "full_access": full})
        }).collect::<Vec<Value>>(),
        "reviewers": reviewer_status(),
    });
    println!("{beat}");
    Ok(0)
}

// --- hub: receive --------------------------------------------------------

fn list<'a>(beat: &'a Value, key: &str) -> &'a [Value] {
    beat.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn is_token(s: &str, extra: &str, max: usize) -> bool {
    !s.is_empty()
        && s.chars().count() <= max
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn receive(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv, &["--unit"], &[])?;
    if !args.rest.is_empty() {
        return usage_err(format!("多余的参数: {}", args.rest.join(" ")));
    }
    let mut raw = Vec::new();
    let _ = std::io::stdin()
        .take(MAX_HEARTBEAT_BYTES as u64 + 1)
        .read_to_end(&mut raw);
    if raw.len() > MAX_HEARTBEAT_BYTES {
        return usage_err("心跳超过 256 KiB,拒收");
    }
    let Ok(beat) = serde_json::from_slice::<Value>(&raw) else {
        return usage_err("心跳不是合法的 JSON");
    };
    let Some(unit) = beat
        .as_object()
        .and_then(|b| b.get("unit"))
        .and_then(Value::as_str)
    else {
        return usage_err("心跳缺少 unit 字段");
    };
    if !is_unit(unit) {
        return usage_err("心跳里的单元号不合法");
    }
    if let Some(allowed) = args.get("--unit").filter(|a| !a.is_empty() && *a != unit) {
        return Err(Fail(
            REJECTED,
            format!("心跳自报单元 {unit},但这个连接只允许 {allowed},拒收"),
        ));
    }
    if !load_registry().contains_key(unit) {
        return Err(Fail(REJECTED, format!("单元未登记: {unit},拒收")));
    }
    // Only short, plain values of known fields reach the hub's files.
    let checks: Vec<Value> = list(&beat, "checks")
        .iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?;
            Some(json!({
                "name": cut(name, 64),
                "ok": c.get("ok") == Some(&Value::Bool(true)),
                "detail": cut(&text_of(c.get("detail")), 200),
            }))
        })
        .take(200)
        .collect();
    let macs: Vec<String> = list(&beat, "macs")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .filter(|m| is_mac(m))
        .take(64)
        .collect();
    let permission = beat
        .get("permission")
        .and_then(Value::as_str)
        .filter(|p| PERMISSIONS.contains(p))
        .unwrap_or("unknown");
    let agents: Vec<Value> = list(&beat, "agents")
        .iter()
        .filter_map(|a| {
            let field = |k: &str| a.get(k)?.as_str().filter(|v| is_token(v, ":_.-", 40));
            Some(json!({
                "kind": field("kind")?, "status": field("status")?, "pane": field("pane")?,
            }))
        })
        .take(64)
        .collect();
    let backends: Vec<Value> = list(&beat, "backends")
        .iter()
        .filter_map(|b| {
            let pid = b.get("pid").filter(|p| p.is_i64() || p.is_u64())?;
            Some(json!({
                "pid": pid,
                "full_access": b.get("full_access") == Some(&Value::Bool(true)),
                "solo": b.get("solo") == Some(&Value::Bool(true)),
            }))
        })
        .take(64)
        .collect();
    let reviewers: Vec<Value> = list(&beat, "reviewers")
        .iter()
        .filter_map(|r| {
            let kind = r
                .get("kind")?
                .as_str()
                .filter(|k| is_token(k, "_-", 16) && !k.chars().any(|c| c.is_ascii_uppercase()))?;
            Some(json!({
                "kind": kind,
                "installed": r.get("installed") == Some(&Value::Bool(true)),
                "logged_in": r.get("logged_in").filter(|v| v.is_boolean()),
            }))
        })
        .take(16)
        .collect();
    let stored = json!({
        "v": 1,
        "unit": unit,
        "permission": permission,
        "reviewers": reviewers,
        "backends": backends,
        "agents": agents,
        "hostname": norm_host(&text_of(beat.get("hostname"))),
        "macs": macs,
        "sent_at": cut(&text_of(beat.get("sent_at")), 40),
        "checks": checks,
        // The hub's clock: staleness is judged by it alone.
        "received_at": now_seconds(),
    });
    write_atomic(&heartbeat_path(unit), &format!("{stored}\n"))
        .map_err(|e| io_fail("写心跳失败", e))?;
    println!("received {unit}");
    Ok(0)
}

// --- hub: status / check / where -----------------------------------------

/// What the hub makes of one registered unit.
pub struct Health {
    /// `healthy`, `degraded`, `stale` or `offline`.
    pub state: &'static str,
    pub reasons: Vec<String>,
    pub beat: Option<Value>,
}

fn evaluate(row: &Row, now: f64, stale_after: u64) -> Health {
    let beat = std::fs::read_to_string(heartbeat_path(&row["unit"]))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|b| b.is_object() && b.get("received_at").is_some_and(Value::is_number));
    let verdict = |state, reasons, beat| Health {
        state,
        reasons,
        beat,
    };
    if row["enabled"] == "no" {
        return verdict("offline", vec!["人工标记离线".to_string()], beat);
    }
    let Some(beat) = beat else {
        return verdict("stale", vec!["没有收到过心跳".to_string()], None);
    };
    let age = now - beat["received_at"].as_f64().unwrap_or(0.0);
    if age > stale_after as f64 {
        return verdict(
            "stale",
            vec![format!(
                "最近一次心跳在 {} 秒前(上限 {stale_after} 秒)",
                age as i64
            )],
            Some(beat),
        );
    }
    let mut reasons = Vec::new();
    let reported = beat.get("hostname").and_then(Value::as_str);
    if reported != Some(row["hostname"].as_str()) {
        reasons.push(format!(
            "主机名不符:登记 {},上报 {}",
            row["hostname"],
            reported.filter(|h| !h.is_empty()).unwrap_or("空")
        ));
    }
    if !list(&beat, "macs")
        .iter()
        .any(|m| m.as_str() == Some(row["mac"].as_str()))
    {
        reasons.push(format!(
            "网卡地址不符:登记的 {} 不在上报的地址里",
            row["mac"]
        ));
    }
    for c in list(&beat, "checks") {
        if c.get("ok") != Some(&Value::Bool(true)) {
            let detail = c
                .get("detail")
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
                .map(|d| format!(":{d}"))
                .unwrap_or_default();
            reasons.push(format!(
                "巡检未过 {}{detail}",
                c.get("name").and_then(Value::as_str).unwrap_or("?")
            ));
        }
    }
    let state = if reasons.is_empty() {
        "healthy"
    } else {
        "degraded"
    };
    verdict(state, reasons, Some(beat))
}

/// The health of a registered unit; `None` when it is not registered.
pub fn health(unit: &str, stale_after: u64) -> Option<Health> {
    load_registry()
        .get(unit)
        .map(|row| evaluate(row, now_seconds(), stale_after))
}

/// The reviewer CLIs a unit last reported: (kind, installed, logged in).
pub fn reviewers_of(beat: &Value) -> Vec<(String, bool, Option<bool>)> {
    list(beat, "reviewers")
        .iter()
        .filter_map(|r| {
            Some((
                r.get("kind")?.as_str()?.to_string(),
                r.get("installed") == Some(&Value::Bool(true)),
                r.get("logged_in").and_then(Value::as_bool),
            ))
        })
        .collect()
}

pub fn state_zh(state: &str) -> &'static str {
    match state {
        "healthy" => "正常",
        "degraded" => "降级",
        "stale" => "失联",
        _ => "离线",
    }
}

/// `claude 已登录,codex 未登录` — or `-` when the unit did not say.
fn reviewers_text(beat: &Value) -> String {
    let words: Vec<String> = reviewers_of(beat)
        .into_iter()
        .map(|(kind, installed, logged_in)| {
            let state = match (installed, logged_in) {
                (false, _) => "未安装",
                (true, Some(true)) => "已登录",
                (true, Some(false)) => "未登录",
                (true, None) => "登录状态未知",
            };
            format!("{kind} {state}")
        })
        .collect();
    if words.is_empty() {
        "-".to_string()
    } else {
        words.join(",")
    }
}

/// One record per registered unit, as `status --json` lists them.
pub fn report(stale_after: u64) -> Vec<Value> {
    let now = now_seconds();
    load_registry()
        .iter()
        .map(|(unit, row)| {
            let h = evaluate(row, now, stale_after);
            let beat = h.beat.as_ref();
            let of = |key: &str| {
                beat.and_then(|b| b.get(key))
                    .cloned()
                    .unwrap_or_else(|| json!([]))
            };
            json!({
                "unit": unit,
                "state": h.state,
                "dispatchable": h.state == "healthy",
                "reasons": h.reasons,
                "hostname": row["hostname"],
                "mac": row["mac"],
                "heartbeat_age_seconds": beat
                    .map(|b| (now - b["received_at"].as_f64().unwrap_or(0.0)) as i64),
                // What the unit said about itself; told, never judged.
                "permission": beat.map(|b| {
                    b.get("permission")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                }),
                "backends": of("backends"),
                "agents": of("agents"),
                "reviewers": of("reviewers"),
            })
        })
        .collect()
}

/// The inner-loop kinds in a unit record, sorted, or `-`.
pub fn kinds_text(record: &Value) -> String {
    let mut kinds: Vec<&str> = list(record, "agents")
        .iter()
        .map(|a| a.get("kind").and_then(Value::as_str).unwrap_or("?"))
        .collect();
    kinds.sort();
    kinds.dedup();
    if kinds.is_empty() {
        "-".to_string()
    } else {
        kinds.join(",")
    }
}

fn status(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv, &["--stale-after"], &["--json"])?;
    if !args.rest.is_empty() {
        return usage_err(format!("多余的参数: {}", args.rest.join(" ")));
    }
    let report = report(args.stale_after()?);
    if args.flags.iter().any(|f| f == "--json") {
        let doc = json!({ "units": report });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
        );
        return Ok(0);
    }
    if report.is_empty() {
        println!("没有登记任何单元");
    }
    for r in &report {
        let word = |key: &str| r.get(key).and_then(Value::as_str).unwrap_or_default();
        let mut line = format!(
            "{}\t{}({})\t心跳 {}\t内环 {}\t权限 {}\t复验方 {}\t{}",
            word("unit"),
            state_zh(word("state")),
            word("state"),
            r["heartbeat_age_seconds"]
                .as_i64()
                .map(|a| format!("{a}s"))
                .unwrap_or_else(|| "-".to_string()),
            kinds_text(r),
            match r["permission"].as_str() {
                Some("full") => "完全访问",
                Some("sandboxed") => "沙箱",
                Some("mixed") => "混合",
                Some("none") => "无内环",
                Some("unknown") => "未知",
                _ => "-",
            },
            reviewers_text(r),
            word("hostname"),
        );
        let reasons: Vec<&str> = list(r, "reasons")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if !reasons.is_empty() {
            line.push('\t');
            line.push_str(&reasons.join(";"));
        }
        println!("{line}");
    }
    Ok(0)
}

/// Whether the unit is this machine (a single-machine deployment); `None`
/// when it is not registered.
pub fn is_local(unit: &str) -> Option<bool> {
    load_registry()
        .get(unit)
        .map(|row| row["hostname"] == this_host())
}

fn where_is(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv, &[], &[])?;
    let [unit] = args.positionals(["<单元号>"])?;
    let unit = check_unit_name(unit)?;
    match is_local(unit) {
        None => usage_err(format!("单元未登记: {unit}")),
        Some(local) => {
            println!("{}", if local { "local" } else { "remote" });
            Ok(0)
        }
    }
}

fn check(argv: &[String]) -> Result<i32, Fail> {
    let args = parse(argv, &["--stale-after"], &[])?;
    let [unit] = args.positionals(["<单元号>"])?;
    let unit = check_unit_name(unit)?;
    let Some(h) = health(unit, args.stale_after()?) else {
        return usage_err(format!("单元未登记: {unit}"));
    };
    if h.state == "healthy" {
        println!("{unit} 正常,可以派卡");
        return Ok(0);
    }
    eprintln!(
        "{unit} {},不要派卡:{}",
        state_zh(h.state),
        h.reasons.join(";")
    );
    Ok(1)
}

/// `argv` is what follows `olp unit`.
pub fn run(argv: &[String]) -> i32 {
    let (action, rest) = match argv.split_first() {
        Some((a, rest)) if !matches!(a.as_str(), "-h" | "--help") => (a.as_str(), rest),
        Some(_) => {
            println!("{USAGE}");
            return 0;
        }
        None => {
            eprintln!("{USAGE}");
            return USAGE_CODE;
        }
    };
    if rest.iter().any(|w| w == "-h" || w == "--help") {
        println!("{USAGE}");
        return 0;
    }
    let result = match action {
        "register" => register(rest),
        "set-state" => set_state(rest),
        "heartbeat" => heartbeat(rest),
        "receive" => receive(rest),
        "status" => status(rest),
        "check" => check(rest),
        "where" => where_is(rest),
        other => usage_err(format!("没有这个动作: {other}\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            eprintln!("olp unit: {why}");
            code
        }
    }
}
