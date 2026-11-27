//! `octoscode olp remote append|ack|flush|queue|mirror|serve` — a unit
//! writing to a hub whose blackboard is a Markdown file.
//!
//! The board lives on the hub only. A unit's write runs there, under the
//! board's lock, through the ssh forced command `serve`; when the hub
//! cannot be reached the write is queued on the unit and resent in order.
//! With `hub = local` the same `serve` runs on this machine, no ssh.
//!
//! What `serve` lets a unit do: `append <key>` and `read`, nothing else.
//! The key must start with `<unit>:`; the text may not contain a line
//! signed `外环(` (verdicts are the outer loop's) nor a numbered heading
//! (cards are opened by the outer loop); 256 KiB at most.
//!
//! (A hub that keeps an event log is served by `remote serve-log`; see
//! `client.rs`.)
//!
//! Exit codes: 0; 2 usage or configuration; 3 refused by the hub; 75 the
//! hub was not reached, the write is queued; else the board writer's code.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use super::board::strip_quote;
use super::client::{conf, setting};
use super::unit::{is_target, is_unit};

const MAX_BODY: usize = 256 * 1024;
pub const EX_TEMPFAIL: i32 = 75;
const UNREACHABLE: i32 = 255;
const LOCAL: &str = "local";
const PROG: &str = "olp-board-remote";

pub struct Fail(pub i32, pub String);

/// Where a unit's writes go.
pub struct Hub {
    target: String,
    pub unit: String,
    /// The board's path, when the hub is this machine.
    board: String,
}

fn is_key(s: &str) -> bool {
    (1..=180).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@+=-".contains(&b))
}

fn hex(bytes: &[u8], chars: usize) -> String {
    let digest = Sha256::digest(bytes);
    let text: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    text[..chars].to_string()
}

/// The hub and this unit's id: flag, then environment, then `unit.conf`.
pub fn resolve(hub: Option<&String>, unit: Option<&String>) -> Result<Hub, Fail> {
    let conf = conf();
    let target = setting(hub, "OLP_HUB", &conf, "hub");
    let unit = setting(unit, "OLP_UNIT_ID", &conf, "unit");
    if !is_target(&target) {
        return Err(Fail(
            2,
            "没有配置 hub 的 ssh 目标(--hub、OLP_HUB 或 unit.conf 里的 hub=)".to_string(),
        ));
    }
    if !is_unit(&unit) {
        return Err(Fail(
            2,
            "没有配置本单元号(--unit、OLP_UNIT_ID 或 unit.conf 里的 unit=)".to_string(),
        ));
    }
    let board = setting(None, "OLP_BOARD", &conf, "board");
    if target == LOCAL && !Path::new(&board).is_absolute() {
        return Err(Fail(
            2,
            "hub = local 时要给黑板的绝对路径(OLP_BOARD 或 unit.conf 里的 board=)".to_string(),
        ));
    }
    Ok(Hub {
        target,
        unit,
        board,
    })
}

fn queue_dir() -> PathBuf {
    std::env::var_os("OLP_QUEUE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".octos/outer/queue")
        })
}

struct Ran {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

/// Run the hub side with one request.
fn hub_run(hub: &Hub, request: &[&str], stdin: &[u8]) -> Ran {
    let unreachable = |why: String| Ran {
        code: UNREACHABLE,
        stdout: Vec::new(),
        stderr: why,
    };
    let cmd = if hub.target == LOCAL {
        // The forced command, run here: the same handler, the same rules.
        let Ok(exe) = std::env::current_exe() else {
            return unreachable("无法在本机运行 hub 一侧".to_string());
        };
        let line = shlex::try_join(request.iter().copied()).unwrap_or_default();
        let mut cmd = Command::new(exe);
        cmd.args([
            "olp", "remote", "serve", "--unit", &hub.unit, "--board", &hub.board,
        ])
        .env("SSH_ORIGINAL_COMMAND", line);
        cmd
    } else {
        let ssh = std::env::var("OLP_SSH").ok().filter(|v| !v.is_empty());
        let mut cmd = Command::new(ssh.as_deref().unwrap_or("ssh"));
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            &hub.target,
        ])
        .args(request);
        cmd
    };
    // The whole request has a deadline, not only the connection.
    match super::proc::run(cmd, Some(stdin), super::proc::ssh_limit()) {
        Err(super::proc::RunError::Spawn(e)) => unreachable(format!("无法运行 ssh: {e}")),
        Err(super::proc::RunError::Timeout) => unreachable(format!(
            "{} 秒内没有答复",
            super::proc::ssh_limit().as_secs()
        )),
        Ok(out) => Ran {
            code: out.code.unwrap_or(UNREACHABLE),
            stderr: out.stderr_text(),
            stdout: out.stdout,
        },
    }
}

