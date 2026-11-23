//! `octoscode olp doctor` — do the OctoLoop commands written down in
//! various places still work?
//!
//! Git hooks, a work repository's local push settings, keepalive jobs and
//! ssh forced commands each hold a command that runs later. One that names
//! a script inside a single version's tool directory keeps running old
//! code after an upgrade, and fails once that directory is gone; one that
//! names `<octoscode> olp …` follows upgrades.
//!
//! Checked: forced commands in `~/.ssh/authorized_keys`; the hooks of hub
//! repositories (`--repo`, and those the forced commands serve); the local
//! push settings and the hub remote of work repositories (`--work`, and the
//! current directory); this machine's keepalive jobs; where the skill reads
//! its text from; unit heartbeats in an old format; processes still running
//! a binary that has since been replaced.
//!
//! Each item is ok, warn or fail, with a fix for the last two. Read-only.
//! Exit codes: 0 no fail; 1 a fail; 2 usage.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const HOOKS: [&str; 2] = ["pre-receive", "post-receive"];
const COMPONENTS: [&str; 3] = ["octoscode", "octos", "herdr"];

/// One shell word, quoted only when it has to be.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\"'\"'"))
    }
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn absolute(path: &str) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_script(name: &str) -> bool {
    let stem = name
        .strip_suffix(".py")
        .or_else(|| name.strip_suffix(".sh"));
    stem.and_then(|s| s.strip_prefix("olp-"))
        .is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        })
}

/// Where a command line calls an OctoLoop tool: (is the CLI, path, index).
fn calls(tokens: &[String]) -> Vec<(bool, &str, usize)> {
    tokens
        .iter()
        .enumerate()
        .filter_map(|(i, token)| {
            if base(token) == "octoscode" && tokens.get(i + 1).map(String::as_str) == Some("olp") {
                Some((true, token.as_str(), i))
            } else if is_script(base(token)) {
                Some((false, token.as_str(), i))
            } else {
                None
            }
        })
        .collect()
}

