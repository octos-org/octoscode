//! `octoscode olp keepalive heartbeat|install|uninstall|list|render|doctor`
//! — register what must always run on a machine as jobs of the current
//! user: launchd agents on macOS, systemd user units on Linux.
//!
//! `install <name> (--interval N | --keepalive) [--env K=V]… -- <command>`
//! registers any command; `heartbeat --unit U --hub H` registers the unit's
//! heartbeat (`… unit heartbeat | ssh H receive`, or straight into the
//! local `unit receive` when the hub is `local`). `doctor` only reports
//! what keeps jobs from running unattended; it changes nothing.
//!
//! A job's PATH is short, so one is written into it; `--env` overrides.
//! Logs go to `~/.octos/outer/logs/<name>.log`.
//!
//! What is deliberately not done: only infrastructure is brought back. A
//! conversation in a pane and the outer loop's duty lock are not restored
//! here.
//!
//! Exit codes: 0; 1 the service manager failed, or `doctor` found a
//! problem; 2 usage, or asked to load jobs on the wrong system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::unit::is_unit;

struct Fail(i32, String);

fn fail<T>(code: i32, why: impl Into<String>) -> Result<T, Fail> {
    Err(Fail(code, why.into()))
}

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Launchd,
    Systemd,
}

impl Backend {
    fn prog(self) -> &'static str {
        match self {
            Backend::Launchd => "olp-launchd",
            Backend::Systemd => "olp-systemd",
        }
    }

    /// Can jobs be loaded on this machine?
    fn native(self) -> bool {
        match self {
            Backend::Launchd => cfg!(target_os = "macos"),
            Backend::Systemd => cfg!(target_os = "linux"),
        }
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn log_path(name: &str) -> PathBuf {
    home().join(".octos/outer/logs").join(format!("{name}.log"))
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.len() <= 41
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_target(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || "._@-".contains(c))
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// The PATH written into a job: a service manager's own is too short.
fn default_path(backend: Backend) -> String {
    match backend {
        Backend::Launchd => {
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
        }
        Backend::Systemd => {
            let h = home();
            format!(
                "{0}/.local/bin:{0}/.cargo/bin:/usr/local/bin:/usr/bin:/bin",
                h.display()
            )
        }
    }
}

fn output(cmd: &mut Command) -> (bool, String, String) {
    match cmd.output() {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(e) => (false, String::new(), e.to_string()),
    }
}

fn user_id() -> String {
    output(Command::new("id").arg("-u")).1
}

fn user_name() -> String {
    ["USER", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| output(Command::new("id").arg("-un")).1)
}

struct Job {
    name: String,
    command: Vec<String>,
    interval: Option<u64>,
    env: BTreeMap<String, String>,
}

fn job(
    backend: Backend,
    name: &str,
    command: &[String],
    interval: Option<u64>,
    keepalive: bool,
    env_pairs: &[String],
) -> Result<Job, Fail> {
    if !is_name(name) {
        return fail(
            2,
            format!("名字不合法: {name}(小写字母、数字、连字符,以字母或数字开头)"),
        );
    }
    if command.is_empty() {
        return fail(2, "缺少要运行的命令(写在 -- 之后)");
    }
    if keepalive == interval.is_some() {
        return fail(2, "--interval 与 --keepalive 必须且只能给一个");
    }
    if interval.is_some_and(|n| !(10..=86400).contains(&n)) {
        return fail(2, "--interval 须在 10 到 86400 秒之间");
    }
    if backend == Backend::Systemd && !Path::new(&command[0]).is_absolute() {
        return fail(
            2,
            format!(
                "命令须写绝对路径(systemd 不按 PATH 找 ExecStart): {}",
                command[0]
            ),
        );
    }
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), default_path(backend));
    for pair in env_pairs {
        let ok = pair.split_once('=').filter(|(key, _)| {
            let mut chars = key.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        let Some((key, value)) = ok else {
            return fail(2, format!("--env 须写成 KEY=VALUE: {pair}"));
        };
        env.insert(key.to_string(), value.to_string());
    }
    Ok(Job {
        name: name.to_string(),
        command: command.to_vec(),
        interval,
        env,
    })
}

// --- launchd -------------------------------------------------------------

const LAUNCHD_PREFIX: &str = "org.octos.olp.";

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unxml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// The job as a property list, keys sorted.
fn plist(job: &Job) -> String {
    let log = xml(&log_path(&job.name).to_string_lossy());
    let string = |s: &str| format!("<string>{}</string>", xml(s));
    let mut keys: Vec<(&str, String)> = vec![
        ("EnvironmentVariables", {
            let rows: String = job
                .env
                .iter()
                .map(|(k, v)| format!("\t\t<key>{}</key>\n\t\t{}\n", xml(k), string(v)))
                .collect();
            format!("<dict>\n{rows}\t</dict>")
        }),
        ("Label", string(&format!("{LAUNCHD_PREFIX}{}", job.name))),
        ("ProcessType", string("Background")),
        ("ProgramArguments", {
            let rows: String = job
                .command
                .iter()
                .map(|a| format!("\t\t{}\n", string(a)))
                .collect();
            format!("<array>\n{rows}\t</array>")
        }),
        ("RunAtLoad", "<true/>".to_string()),
        ("StandardErrorPath", format!("<string>{log}</string>")),
        ("StandardOutPath", format!("<string>{log}</string>")),
    ];
    match job.interval {
        Some(n) => keys.push(("StartInterval", format!("<integer>{n}</integer>"))),
        None => {
            keys.push(("KeepAlive", "<true/>".to_string()));
            // launchd's own default; written so a crash loop does not spin.
            keys.push(("ThrottleInterval", "<integer>10</integer>".to_string()));
        }
    }
    keys.sort_by(|a, b| a.0.cmp(b.0));
    let body: String = keys
        .iter()
        .map(|(key, value)| format!("\t<key>{key}</key>\n\t{value}\n"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    )
}

fn launchctl(args: &[&str]) -> (bool, String, String) {
    output(Command::new("launchctl").args(args))
}

fn launchd_domain() -> String {
    format!("gui/{}", user_id())
}

// --- systemd -------------------------------------------------------------

const SYSTEMD_PREFIX: &str = "olp-";
const MARKER: &str = "# generated by scripts/olp-systemd.py; re-run the tool instead of editing";
const COMMAND_MARK: &str = "# olp-command: ";

/// One argument in ExecStart syntax: quoted, with `\`, `"` escaped and `%`,
/// `$` doubled, so it reaches the program unchanged.
fn quote_arg(arg: &str) -> Result<String, Fail> {
    if arg.contains(['\n', '\r']) {
        return fail(2, "命令参数里不能有换行");
    }
    Ok(format!(
        "\"{}\"",
        arg.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}

/// The job's unit files: a service, plus a timer for an interval job.
fn units(job: &Job) -> Result<Vec<(String, String)>, Fail> {
    let label = format!("{SYSTEMD_PREFIX}{}", job.name);
    let log = log_path(&job.name);
    let mut lines = vec![
        MARKER.to_string(),
        format!(
            "{COMMAND_MARK}{}",
            serde_json::to_string(&job.command).unwrap_or_default()
        ),
        "[Unit]".to_string(),
        format!("Description=OctoLoop {}", job.name),
        String::new(),
        "[Service]".to_string(),
        if job.interval.is_none() {
            "Type=simple".to_string()
        } else {
            "Type=oneshot".to_string()
        },
    ];
    for (key, value) in &job.env {
        if value.contains(['\n', '\r']) {
            return fail(2, format!("--env {key} 的值里不能有换行"));
        }
        lines.push(format!(
            "Environment=\"{key}={}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
        ));
    }
    let exec: Result<Vec<String>, Fail> = job.command.iter().map(|a| quote_arg(a)).collect();
    lines.push(format!("ExecStart={}", exec?.join(" ")));
    lines.push(format!("StandardOutput=append:{}", log.display()));
    lines.push(format!("StandardError=append:{}", log.display()));
    if job.interval.is_none() {
        lines.extend(
            [
                "Restart=always",
                "RestartSec=10",
                "",
                "[Install]",
                "WantedBy=default.target",
            ]
            .map(str::to_string),
        );
    }
    let mut out = vec![(
        format!("{label}.service"),
        format!("{}\n", lines.join("\n")),
    )];
    if let Some(interval) = job.interval {
        out.push((
            format!("{label}.timer"),
            format!(
                "{MARKER}\n[Unit]\nDescription=OctoLoop {} (every {interval} s)\n\n[Timer]\nOnBootSec=10\nOnActiveSec=10\nOnUnitActiveSec={interval}\nAccuracySec=1\nUnit={label}.service\n\n[Install]\nWantedBy=timers.target\n",
                job.name
            ),
        ));
    }
    Ok(out)
}

fn systemctl(args: &[&str]) -> (bool, String, String) {
    // `systemctl --user` finds the user manager through XDG_RUNTIME_DIR; a
    // shell reached by su, sudo -u or a bare ssh command often lacks it.
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user").args(args);
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        cmd.env("XDG_RUNTIME_DIR", format!("/run/user/{}", user_id()));
    }
    output(&mut cmd)
}

/// Stop and disable both units of a job, one call each: systemctl fails the
/// whole command when any named unit does not exist.
fn stop_job(label: &str) {
    for ext in [".timer", ".service"] {
        systemctl(&["disable", "--now", &format!("{label}{ext}")]);
    }
    systemctl(&["reset-failed", &format!("{label}.service")]);
}

// --- shared --------------------------------------------------------------

struct Flags {
    dir: Option<String>,
    no_load: bool,
    interval: Option<u64>,
    keepalive: bool,
    env: Vec<String>,
    values: BTreeMap<String, String>,
    rest: Vec<String>,
}

fn flags(argv: &[String], valued: &[&str]) -> Result<Flags, Fail> {
    let mut out = Flags {
        dir: None,
        no_load: false,
        interval: None,
        keepalive: false,
        env: Vec::new(),
        values: BTreeMap::new(),
        rest: Vec::new(),
    };
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
            "--no-load" => out.no_load = true,
            "--keepalive" => out.keepalive = true,
            "--dir" => out.dir = Some(value()?),
            "--env" => out.env.push(value()?),
            "--interval" => {
                let text = value()?;
                out.interval = Some(
                    text.parse::<u64>()
                        .map_err(|_| Fail(2, format!("--interval 不是整数: {text}")))?,
                );
            }
            other if valued.contains(&other) => {
                out.values.insert(other.to_string(), value()?);
            }
            other if other.starts_with("--") => {
                return fail(2, format!("不认识的参数: {other}"));
            }
            _ => out.rest.push(word.clone()),
        }
    }
    Ok(out)
}

fn jobs_dir(backend: Backend, f: &Flags) -> PathBuf {
    match (&f.dir, backend) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Backend::Launchd) => home().join("Library/LaunchAgents"),
        (None, Backend::Systemd) => home().join(".config/systemd/user"),
    }
}