/// Queued writes in send order: (file, key, text).
fn queued() -> Vec<(PathBuf, String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(queue_dir()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "entry"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(&path).ok()?;
            let end = bytes.iter().position(|b| *b == b'\n')?;
            let key = String::from_utf8_lossy(&bytes[..end]).into_owned();
            is_key(&key).then(|| (path, key, bytes[end + 1..].to_vec()))
        })
        .collect()
}

fn enqueue(key: &str, body: &[u8]) {
    if queued().iter().any(|(_, queued_key, _)| queued_key == key) {
        return; // the same write is already waiting
    }
    let dir = queue_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{nanos:020}-{}.entry", hex(key.as_bytes(), 12));
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        let mut tmp = tempfile::Builder::new().prefix(".q-").tempfile_in(&dir)?;
        tmp.write_all(key.as_bytes())?;
        tmp.write_all(b"\n")?;
        tmp.write_all(body)?;
        tmp.as_file().sync_all()?;
        tmp.persist(dir.join(name)).map_err(|e| e.error)?;
        Ok(())
    });
    if let Err(e) = written {
        eprintln!("{PROG}: 排不进队列 {}: {e}", dir.display());
    }
}

/// Resend queued writes in order: (sent, still waiting, hub unreachable).
fn flush_queue(hub: &Hub) -> (usize, usize, bool) {
    let items = queued();
    let mut sent = 0;
    for (index, (path, key, body)) in items.iter().enumerate() {
        let ran = hub_run(hub, &["append", key], body);
        if ran.code == UNREACHABLE {
            return (sent, items.len() - index, true);
        }
        if ran.code != 0 {
            // Refused outright: it would never go through; move it aside so
            // it does not hold up what follows.
            let _ = std::fs::rename(path, path.with_extension("rejected"));
            eprintln!(
                "{PROG}: 排队的写入 {key} 被 hub 拒绝(退出码 {}):{}",
                ran.code,
                ran.stderr.trim()
            );
            continue;
        }
        let _ = std::fs::remove_file(path);
        sent += 1;
    }
    (sent, 0, false)
}

/// Send one write, after whatever is queued; queue it when the hub cannot
/// be reached. Returns the exit code.
pub fn deliver(hub: &Hub, key: &str, body: &[u8]) -> i32 {
    let (_, remaining, unreachable) = flush_queue(hub);
    if unreachable {
        enqueue(key, body);
        eprintln!("{PROG}: hub 不可达,已排队(队列里共 {} 条)", remaining + 1);
        return EX_TEMPFAIL;
    }
    let ran = hub_run(hub, &["append", key], body);
    if ran.code == UNREACHABLE {
        enqueue(key, body);
        eprintln!("{PROG}: hub 不可达,已排队(队列里共 1 条)");
        return EX_TEMPFAIL;
    }
    print!("{}", String::from_utf8_lossy(&ran.stdout));
    if !ran.stderr.trim().is_empty() {
        eprintln!("{}", ran.stderr.trim());
    }
    ran.code
}

/// `<unit>:<key>`; without a key, the first 16 hex digits of the text's hash.
pub fn make_key(unit: &str, key: Option<&str>, body: &[u8]) -> Result<String, Fail> {
    let suffix = match key.filter(|k| !k.is_empty()) {
        Some(key) if is_key(key) => key.to_string(),
        Some(_) => {
            return Err(Fail(
                2,
                "--key 只允许 [A-Za-z0-9._:/@+=-],最长 180".to_string(),
            ));
        }
        None => hex(body, 16),
    };
    let full = if suffix.starts_with(&format!("{unit}:")) {
        suffix
    } else {
        format!("{unit}:{suffix}")
    };
    if is_key(&full) {
        Ok(full)
    } else {
        Err(Fail(2, "--key 太长".to_string()))
    }
}

struct Flags {
    hub: Option<String>,
    unit: Option<String>,
    key: Option<String>,
    to: Option<String>,
    board: Option<String>,
    sign: bool,
    rest: Vec<String>,
}

fn flags(argv: &[String]) -> Result<Flags, Fail> {
    let mut out = Flags {
        hub: None,
        unit: None,
        key: None,
        to: None,
        board: None,
        sign: false,
        rest: Vec::new(),
    };
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        let slot = match name {
            "--sign" => {
                out.sign = true;
                continue;
            }
            "--hub" => &mut out.hub,
            "--unit" => &mut out.unit,
            "--key" => &mut out.key,
            "--to" => &mut out.to,
            "--board" => &mut out.board,
            other if other.starts_with("--") => {
                return Err(Fail(2, format!("不认识的参数: {other}")));
            }
            _ => {
                out.rest.push(word.clone());
                continue;
            }
        };
        *slot = Some(
            inline
                .or_else(|| words.next().cloned())
                .ok_or_else(|| Fail(2, format!("{name} 需要一个值")))?,
        );
    }
    Ok(out)
}