/// Directories that hold one tool directory per octoscode version.
fn bundle_roots() -> Vec<String> {
    let mut roots = vec![home().join(".octos/outer/olp-bundle")];
    if let Some(extra) = std::env::var_os("OCTOSCODE_OLP_BUNDLE").filter(|v| !v.is_empty()) {
        roots.push(PathBuf::from(extra));
    }
    let mut out: Vec<String> = roots
        .iter()
        .flat_map(|r| {
            [
                absolute(&r.to_string_lossy()),
                real(r).to_string_lossy().into_owned(),
            ]
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A path inside one version's tool directory (not the `current` link).
fn pinned(path: &str, roots: &[String]) -> bool {
    let path = absolute(path);
    for root in roots {
        if let Some(rest) = path.strip_prefix(&format!("{root}/")) {
            return rest.split('/').next() != Some("current");
        }
    }
    path.contains("/olp-bundle/") && !path.contains("/olp-bundle/current/")
}

fn worse(a: &'static str, b: &'static str) -> &'static str {
    let rank = |s: &str| match s {
        "ok" => 0,
        "warn" => 1,
        _ => 2,
    };
    if rank(b) > rank(a) { b } else { a }
}

/// (status, detail) for one written command; `None` when it calls no tool.
fn judge(tokens: &[String], roots: &[String]) -> Option<(&'static str, String)> {
    let found = calls(tokens);
    if found.is_empty() {
        return None;
    }
    let (mut status, mut notes) = ("ok", Vec::new());
    for (cli, path, _) in found {
        if !Path::new(path).is_absolute() {
            if cli {
                status = worse(status, "warn");
                notes.push(
                    "按 PATH 找 octoscode;钩子、保活任务、ssh 强制命令的 PATH 很短,写绝对路径更稳"
                        .to_string(),
                );
            }
        } else if !Path::new(path).exists() {
            status = "fail";
            notes.push(format!("不存在:{path}"));
        } else if !cli && pinned(path, roots) {
            status = worse(status, "warn");
            notes.push(format!(
                "固定在某一版本的工具目录,升级 octoscode 后仍跑旧代码:{path}"
            ));
        } else if !cli {
            notes.push(format!("调用仓库里的脚本(随 git pull 更新):{path}"));
        } else {
            notes.push(format!("调用 {path} olp"));
        }
    }
    Some((status, notes.join(";")))
}

/// The value after `name` in a token list.
fn option<'a>(tokens: &'a [String], name: &str) -> Option<&'a str> {
    tokens
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

/// The same command written as `<octoscode> olp …`.
fn as_olp(tokens: &[String]) -> Option<String> {
    for (cli, path, i) in calls(tokens) {
        if cli {
            continue;
        }
        let action = tokens.get(i + 1).map(String::as_str).unwrap_or_default();
        let words: &[&str] = match (base(path), action) {
            ("olp-unit.py", "receive") => &["unit", "receive"],
            ("olp-unit.py", "heartbeat") => &["unit", "heartbeat"],
            ("olp-git-hub.py", "serve") => &["repo", "serve"],
            ("olp-git-hub.py", "hook") => &["repo", "hook"],
            ("olp-git-hub.py", "local-pack") => &["repo", "local-pack"],
            ("olp-board-remote.py", "serve") => &["remote", "serve"],
            ("olp-answer.py", "serve") => &["ask", "serve"],
            _ => continue,
        };
        let exe = super::repo::self_exe();
        let mut line = vec![quote(&exe), "olp".to_string()];
        line.extend(words.iter().map(|w| w.to_string()));
        line.extend(tokens[(i + 2).min(tokens.len())..].iter().map(|t| quote(t)));
        return Some(line.join(" "));
    }
    None
}

struct Report {
    items: Vec<Value>,
    roots: Vec<String>,
}

impl Report {
    fn add(&mut self, kind: &str, place: &str, status: &str, detail: &str, fix: Option<String>) {
        self.items.push(json!({
            "kind": kind, "where": place, "status": status, "detail": detail,
            "fix": if status == "ok" { None } else { fix },
        }));
    }

    /// Judge one written command line. Returns its tokens, or `None` when
    /// it cannot be split.
    fn line(
        &mut self,
        kind: &str,
        place: &str,
        line: &str,
        fix: impl Fn(&[String]) -> Option<String>,
    ) -> Option<Vec<String>> {
        let Some(tokens) = shlex::split(line) else {
            self.add(
                kind,
                place,
                "fail",
                &format!("命令无法解析:{line}"),
                fix(&[]),
            );
            return None;
        };
        if let Some((status, detail)) = judge(&tokens, &self.roots) {
            self.add(kind, place, status, &detail, fix(&tokens));
        }
        Some(tokens)
    }
}

fn git(args: &[&str], cwd: &Path) -> String {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

/// `command="…"` at the start of a key line's options.
fn forced_command(line: &str) -> Option<String> {
    let mut at = 0;
    loop {
        let rest = &line[at..];
        let found = rest.find("command=\"")?;
        let before = &line[..at + found];
        let start = at + found + "command=\"".len();
        if before.is_empty() || before.ends_with(',') {
            let mut out = String::new();
            let mut chars = line[start..].chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return Some(out),
                    '\\' => match chars.next() {
                        Some('"') => out.push('"'),
                        Some(other) => {
                            out.push('\\');
                            out.push(other);
                        }
                        None => return None,
                    },
                    other => out.push(other),
                }
            }
            return None;
        }
        at = start;
    }
}

/// Forced commands; returns the hub repositories they serve.
fn check_authorized_keys(report: &mut Report, path: &Path) -> Vec<String> {
    let mut repos = Vec::new();
    let Ok(bytes) = std::fs::read(path) else {
        return repos;
    };
    for (number, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let Some(command) = forced_command(line) else {
            continue;
        };
        let place = format!("{}:{}", path.display(), number + 1);
        let tokens = report.line("ssh 强制命令", &place, &command, |tokens| {
            as_olp(tokens).map(|line| format!("把这一行的强制命令改成 command=\"{line}\""))
        });
        if let Some(tokens) = tokens.filter(|t| !calls(t).is_empty()) {
            if let Some(repo) = option(&tokens, "--repo") {
                if tokens.iter().any(|t| t == "serve") && !repos.iter().any(|r| r == repo) {
                    repos.push(repo.to_string());
                }
            }
        }
    }
    repos
}