fn require_native(backend: Backend, f: &Flags) -> Result<(), Fail> {
    if f.no_load || backend.native() {
        return Ok(());
    }
    fail(
        2,
        match backend {
            Backend::Launchd => "launchd 只在 macOS 上可用;在别的系统上只能加 --no-load 写文件",
            Backend::Systemd => "systemd 只在 Linux 上可用;在别的系统上只能加 --no-load 写文件",
        },
    )
}

fn write_file(path: &Path, text: &str) -> Result<(), Fail> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let written = std::fs::write(&tmp, text).and_then(|()| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        }
        std::fs::rename(&tmp, path)
    });
    written.map_err(|e| Fail(1, format!("写不了 {}: {e}", path.display())))?;
    println!("written {}", path.display());
    Ok(())
}

fn write_and_load(backend: Backend, f: &Flags, job: &Job) -> Result<i32, Fail> {
    require_native(backend, f)?;
    // Build the files first: a job that cannot be written leaves nothing.
    let files = match backend {
        Backend::Launchd => Vec::new(),
        Backend::Systemd => units(job)?,
    };
    let dir = jobs_dir(backend, f);
    let made = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::create_dir_all(log_path(&job.name).parent().unwrap_or(Path::new(".")))
    });
    made.map_err(|e| Fail(1, format!("建不了目录: {e}")))?;
    match backend {
        Backend::Launchd => {
            let label = format!("{LAUNCHD_PREFIX}{}", job.name);
            let path = dir.join(format!("{label}.plist"));
            write_file(&path, &plist(job))?;
            if f.no_load {
                return Ok(0);
            }
            // A loaded job of the same name goes first, or the new settings
            // would not take effect; failing when none is loaded is normal.
            launchctl(&["bootout", &format!("{}/{label}", launchd_domain())]);
            let (ok, out, err) =
                launchctl(&["bootstrap", &launchd_domain(), &path.to_string_lossy()]);
            if !ok {
                return fail(
                    1,
                    format!(
                        "launchctl bootstrap 失败: {}",
                        if err.is_empty() { out } else { err }
                    ),
                );
            }
            println!("loaded {label}");
        }
        Backend::Systemd => {
            let label = format!("{SYSTEMD_PREFIX}{}", job.name);
            if !f.no_load {
                stop_job(&label);
            }
            // An interval job that becomes a keepalive job leaves no timer.
            for ext in [".service", ".timer"] {
                let name = format!("{label}{ext}");
                if !files.iter().any(|(file, _)| *file == name) {
                    let _ = std::fs::remove_file(dir.join(name));
                }
            }
            for (file, text) in &files {
                write_file(&dir.join(file), text)?;
            }
            if f.no_load {
                return Ok(0);
            }
            let (ok, out, err) = systemctl(&["daemon-reload"]);
            if !ok {
                let detail = if err.is_empty() { out } else { err };
                let hint = if detail.contains("connect to bus") {
                    // No user manager: nobody is logged in and linger is off.
                    format!(
                        "。这个用户的 systemd 没有在运行:先执行 sudo loginctl enable-linger {},再重跑本命令(单元文件已写好)",
                        user_name()
                    )
                } else {
                    String::new()
                };
                return fail(
                    1,
                    format!("systemctl --user daemon-reload 失败: {detail}{hint}"),
                );
            }
            let target = files
                .iter()
                .map(|(file, _)| file.as_str())
                .find(|file| file.ends_with(".timer"))
                .unwrap_or(&files[0].0);
            let (ok, out, err) = systemctl(&["enable", "--now", target]);
            if !ok {
                return fail(
                    1,
                    format!(
                        "systemctl --user enable --now {target} 失败: {}",
                        if err.is_empty() { out } else { err }
                    ),
                );
            }
            println!("enabled {target}");
        }
    }
    Ok(0)
}