fn read_stdin(limit: usize) -> Vec<u8> {
    let mut body = Vec::new();
    let _ = std::io::stdin()
        .take(limit as u64 + 1)
        .read_to_end(&mut body);
    body
}

fn append(argv: &[String]) -> Result<i32, Fail> {
    // Extra positionals are ignored: the MCP audit hook passes some.
    let args = flags(argv)?;
    let hub = resolve(args.hub.as_ref(), args.unit.as_ref())?;
    let mut body = read_stdin(MAX_BODY);
    if body.len() > MAX_BODY {
        return Err(Fail(2, "正文超过 256 KiB".to_string()));
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Err(Fail(2, "正文为空".to_string()));
    }
    if args.sign {
        // Lines with no signature of their own (the MCP server's audit
        // entries) get the unit's, so the hub can tell whose they are.
        let text = String::from_utf8_lossy(&body).into_owned();
        let signed: Vec<String> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| format!("> 单元({})·{line}", hub.unit))
            .collect();
        body = format!("{}\n", signed.join("\n")).into_bytes();
    }
    let key = make_key(&hub.unit, args.key.as_deref(), &body)?;
    Ok(deliver(&hub, &key, &body))
}

fn ack(argv: &[String]) -> Result<i32, Fail> {
    let args = flags(argv)?;
    let hub = resolve(args.hub.as_ref(), args.unit.as_ref())?;
    let [number, kind, text @ ..] = args.rest.as_slice() else {
        return Err(Fail(
            2,
            "用法: remote ack <N> done|wontdo|blocked <说明>".to_string(),
        ));
    };
    let number: u64 = number
        .parse()
        .map_err(|_| Fail(2, format!("卡号必须是数字: {number}")))?;
    if !matches!(kind.as_str(), "done" | "wontdo" | "blocked") {
        return Err(Fail(2, "结果只能是 done、wontdo 或 blocked".to_string()));
    }
    let text = text.join(" ");
    let text = text.trim();
    if text.is_empty() {
        return Err(Fail(
            2,
            "ACK 需要说明(done 附 commit/测试证据,wontdo 附理由,blocked 附原因)".to_string(),
        ));
    }
    if text.contains(['\n', '\r']) {
        return Err(Fail(2, "ACK 说明必须是单行".to_string()));
    }
    let body = format!("> 内环({})·#{number} ACK({kind}): {text}\n", hub.unit).into_bytes();
    let key = make_key(&hub.unit, None, &body)?;
    Ok(deliver(&hub, &key, &body))
}

fn mirror(argv: &[String]) -> Result<i32, Fail> {
    let args = flags(argv)?;
    // No guessed default: a read-only copy written over the wrong file is
    // worse than a usage error.
    let configured = setting(None, "OLP_MIRROR", &conf(), "mirror");
    let target = match (&args.to, configured.is_empty()) {
        (Some(to), _) => std::path::absolute(to).unwrap_or_else(|_| PathBuf::from(to)),
        (None, false) if Path::new(&configured).is_absolute() => PathBuf::from(&configured),
        (None, false) => {
            return Err(Fail(
                2,
                format!("配置里的镜像路径必须是绝对路径: {configured}"),
            ));
        }
        (None, true) => {
            return Err(Fail(
                2,
                "没有镜像路径(--to、OLP_MIRROR 或 unit.conf 里的 mirror=)".to_string(),
            ));
        }
    };
    let hub = resolve(args.hub.as_ref(), args.unit.as_ref())?;
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if hub.target == LOCAL && real(&target) == real(Path::new(&hub.board)) {
        // One machine, one project: the board the inner loop reads is the
        // authoritative one. Copying it onto itself would make it read-only.
        println!("hub 就是本机,镜像路径就是权威黑板,不需要同步");
        return Ok(0);
    }
    let ran = hub_run(&hub, &["read"], b"");
    if ran.code == UNREACHABLE {
        eprintln!("{PROG}: hub 不可达,镜像保持不变");
        return Ok(EX_TEMPFAIL);
    }
    if ran.code != 0 {
        return Err(Fail(
            ran.code,
            format!("hub 拒绝读取:{}", ran.stderr.trim()),
        ));
    }
    let dir = target.parent().unwrap_or(Path::new("."));
    let written = std::fs::create_dir_all(dir).and_then(|()| {
        let mut tmp = tempfile::Builder::new()
            .prefix(".mirror-")
            .tempfile_in(dir)?;
        tmp.write_all(&ran.stdout)?;
        let mut mode = tmp.as_file().metadata()?.permissions();
        mode.set_readonly(true);
        #[cfg(unix)]
        std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o444);
        std::fs::set_permissions(tmp.path(), mode)?;
        tmp.persist(&target).map_err(|e| e.error)?;
        Ok(())
    });
    written.map_err(|e| Fail(2, format!("写不了镜像 {}: {e}", target.display())))?;
    println!(
        "mirrored {} bytes to {}",
        ran.stdout.len(),
        target.display()
    );
    Ok(0)
}