fn check_hub_repo(report: &mut Report, repo: &str) {
    let dir = Path::new(repo);
    if !dir.is_dir() {
        report.add("hub 钩子", repo, "fail", "裸仓库不存在", None);
        return;
    }
    let board = git(&["config", "olp.board"], dir);
    let fix = format!(
        "octoscode olp repo init {} --board {}",
        quote(repo),
        quote(if board.is_empty() { "<黑板>" } else { &board })
    );
    for name in HOOKS {
        let path = dir.join("hooks").join(name);
        let place = format!("{repo} {name}");
        let Ok(text) = std::fs::read_to_string(&path) else {
            report.add("hub 钩子", &place, "fail", "钩子缺失", Some(fix.clone()));
            continue;
        };
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        };
        #[cfg(not(unix))]
        let executable = true;
        if !executable {
            report.add(
                "hub 钩子",
                &place,
                "fail",
                "钩子不可执行",
                Some(fix.clone()),
            );
            continue;
        }
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("exec "))
            .unwrap_or_default();
        let tokens = report.line("hub 钩子", &place, line, |_| Some(fix.clone()));
        if tokens.is_some_and(|t| calls(&t).is_empty()) {
            report.add(
                "hub 钩子",
                &place,
                "fail",
                "钩子里没有调用 OctoLoop 的命令",
                Some(fix.clone()),
            );
        }
    }
}

fn check_work_repo(report: &mut Report, work: &str, remote: &str, explicit: bool) {
    let top = if Path::new(work).is_dir() {
        git(&["rev-parse", "--show-toplevel"], Path::new(work))
    } else {
        String::new()
    };
    if top.is_empty() {
        if explicit {
            report.add("本机推送", work, "fail", "不是 git 工作仓库", None);
        }
        return;
    }
    let dir = Path::new(&top);
    let mut found = false;
    for key in ["receivepack", "uploadpack"] {
        let line = git(&["config", &format!("remote.{remote}.{key}")], dir);
        if line.is_empty() {
            continue;
        }
        found = true;
        let place = format!("{top} remote.{remote}.{key}");
        report.line("本机推送", &place, &line, |tokens| {
            Some(format!(
                "cd {} && octoscode olp repo connect-local --unit {} --repo {} --remote {}",
                quote(&top),
                option(tokens, "--unit").unwrap_or("<单元>"),
                quote(option(tokens, "--repo").unwrap_or("<裸仓库>")),
                quote(remote)
            ))
        });
    }
    if explicit && !found {
        report.add(
            "本机推送",
            &top,
            "ok",
            &format!("remote {remote} 没有本机推送设置(多机部署走 ssh,不需要)"),
            None,
        );
    }
    // A unit's clone that copied the hub's own local path as its hub remote:
    // on the unit that path is not the bare repository.
    let url = git(&["remote", "get-url", remote], dir);
    let local = url
        .strip_prefix("file://")
        .or_else(|| url.starts_with(['/', '~', '.']).then_some(url.as_str()));
    let Some(local) = local else { return }; // ssh/https: not checked offline
    let expanded = match local.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => dir.join(local),
    };
    let path = absolute(&expanded.to_string_lossy());
    let bare = Path::new(&path).is_dir()
        && git(&["rev-parse", "--is-bare-repository"], Path::new(&path)) == "true";
    let place = format!("{top} remote.{remote}.url");
    if bare {
        report.add(
            "hub 远程",
            &place,
            "ok",
            &format!("本机的裸仓库 {path}"),
            None,
        );
    } else {
        report.add(
            "hub 远程",
            &place,
            "fail",
            &format!("指向本机路径 {path},但这台机器上没有这个裸仓库(多半是照抄了 hub 上的路径)"),
            Some(format!(
                "cd {} && git remote set-url {} <hub 的 ssh 地址,例如 olp-hub-git:/path/hub.git>",
                quote(&top),
                quote(remote)
            )),
        );
    }
}