fn heartbeat(backend: Backend, argv: &[String]) -> Result<i32, Fail> {
    let f = flags(argv, &["--unit", "--hub", "--scripts", "--require"])?;
    let (Some(unit), Some(hub)) = (f.values.get("--unit"), f.values.get("--hub")) else {
        return fail(2, "heartbeat 需要 --unit <单元号> 与 --hub <ssh 目标>");
    };
    if !is_unit(unit) {
        return fail(2, format!("单元号不合法: {unit}"));
    }
    if !is_target(hub) {
        return fail(2, format!("--hub 不是合法的 ssh 目标: {hub}"));
    }
    if f.keepalive || !f.rest.is_empty() {
        return fail(2, "heartbeat 不接受这些参数");
    }
    let require = f.values.get("--require");
    if let Some(list) = require {
        let ok = !list.is_empty()
            && list
                .split(',')
                .all(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase()));
        if !ok {
            return fail(
                2,
                format!("--require 须写成逗号分隔的巡检项,如 agent,disk: {list}"),
            );
        }
    }
    // The job calls this binary, and so follows upgrades; `--scripts` asks
    // for that directory's script instead.
    let tool: Vec<String> = match f.values.get("--scripts") {
        Some(dir) => {
            let script = std::path::absolute(dir)
                .unwrap_or_else(|_| PathBuf::from(dir))
                .join("olp-unit.py");
            if !script.is_file() {
                eprintln!(
                    "{}: 提醒:{} 现在不存在,任务运行时需要它",
                    backend.prog(),
                    script.display()
                );
            }
            vec![
                "python3".to_string(),
                "-B".to_string(),
                script.to_string_lossy().into_owned(),
            ]
        }
        None => vec![
            super::repo::self_exe(),
            "olp".to_string(),
            "unit".to_string(),
        ],
    };
    let mut env = f.env.clone();
    // The heartbeat asks herdr which inner loops run; record where herdr is
    // now unless the caller already said.
    if !env.iter().any(|pair| pair.starts_with("OLP_HERDR=")) {
        match which("herdr") {
            Some(path) => {
                let real = std::fs::canonicalize(&path).unwrap_or(path);
                env.push(format!("OLP_HERDR={}", real.display()));
            }
            None => eprintln!(
                "{}: 提醒:现在的 PATH 上找不到 herdr;心跳的 agent 巡检会失败。装好后重跑,或用 --env OLP_HERDR=<路径>",
                backend.prog()
            ),
        }
    }
    // The heartbeat asks the reviewer CLIs whether they are logged in. A
    // job's PATH is not the shell's: the CLI may be outside it, or another
    // file of the same name may come first there (seen: an installer stub
    // that goes to the network and never returns). So record the very
    // executables this shell resolves, and put their directories, and
    // `node`'s, first on the job's PATH for what they start in turn.
    for (name, key) in [("claude", "OLP_CLAUDE"), ("codex", "OLP_CODEX")] {
        if env.iter().any(|pair| pair.starts_with(&format!("{key}="))) {
            continue;
        }
        if let Some(path) = which(name).and_then(|p| std::path::absolute(p).ok()) {
            env.push(format!("{key}={}", path.display()));
        }
    }
    if !env.iter().any(|pair| pair.starts_with("PATH=")) {
        let base = default_path(backend);
        // Found directories first, in the order this shell would search.
        let mut dirs: Vec<String> = Vec::new();
        for name in ["claude", "codex", "node"] {
            let dir = which(name).and_then(|p| p.parent().map(Path::to_path_buf));
            if let Some(dir) = dir.and_then(|d| d.to_str().map(str::to_string)) {
                if dir.starts_with('/') && !dir.contains(':') && !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
        for dir in base.split(':') {
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_string());
            }
        }
        if dirs.join(":") != base {
            env.push(format!("PATH={}", dirs.join(":")));
        }
    }
    // The same ssh the other commands use: `OLP_SSH` when it is set, here
    // or with `--env`. A job has no shell profile, so the path is absolute.
    let ssh = env
        .iter()
        .find_map(|pair| pair.strip_prefix("OLP_SSH="))
        .map(str::to_string)
        .or_else(|| std::env::var("OLP_SSH").ok())
        .filter(|v| !v.is_empty())
        .map(|v| match which(&v) {
            Some(found) if !v.contains('/') => found.to_string_lossy().into_owned(),
            _ => std::path::absolute(&v)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(v),
        })
        .unwrap_or_else(|| "ssh".to_string());
    // One shell word, quoted only when it has to be.
    let quote = |word: &str| {
        let plain = !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
        if plain {
            word.to_string()
        } else {
            format!("'{}'", word.replace('\'', "'\"'\"'"))
        }
    };
    let shell = |words: &[&str]| words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
    let mut send: Vec<&str> = tool.iter().map(String::as_str).collect();
    send.extend(["heartbeat", "--unit", unit]);
    if let Some(list) = require {
        send.extend(["--require", list]);
    }
    let pipeline = if hub == "local" {
        // Single machine: hand the heartbeat to the hub side directly.
        let mut take: Vec<&str> = tool.iter().map(String::as_str).collect();
        take.extend(["receive", "--unit", unit]);
        format!("{} | {}", shell(&send), shell(&take))
    } else {
        format!(
            "{} | {} -o BatchMode=yes -o ConnectTimeout=10 {} receive",
            shell(&send),
            shell(&[&ssh]),
            shell(&[hub])
        )
    };
    let command = ["/bin/sh", "-c", &pipeline].map(str::to_string);
    let job = job(
        backend,
        "heartbeat",
        &command,
        Some(f.interval.unwrap_or(60)),
        false,
        &env,
    )?;
    write_and_load(backend, &f, &job)
}