/// The hub's forced command.
fn serve(argv: &[String]) -> Result<i32, Fail> {
    let args = flags(argv)?;
    let (Some(unit), Some(board)) = (&args.unit, &args.board) else {
        return Err(Fail(
            2,
            "serve 需要 --unit <单元号> 与 --board <黑板路径>".to_string(),
        ));
    };
    if !is_unit(unit) {
        return Err(Fail(2, format!("单元号不合法: {unit}")));
    }
    let board = std::path::absolute(board).unwrap_or_else(|_| PathBuf::from(board));
    let words = shlex::split(&std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default())
        .unwrap_or_default();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let refuse = |why: &str| Err(Fail(3, why.to_string()));
    if words == ["read"] {
        let Ok(bytes) = std::fs::read(&board) else {
            return refuse("黑板不存在");
        };
        let _ = std::io::stdout().write_all(&bytes);
        return Ok(0);
    }
    let ["append", key] = words.as_slice() else {
        return refuse("这个密钥只能用于 append <键> 与 read");
    };
    let own = key.strip_prefix(&format!("{unit}:")).is_some_and(is_key);
    if !own || key.len() > 200 {
        return refuse(&format!("键必须以 {unit}: 开头且只含 [A-Za-z0-9._:/@+=-]"));
    }
    let body = read_stdin(MAX_BODY);
    if body.len() > MAX_BODY {
        return refuse("正文超过 256 KiB");
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return refuse("正文为空");
    }
    let Ok(text) = std::str::from_utf8(&body) else {
        return refuse("正文不是 UTF-8");
    };
    for line in text.lines() {
        if strip_quote(line).starts_with("外环(") {
            return refuse("单元不能写外环署名的行(采认、打回、改判、搁置只在 hub 上写)");
        }
        let numbered = line.strip_prefix("### ").is_some_and(|rest| {
            rest.starts_with("{N}") || rest.starts_with(|c: char| c.is_ascii_digit())
        });
        if numbered {
            return refuse("单元不能写编号条目(立卡只在大外环)");
        }
    }
    Ok(super::append::write_entry(&board, Some(key), body))
}

/// `argv` is what follows `olp remote` (the Markdown channel's actions).
pub fn run(argv: &[String]) -> i32 {
    let usage = "用法: octoscode olp remote <动作>\n  单元上:append [--key K] [--sign] | ack <N> done|wontdo|blocked <说明> | flush | queue | mirror [--to 路径]\n  hub 上(ssh 强制命令):serve --unit <单元号> --board <黑板路径>\n  事件日志上另有:progress、inbox、watch、serve-log\n退出码:0 成功;2 用法或配置错误;3 hub 拒绝;75 hub 不可达,已排队";
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
    if rest.iter().any(|w| w == "-h" || w == "--help") {
        println!("{usage}");
        return 0;
    }
    let result = match action {
        "append" => append(rest),
        "ack" => ack(rest),
        "mirror" => mirror(rest),
        "serve" => serve(rest),
        "flush" => flags(rest)
            .and_then(|args| resolve(args.hub.as_ref(), args.unit.as_ref()))
            .map(|hub| {
                let (sent, remaining, unreachable) = flush_queue(&hub);
                println!("已重发 {sent} 条,队列里还有 {remaining} 条");
                if unreachable { EX_TEMPFAIL } else { 0 }
            }),
        "queue" => {
            let items = queued();
            if items.is_empty() {
                println!("队列为空");
            }
            for (_, key, body) in &items {
                let text = String::from_utf8_lossy(body);
                let first: String = text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect();
                println!("{key}\t{first}");
            }
            Ok(0)
        }
        other => Err(Fail(2, format!("没有这个动作: {other}\n{usage}"))),
    };
    match result {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            eprintln!("{PROG}: {why}");
            code
        }
    }
}