fn keepalive_fix(name: &str, tokens: &[String]) -> String {
    if name != "heartbeat" {
        return format!("用 octoscode olp keepalive install {name} … 重装这个任务");
    }
    let receive = tokens.iter().position(|t| t == "receive");
    let hub = match receive.filter(|at| *at > 0 && tokens.iter().any(|t| t == "ssh")) {
        Some(at) => tokens[at - 1].as_str(),
        None => "local",
    };
    format!(
        "octoscode olp keepalive heartbeat --unit {} --hub {}{}",
        option(tokens, "--unit").unwrap_or("<单元>"),
        quote(hub),
        option(tokens, "--require")
            .map(|r| format!(" --require {r}"))
            .unwrap_or_default()
    )
}

/// `sh -c <pipeline>` is judged by its pipeline.
fn job_line(args: &[String]) -> String {
    match args {
        [shell, flag, line] if flag == "-c" && matches!(base(shell), "sh" | "bash") => line.clone(),
        _ => args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "),
    }
}

fn check_keepalive(report: &mut Report) {
    let entries = |dir: &Path, prefix: &str, suffix: &str| {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|f| Some(f.strip_prefix(prefix)?.strip_suffix(suffix)?.to_string()))
            .collect();
        names.sort();
        names
    };
    let agents = home().join("Library/LaunchAgents");
    for name in entries(&agents, "org.octos.olp.", ".plist") {
        let path = agents.join(format!("org.octos.olp.{name}.plist"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let args: Option<Vec<String>> = text
            .split_once("<key>ProgramArguments</key>")
            .filter(|_| text.contains("<plist"))
            .map(|(_, rest)| {
                rest.split("<key>")
                    .next()
                    .unwrap_or_default()
                    .split("<string>")
                    .skip(1)
                    .filter_map(|part| part.split_once("</string>"))
                    .map(|(s, _)| {
                        s.replace("&lt;", "<")
                            .replace("&gt;", ">")
                            .replace("&apos;", "'")
                            .replace("&quot;", "\"")
                            .replace("&amp;", "&")
                    })
                    .collect()
            });
        match args {
            None => report.add(
                "保活任务",
                &path.to_string_lossy(),
                "fail",
                "无法解析",
                Some(keepalive_fix(&name, &[])),
            ),
            Some(args) => {
                report.line(
                    "保活任务",
                    &format!("launchd {name}"),
                    &job_line(&args),
                    |t| Some(keepalive_fix(&name, t)),
                );
            }
        }
    }
    let units = home().join(".config/systemd/user");
    for name in entries(&units, "olp-", ".service") {
        let path = units.join(format!("olp-{name}.service"));
        let args = std::fs::read_to_string(&path).ok().and_then(|text| {
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("# olp-command: "))?;
            serde_json::from_str::<Vec<String>>(line).ok()
        });
        match args {
            None => report.add(
                "保活任务",
                &path.to_string_lossy(),
                "fail",
                "找不到它记下的命令",
                Some(keepalive_fix(&name, &[])),
            ),
            Some(args) => {
                report.line(
                    "保活任务",
                    &format!("systemd {name}"),
                    &job_line(&args),
                    |t| Some(keepalive_fix(&name, t)),
                );
            }
        }
    }
}