fn uninstall(backend: Backend, argv: &[String]) -> Result<i32, Fail> {
    let f = flags(argv, &[])?;
    let [name] = f.rest.as_slice() else {
        return fail(2, "uninstall 需要一个任务名");
    };
    if !is_name(name) {
        return fail(
            2,
            format!("名字不合法: {name}(小写字母、数字、连字符,以字母或数字开头)"),
        );
    }
    require_native(backend, &f)?;
    let dir = jobs_dir(backend, &f);
    let files: Vec<PathBuf> = match backend {
        Backend::Launchd => {
            let label = format!("{LAUNCHD_PREFIX}{name}");
            if !f.no_load {
                launchctl(&["bootout", &format!("{}/{label}", launchd_domain())]);
            }
            vec![dir.join(format!("{label}.plist"))]
        }
        Backend::Systemd => {
            let label = format!("{SYSTEMD_PREFIX}{name}");
            if !f.no_load {
                stop_job(&label);
            }
            vec![
                dir.join(format!("{label}.timer")),
                dir.join(format!("{label}.service")),
            ]
        }
    };
    let mut removed = false;
    for path in &files {
        if path.is_file() && std::fs::remove_file(path).is_ok() {
            println!("removed {}", path.display());
            removed = true;
        }
    }
    if !removed {
        println!("{} 不存在", files[files.len() - 1].display());
    }
    if backend == Backend::Systemd && !f.no_load {
        systemctl(&["daemon-reload"]);
    }
    Ok(0)
}

fn list(backend: Backend, argv: &[String]) -> Result<i32, Fail> {
    let f = flags(argv, &[])?;
    let dir = jobs_dir(backend, &f);
    let (prefix, suffix) = match backend {
        Backend::Launchd => (LAUNCHD_PREFIX, ".plist"),
        Backend::Systemd => (SYSTEMD_PREFIX, ".service"),
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|file| Some(file.strip_prefix(prefix)?.strip_suffix(suffix)?.to_string()))
        .collect();
    names.sort();
    let can_ask = !f.no_load && backend.native();
    let mut shown = 0;
    for name in &names {
        let path = dir.join(format!("{prefix}{name}{suffix}"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        match backend {
            Backend::Launchd => {
                shown += 1;
                if !text.contains("<plist") {
                    println!("{name}\t无法解析 {}", path.display());
                    continue;
                }
                let after = |key: &str| {
                    text.split_once(&format!("<key>{key}</key>"))
                        .map(|(_, rest)| rest.split("<key>").next().unwrap_or_default().to_string())
                };
                let mode = if after("KeepAlive").is_some() {
                    "常驻".to_string()
                } else {
                    let seconds = after("StartInterval")
                        .and_then(|v| {
                            Some(
                                v.split_once("<integer>")?
                                    .1
                                    .split_once("</integer>")?
                                    .0
                                    .to_string(),
                            )
                        })
                        .unwrap_or_else(|| "None".to_string());
                    format!("每 {seconds} 秒")
                };
                let command: Vec<String> = after("ProgramArguments")
                    .unwrap_or_default()
                    .split("<string>")
                    .skip(1)
                    .filter_map(|part| part.split_once("</string>").map(|(s, _)| unxml(s)))
                    .collect();
                let state = if can_ask {
                    let label = format!("{}/{LAUNCHD_PREFIX}{name}", launchd_domain());
                    if launchctl(&["print", &label]).0 {
                        "已加载"
                    } else {
                        "未加载"
                    }
                } else {
                    "未查询"
                };
                println!("{name}\t{mode}\t{state}\t{}", command.join(" "));
            }
            Backend::Systemd => {
                if !text.contains(MARKER) {
                    continue; // not one of ours
                }
                shown += 1;
                let command = text
                    .lines()
                    .find_map(|l| l.strip_prefix(COMMAND_MARK))
                    .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok());
                let label = format!("{SYSTEMD_PREFIX}{name}");
                let timer = dir.join(format!("{label}.timer"));
                let (mode, target) = if timer.is_file() {
                    let seconds = std::fs::read_to_string(&timer)
                        .unwrap_or_default()
                        .lines()
                        .find_map(|l| l.strip_prefix("OnUnitActiveSec=").map(str::to_string))
                        .unwrap_or_else(|| "?".to_string());
                    (format!("每 {seconds} 秒"), format!("{label}.timer"))
                } else {
                    ("常驻".to_string(), format!("{label}.service"))
                };
                let state = if can_ask {
                    let active = systemctl(&["is-active", &target]).1;
                    if active == "active" {
                        "运行中".to_string()
                    } else {
                        format!(
                            "未运行({})",
                            if active.is_empty() {
                                "unknown"
                            } else {
                                &active
                            }
                        )
                    }
                } else {
                    "未查询".to_string()
                };
                println!(
                    "{name}\t{mode}\t{state}\t{}",
                    command
                        .map(|c| c.join(" "))
                        .unwrap_or_else(|| "(命令不可读)".to_string())
                );
            }
        }
    }
    if shown == 0 {
        println!("没有本工具注册的任务");
    }
    Ok(0)
}

fn doctor(backend: Backend, argv: &[String]) -> Result<i32, Fail> {
    let f = flags(argv, &["--pmset-output", "--linger-output"])?;
    let read = |path: &String| {
        std::fs::read_to_string(path).map_err(|e| Fail(2, format!("读不了 {path}: {e}")))
    };
    let mut problems = 0;
    match backend {
        Backend::Launchd => {
            let text = match f.values.get("--pmset-output") {
                Some(path) => read(path)?,
                None if !backend.native() => {
                    return fail(2, "doctor 检查的是 macOS 的电源设置;在别的系统上不适用");
                }
                None => {
                    let (ok, out, err) = output(Command::new("pmset").arg("-g"));
                    if !ok {
                        return fail(1, format!("pmset -g 失败: {err}"));
                    }
                    out
                }
            };
            // Settings under "Currently in use:", or all of it when absent.
            let mut values = BTreeMap::new();
            let mut active = !text.contains("Currently in use:");
            for line in text.lines() {
                let line = line.trim();
                if line.ends_with(':') {
                    active = line == "Currently in use:";
                } else if active {
                    let mut words = line.split_whitespace();
                    if let (Some(key), Some(value)) = (words.next(), words.next()) {
                        values.insert(key.to_string(), value.to_string());
                    }
                }
            }
            for (key, want, meaning, fix) in [
                ("sleep", "0", "机器不自动休眠", "sudo pmset -a sleep 0"),
                (
                    "autorestart",
                    "1",
                    "断电恢复后自动开机",
                    "sudo pmset -a autorestart 1",
                ),
            ] {
                match values.get(key).map(String::as_str) {
                    Some(got) if got == want => println!("ok    {meaning}({key} = {got})"),
                    None => {
                        problems += 1;
                        println!(
                            "??    {meaning}:pmset 没有列出 {key}(笔记本没有 autorestart)。修正:{fix}"
                        );
                    }
                    Some(got) => {
                        problems += 1;
                        println!("FAIL  {meaning}:{key} = {got},应为 {want}。修正:{fix}");
                    }
                }
            }
            println!("提醒  launchd 的用户任务在登录后才启动:单元要设成开机自动登录。");
        }
        Backend::Systemd => {
            let user = user_name();
            let text = match f.values.get("--linger-output") {
                Some(path) => read(path)?,
                None if !backend.native() => {
                    return fail(2, "doctor 检查的是 systemd 的 linger;在别的系统上不适用");
                }
                None => match Command::new("loginctl")
                    .args(["show-user", &user, "--property=Linger"])
                    .output()
                {
                    Err(_) => {
                        return fail(1, "找不到 loginctl:这台机器不是 systemd 系统,本工具不适用");
                    }
                    // A user with no session and no linger is unknown to logind.
                    Ok(out) if !out.status.success() => "Linger=no\n".to_string(),
                    Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
                },
            };
            let got = text.lines().map(str::trim).find_map(|line| {
                line.strip_prefix("Linger=")
                    .or(matches!(line, "yes" | "no").then_some(line))
            });
            let fix = format!("sudo loginctl enable-linger {user}");
            match got {
                Some("yes") => {
                    println!("ok    用户单元在开机时启动,不需要登录(Linger = yes,用户 {user})")
                }
                Some("no") => {
                    problems += 1;
                    println!("FAIL  用户单元只在 {user} 登录期间运行:Linger = no。修正:{fix}");
                }
                _ => {
                    problems += 1;
                    println!("??    读不到 {user} 的 Linger 设置。修正:{fix}");
                }
            }
            println!(
                "提醒  桌面发行版可能自动休眠:单元要关掉(sudo systemctl mask sleep.target suspend.target)。"
            );
        }
    }
    Ok(i32::from(problems > 0))
}

/// `argv` is what follows `olp keepalive`.
pub fn run(argv: &[String]) -> i32 {
    let usage = "用法: octoscode olp keepalive <动作>\n  heartbeat --unit <单元号> --hub <ssh 目标|local> [--interval 60] [--require 项,项] [--env K=V]…\n  install <名字> (--interval 秒 | --keepalive) [--env K=V]… -- <命令> [参数…]\n  uninstall <名字>\n  list\n  render <名字> (--interval 秒 | --keepalive) [--env K=V]… -- <命令> [参数…]\n  doctor\n  install / heartbeat / uninstall / list 都可加 --dir <目录>、--no-load\nmacOS 上是 launchd 任务,Linux 上是 systemd 用户单元。\n退出码:0 成功;1 服务管理器失败或 doctor 发现问题;2 用法错误";
    let backend = match std::env::var("OLP_KEEPALIVE_BACKEND").as_deref() {
        Ok("launchd") => Backend::Launchd,
        Ok("systemd") => Backend::Systemd,
        _ if cfg!(target_os = "macos") => Backend::Launchd,
        _ if cfg!(target_os = "linux") => Backend::Systemd,
        _ => {
            eprintln!("octoscode olp keepalive: 只支持 macOS(launchd)与 Linux(systemd)");
            return 2;
        }
    };
    // The job's own command follows `--`; split it off so its flags are
    // never mistaken for ours.
    let (argv, command) = match argv.iter().position(|w| w == "--") {
        Some(at) => (&argv[..at], argv[at + 1..].to_vec()),
        None => (argv, Vec::new()),
    };
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
    if !command.is_empty() && !matches!(action, "install" | "render") {
        eprintln!("{}: {action} 不接受 -- 之后的命令", backend.prog());
        return 2;
    }
    let result = match action {
        "install" | "render" => flags(rest, &[]).and_then(|f| {
            let [name] = f.rest.as_slice() else {
                return fail(2, format!("{action} 需要一个任务名"));
            };
            let job = job(backend, name, &command, f.interval, f.keepalive, &f.env)?;
            if action == "install" {
                return write_and_load(backend, &f, &job);
            }
            if f.dir.is_some() || f.no_load {
                return fail(2, "render 不接受 --dir、--no-load");
            }
            match backend {
                Backend::Launchd => print!("{}", plist(&job)),
                Backend::Systemd => {
                    for (file, text) in units(&job)? {
                        print!("# --- {file}\n{text}");
                    }
                }
            }
            Ok(0)
        }),
        "heartbeat" => heartbeat(backend, rest),
        "uninstall" => uninstall(backend, rest),
        "list" => list(backend, rest),
        "doctor" => doctor(backend, rest),
        other => fail(2, format!("没有这个动作: {other}\n{usage}")),
    };
    match result {
        Ok(code) => code,
        Err(Fail(code, why)) => {
            eprintln!("{}: {why}", backend.prog());
            code
        }
    }
}