/// This user's octoscode/octos/herdr processes whose executable has been
/// replaced since they started: (pid, path).
fn stale_processes() -> Vec<(u32, String)> {
    let me = std::process::id();
    let mut out = Vec::new();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let link = entry.path().join("exe");
            let (Ok(exe), Ok(running)) = (std::fs::read_link(&link), std::fs::metadata(&link))
            else {
                continue;
            };
            let exe = exe.to_string_lossy().into_owned();
            let deleted = exe.ends_with(" (deleted)");
            let path = exe.trim_end_matches(" (deleted)").to_string();
            if pid == me || !COMPONENTS.contains(&base(&path)) {
                continue;
            }
            // The file at the path is no longer the one the process runs.
            let replaced = deleted
                || std::fs::metadata(&path)
                    .map(|now| (now.ino(), now.dev()) != (running.ino(), running.dev()))
                    .unwrap_or(true);
            if replaced {
                out.push((pid, path));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let text = |cmd: &mut Command| {
            cmd.output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        };
        let uid = text(Command::new("id").arg("-u"));
        let listing = text(Command::new("ps").args(["-U", uid.trim(), "-o", "pid=,lstart=,comm="]));
        for line in listing.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let Some(pid) = parts.first().and_then(|p| p.parse::<u32>().ok()) else {
                continue;
            };
            if parts.len() < 7 || pid == me {
                continue;
            }
            // `comm` is the name it was started with, often bare: ask for the
            // executable's real path when it is not one.
            let comm = parts[6..].join(" ");
            if !COMPONENTS.contains(&base(&comm)) {
                continue;
            }
            let path = if Path::new(&comm).is_absolute() {
                comm
            } else {
                let files = text(Command::new("lsof").args([
                    "-p",
                    &pid.to_string(),
                    "-a",
                    "-d",
                    "txt",
                    "-Fn",
                ]));
                match files.lines().find_map(|l| l.strip_prefix('n')) {
                    Some(path) if COMPONENTS.contains(&base(path)) => path.to_string(),
                    _ => continue,
                }
            };
            let started = chrono::NaiveDateTime::parse_from_str(
                &parts[1..6].join(" "),
                "%a %b %d %H:%M:%S %Y",
            )
            .ok()
            .and_then(|at| at.and_local_timezone(chrono::Local).single());
            let Some(started) = started else { continue };
            // The file at the path is newer than the process, or gone.
            let replaced = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|modified| {
                    chrono::DateTime::<chrono::Local>::from(modified)
                        > started + chrono::Duration::seconds(1)
                })
                .unwrap_or(true);
            if replaced {
                out.push((pid, path));
            }
        }
    }
    let _ = me;
    out
}

fn check_running_versions(report: &mut Report) {
    let mut stale: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (pid, path) in stale_processes() {
        stale.entry(path).or_default().push(pid);
    }
    for (path, mut pids) in stale {
        pids.sort_unstable();
        let shown: Vec<String> = pids.iter().take(12).map(u32::to_string).collect();
        report.add(
            "运行中的旧版本",
            &path,
            "warn",
            &format!(
                "{} 个进程在 {} 更新之前启动,还在跑旧代码(PID {}{})",
                pids.len(),
                base(&path),
                shown.join(", "),
                if pids.len() > 12 { " …" } else { "" }
            ),
            Some(
                "逐个在合适的时候重启这些会话或服务;常驻任务用 octoscode olp keepalive 重新加载"
                    .to_string(),
            ),
        );
    }
}

/// Heartbeats a pre-v3 unit wrote lack the agent/full checks and the agents
/// list, yet looked healthy.
fn check_heartbeats(report: &mut Report) {
    let beats = super::unit::units_dir().join("heartbeat");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&beats)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    for path in files {
        let unit = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(beat) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            report.add("单元心跳", &unit, "warn", "最近一次心跳读不出", None);
            continue;
        };
        let names: Vec<&str> = beat["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["name"].as_str())
            .collect();
        let mut lacking: Vec<&str> = ["agent", "full"]
            .into_iter()
            .filter(|c| !names.contains(c))
            .collect();
        if beat.get("agents").is_none() {
            lacking.push("agents 字段");
        }
        if lacking.is_empty() {
            report.add(
                "单元心跳",
                &unit,
                "ok",
                "心跳带 v3 巡检项(agent、full)",
                None,
            );
        } else {
            report.add(
                "单元心跳",
                &unit,
                "warn",
                &format!(
                    "心跳是旧版本写的,缺 {}:看起来健康不代表内环在跑",
                    lacking.join("、")
                ),
                Some(format!(
                    "在单元 {unit} 上升级 octoscode 后重装心跳任务:octoscode olp keepalive heartbeat --unit {unit} --hub <hub>"
                )),
            );
        }
    }
}

fn check_octoscode_home(report: &mut Report) {
    let path = home().join(".octos/outer/octoscode-home");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let recorded = text.trim();
    let place = path.to_string_lossy().into_owned();
    let fix = Some("octoscode olp skill install".to_string());
    if !Path::new(recorded)
        .join("skills/octoloop/AGENTS.md")
        .is_file()
    {
        report.add(
            "skill",
            &place,
            "fail",
            &format!("记下的位置没有规程正文:{recorded}"),
            fix,
        );
    } else if pinned(&format!("{recorded}/"), &report.roots) {
        report.add(
            "skill",
            &place,
            "warn",
            &format!("固定在某一版本的工具目录,升级后仍读旧正文:{recorded}"),
            fix,
        );
    } else {
        report.add(
            "skill",
            &place,
            "ok",
            &format!("规程正文在 {recorded}"),
            None,
        );
    }
}

/// `argv` is what follows `olp doctor`.
pub fn run(argv: &[String]) -> i32 {
    let usage = "用法: octoscode olp doctor [--repo <裸仓库>]... [--work <工作仓库>]... [--remote hub] [--json]\n\n检查钩子、保活任务、ssh 强制命令等处写下的 OctoLoop 命令还能不能用。只读。\n每项给出 ok / warn / fail;warn 与 fail 附修复命令。\n退出码:0 没有 fail;1 有 fail;2 用法错误";
    let (mut repos, mut works, mut remote, mut as_json) =
        (Vec::new(), Vec::new(), "hub".to_string(), false);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| words.next().cloned());
        let ok = match name {
            "-h" | "--help" => {
                println!("{usage}");
                return 0;
            }
            "--json" => {
                as_json = true;
                true
            }
            "--repo" => value().map(|v| repos.push(absolute(&v))).is_some(),
            "--work" => value().map(|v| works.push(absolute(&v))).is_some(),
            "--remote" => value().map(|v| remote = v).is_some(),
            _ => false,
        };
        if !ok {
            eprintln!("octoscode olp doctor: 参数不对: {word}\n{usage}");
            return 2;
        }
    }
    let mut report = Report {
        items: Vec::new(),
        roots: bundle_roots(),
    };
    let mut hubs = check_authorized_keys(&mut report, &home().join(".ssh/authorized_keys"));
    for repo in repos {
        if !hubs.contains(&repo) {
            hubs.push(repo);
        }
    }
    for repo in &hubs {
        check_hub_repo(&mut report, repo);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut all: Vec<(String, bool)> = works.into_iter().map(|w| (w, true)).collect();
    if !all.iter().any(|(w, _)| real(Path::new(w)) == real(&cwd)) {
        all.push((cwd.to_string_lossy().into_owned(), false));
    }
    for (work, explicit) in &all {
        check_work_repo(&mut report, work, &remote, *explicit);
    }
    check_keepalive(&mut report);
    check_octoscode_home(&mut report);
    check_heartbeats(&mut report);
    check_running_versions(&mut report);

    let writes = super::repo::self_exe();
    let count = |status: &str| {
        report
            .items
            .iter()
            .filter(|i| i["status"] == status)
            .count()
    };
    let (ok, warn, failed) = (count("ok"), count("warn"), count("fail"));
    if as_json {
        let doc = json!({
            "octoscode": writes,
            "counts": {"ok": ok, "warn": warn, "fail": failed},
            "checks": report.items,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
        );
    } else {
        println!("新写下的命令将调用:{writes} olp …");
        if report.items.is_empty() {
            println!("没有找到写下的 OctoLoop 命令");
        }
        for item in &report.items {
            let word = |key: &str| item[key].as_str().unwrap_or_default();
            println!("{:<5} {}  {}", word("status"), word("kind"), word("where"));
            println!("      {}", word("detail"));
            if let Some(fix) = item["fix"].as_str() {
                println!("      修复:{fix}");
            }
        }
        println!(
            "共 {} 项:ok {ok},warn {warn},fail {failed}",
            report.items.len()
        );
    }
    i32::from(failed > 0)
}
