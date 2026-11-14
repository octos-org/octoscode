//! `octoscode olp` — the one entry point for the OctoLoop tools
//! (docs/OLP_CLI.md, specs/task-olp-cli.spec.md).
//!
//! Phase one: every command runs the existing script, bundled into the
//! binary, with its arguments, output and exit code unchanged. The scripts
//! are written once per binary version to `~/.octos/outer/olp-bundle/<hash>/`
//! with the repository layout they expect (`scripts/`, `docs/`, `skills/`),
//! so siblings they load (the board-append helper, the hub guard, the
//! onboarding cards) are where they look for them. Later phases replace the
//! scripts with Rust one group at a time; the command surface stays.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// Everything the scripts read, at the path they read it from.
const BUNDLE: &[(&str, &[u8])] = &[
    (
        "scripts/olp-alert-forward.py",
        include_bytes!("../../scripts/olp-alert-forward.py"),
    ),
    (
        "scripts/olp-answer.py",
        include_bytes!("../../scripts/olp-answer.py"),
    ),
    (
        "scripts/olp-board-append.sh",
        include_bytes!("../../scripts/olp-board-append.sh"),
    ),
    (
        "scripts/olp-board-archive.py",
        include_bytes!("../../scripts/olp-board-archive.py"),
    ),
    (
        "scripts/olp-board-remote.py",
        include_bytes!("../../scripts/olp-board-remote.py"),
    ),
    (
        "scripts/olp-dispatch.py",
        include_bytes!("../../scripts/olp-dispatch.py"),
    ),
    (
        "scripts/olp-evo-harvest.sh",
        include_bytes!("../../scripts/olp-evo-harvest.sh"),
    ),
    (
        "scripts/olp-evo-index.sh",
        include_bytes!("../../scripts/olp-evo-index.sh"),
    ),
    (
        "scripts/olp-evo-lib.py",
        include_bytes!("../../scripts/olp-evo-lib.py"),
    ),
    (
        "scripts/olp-evo-metrics.sh",
        include_bytes!("../../scripts/olp-evo-metrics.sh"),
    ),
    (
        "scripts/olp-evo-retro.sh",
        include_bytes!("../../scripts/olp-evo-retro.sh"),
    ),
    (
        "scripts/olp-evo-spec-skeleton.sh",
        include_bytes!("../../scripts/olp-evo-spec-skeleton.sh"),
    ),
    (
        "scripts/olp-git-hub.py",
        include_bytes!("../../scripts/olp-git-hub.py"),
    ),
    (
        "scripts/olp-hub-guard.sh",
        include_bytes!("../../scripts/olp-hub-guard.sh"),
    ),
    (
        "scripts/olp-id-alloc.sh",
        include_bytes!("../../scripts/olp-id-alloc.sh"),
    ),
    (
        "scripts/olp-inbox.py",
        include_bytes!("../../scripts/olp-inbox.py"),
    ),
    (
        "scripts/olp-init.sh",
        include_bytes!("../../scripts/olp-init.sh"),
    ),
    (
        "scripts/olp-install-skill.sh",
        include_bytes!("../../scripts/olp-install-skill.sh"),
    ),
    (
        "scripts/olp-launchd.py",
        include_bytes!("../../scripts/olp-launchd.py"),
    ),
    (
        "scripts/olp-local-pr.py",
        include_bytes!("../../scripts/olp-local-pr.py"),
    ),
    (
        "scripts/olp-preflight.sh",
        include_bytes!("../../scripts/olp-preflight.sh"),
    ),
    (
        "scripts/olp-review-evidence-cargo.py",
        include_bytes!("../../scripts/olp-review-evidence-cargo.py"),
    ),
    (
        "scripts/olp-review-evidence.py",
        include_bytes!("../../scripts/olp-review-evidence.py"),
    ),
    (
        "scripts/olp-review-monitor.py",
        include_bytes!("../../scripts/olp-review-monitor.py"),
    ),
    (
        "scripts/olp-summary.py",
        include_bytes!("../../scripts/olp-summary.py"),
    ),
    (
        "scripts/olp-systemd.py",
        include_bytes!("../../scripts/olp-systemd.py"),
    ),
    (
        "scripts/olp-task-push.sh",
        include_bytes!("../../scripts/olp-task-push.sh"),
    ),
    (
        "scripts/olp-unit.py",
        include_bytes!("../../scripts/olp-unit.py"),
    ),
    (
        "scripts/olp-watch-board.sh",
        include_bytes!("../../scripts/olp-watch-board.sh"),
    ),
    (
        "docs/OCTOLOOP_AGENTS.md",
        include_bytes!("../../docs/OCTOLOOP_AGENTS.md"),
    ),
    (
        "docs/OCTOLOOP_AGENTS.zh-CN.md",
        include_bytes!("../../docs/OCTOLOOP_AGENTS.zh-CN.md"),
    ),
    (
        "skills/octoloop/AGENTS.md",
        include_bytes!("../../skills/octoloop/AGENTS.md"),
    ),
];

/// Scripts in `scripts/` that are deliberately not in the bundle: a Windows
/// build diagnostic unrelated to OctoLoop, and the archived MCP prototype
/// (the server is `octoscode olp ask mcp`). A test keeps this list honest.
pub const NOT_BUNDLED: &[&str] = &["olp-cross-check-diagnostic.sh", "olp-mcp-server.py"];

/// What a command runs.
enum Target {
    /// A bundled script; `fixed` arguments go before the caller's.
    Script(&'static str, &'static [&'static str]),
    /// The script for this platform's service manager.
    Keepalive,
    /// A command that is already Rust: re-enter the dispatcher.
    Native(&'static str, &'static [&'static str]),
}

struct Entry {
    group: &'static str,
    /// `None`: the next word is the script's own subcommand, passed through.
    action: Option<&'static str>,
    target: Target,
    help: &'static str,
}

const fn script(
    group: &'static str,
    action: Option<&'static str>,
    file: &'static str,
    fixed: &'static [&'static str],
    help: &'static str,
) -> Entry {
    Entry {
        group,
        action,
        target: Target::Script(file, fixed),
        help,
    }
}

/// The command surface (docs/OLP_CLI.md §2). Order is the listing order.
const COMMANDS: &[Entry] = &[
    script(
        "init",
        None,
        "olp-init.sh",
        &[],
        "铺 OctoLoop 脚手架:olp init [目录]",
    ),
    script(
        "skill",
        Some("install"),
        "olp-install-skill.sh",
        &[],
        "安装 /octoloop 全局入口",
    ),
    script(
        "board",
        Some("append"),
        "olp-board-append.sh",
        &[],
        "在锁内向黑板追加(分配 {N}、按键去重)",
    ),
    script(
        "board",
        Some("inbox"),
        "olp-inbox.py",
        &[],
        "收件箱:等谁决定的卡;--units 看单元占用",
    ),
    script(
        "board",
        Some("archive"),
        "olp-board-archive.py",
        &[],
        "归档已了结的卡,留编号桩",
    ),
    script(
        "board",
        Some("watch"),
        "olp-watch-board.sh",
        &[],
        "板面侦听哨",
    ),
    script(
        "board",
        Some("summary"),
        "olp-summary.py",
        &[],
        "汇总进展,交给项目全景台",
    ),
    script(
        "id",
        Some("alloc"),
        "olp-id-alloc.sh",
        &[],
        "按自然键分配黑板以外的编号",
    ),
    script(
        "card",
        Some("dispatch"),
        "olp-dispatch.py",
        &[],
        "一步派卡(查健康、查空闲、写板、唤醒)",
    ),
    script(
        "unit",
        None,
        "olp-unit.py",
        &[],
        "单元:register / set-state / status / check / where / receive / heartbeat",
    ),
    script(
        "remote",
        None,
        "olp-board-remote.py",
        &[],
        "单元写 hub 的黑板:append / ack / flush / queue / mirror / serve",
    ),
    script(
        "repo",
        Some("init"),
        "olp-git-hub.py",
        &["init"],
        "建或刷新 hub 的本地仓库与钩子",
    ),
    script(
        "repo",
        Some("serve"),
        "olp-git-hub.py",
        &["serve"],
        "ssh 强制命令:单元推拉 hub 仓库",
    ),
    script(
        "repo",
        Some("connect-local"),
        "olp-git-hub.py",
        &["connect-local"],
        "单机部署:把克隆的 hub 远程接到本机仓库",
    ),
    script(
        "repo",
        Some("push"),
        "olp-task-push.sh",
        &[],
        "复验通过后推送 task/N",
    ),
    script(
        "repo",
        Some("status"),
        "olp-local-pr.py",
        &["status"],
        "本地 PR 状态",
    ),
    script(
        "repo",
        Some("promote"),
        "olp-local-pr.py",
        &["promote"],
        "采认后推 GitHub",
    ),
    script(
        "repo",
        Some("hook"),
        "olp-git-hub.py",
        &["hook"],
        "(由 git 调用)",
    ),
    script(
        "repo",
        Some("local-pack"),
        "olp-git-hub.py",
        &["local-pack"],
        "(由 git 调用)",
    ),
    Entry {
        group: "ask",
        action: Some("mcp"),
        target: Target::Native("olp-mcp-serve", &[]),
        help: "单元上的提问 MCP 服务(ask_outer / report_blocked)",
    },
    script(
        "ask",
        None,
        "olp-answer.py",
        &[],
        "提问:list / show / answer(hub),serve(单元)",
    ),
    Entry {
        group: "guard",
        action: Some("watch"),
        target: Target::Native("olp-runtime-watch", &[]),
        help: "运行时守护(只观察)",
    },
    script(
        "guard",
        Some("forward"),
        "olp-alert-forward.py",
        &[],
        "把守护的告警作为负哨送到 hub",
    ),
    Entry {
        group: "keepalive",
        action: None,
        target: Target::Keepalive,
        help: "保活:doctor / heartbeat / install / uninstall / list / render(macOS launchd,Linux systemd)",
    },
    Entry {
        group: "duty",
        action: None,
        target: Target::Native("outer-duty", &[]),
        help: "主审权锁:hold / check",
    },
    script(
        "evo",
        Some("harvest"),
        "olp-evo-harvest.sh",
        &[],
        "采集摩擦信号成症状卡",
    ),
    script(
        "evo",
        Some("retro"),
        "olp-evo-retro.sh",
        &[],
        "出 retro 简报",
    ),
    script(
        "evo",
        Some("metrics"),
        "olp-evo-metrics.sh",
        &[],
        "窗口化诊断",
    ),
    script(
        "evo",
        Some("index"),
        "olp-evo-index.sh",
        &[],
        "生成 FLAW 索引",
    ),
    script(
        "evo",
        Some("skeleton"),
        "olp-evo-spec-skeleton.sh",
        &[],
        "从 FLAW 记录出契约骨架",
    ),
    script(
        "review",
        Some("cargo"),
        "olp-review-evidence-cargo.py",
        &[],
        "互审:cargo 测试执行适配",
    ),
    script(
        "review",
        Some("monitor"),
        "olp-review-monitor.py",
        &[],
        "互审:只读进度视图",
    ),
    script(
        "review",
        None,
        "olp-review-evidence.py",
        &[],
        "行为证据互审:init / freeze / challenge / cross / status / classify",
    ),
];

/// Top-level shortcuts for the three most used commands.
const ALIASES: &[(&str, &str, &str)] = &[
    ("dispatch", "card", "dispatch"),
    ("inbox", "board", "inbox"),
    ("summary", "board", "summary"),
];

fn usage() -> String {
    let mut out = String::from(
        "octoscode olp <组> <动作> [参数…] — OctoLoop 工具的统一入口(docs/OLP_CLI.md)\n\n",
    );
    for e in COMMANDS {
        let name = match e.action {
            Some(a) => format!("{} {}", e.group, a),
            None => format!("{} …", e.group),
        };
        if e.help.starts_with("(由 git") {
            continue;
        }
        out.push_str(&format!("  {name:<20} {}\n", e.help));
    }
    out.push_str("\n别名:");
    for (alias, g, a) in ALIASES {
        out.push_str(&format!(" {alias}={g} {a}"));
    }
    out.push_str("\n参数、输出、退出码与 scripts/olp-* 相同;`octoscode olp <组> <动作> --help` 看该工具自己的帮助。\n");
    out
}

/// Where the bundle for this binary lives, written on first use.
fn bundle_dir() -> std::io::Result<PathBuf> {
    let root = match std::env::var_os("OCTOSCODE_OLP_BUNDLE") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .ok_or_else(|| std::io::Error::other("找不到 HOME"))?;
            Path::new(&home)
                .join(".octos")
                .join("outer")
                .join("olp-bundle")
        }
    };
    let mut hasher = Sha256::new();
    for (name, bytes) in BUNDLE {
        hasher.update(name.as_bytes());
        hasher.update([0u8]);
        hasher.update(bytes);
    }
    let digest = hasher.finalize();
    let tag: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let dir = root.join(&tag);
    if dir.join(".complete").is_file() {
        return Ok(dir);
    }
    // Write to a private directory, then rename into place: two first runs
    // at the same time never see a half-written bundle.
    std::fs::create_dir_all(&root)?;
    let staging = tempfile::Builder::new()
        .prefix(&format!(".{tag}-"))
        .tempdir_in(&root)?;
    for (name, bytes) in BUNDLE {
        let path = staging.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        #[cfg(unix)]
        if name.starts_with("scripts/") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    std::fs::write(staging.path().join(".complete"), b"")?;
    let staged = staging.keep();
    if std::fs::rename(&staged, &dir).is_err() {
        // Another process got there first; its copy is identical.
        let _ = std::fs::remove_dir_all(&staged);
    }
    if dir.join(".complete").is_file() {
        Ok(dir)
    } else {
        Err(std::io::Error::other(format!("无法写入 {}", dir.display())))
    }
}

fn interpreter(file: &str) -> OsString {
    if file.ends_with(".py") {
        std::env::var_os("OCTOSCODE_PYTHON").unwrap_or_else(|| "python3".into())
    } else {
        std::env::var_os("OCTOSCODE_BASH").unwrap_or_else(|| "bash".into())
    }
}

fn run_script(file: &str, fixed: &[&str], args: &[String], cwd: Option<&Path>) -> i32 {
    let dir = match bundle_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("octoscode olp: 无法准备工具目录: {e}");
            return 1;
        }
    };
    let path = dir.join("scripts").join(file);
    let mut cmd = Command::new(interpreter(file));
    if file.ends_with(".py") {
        cmd.arg("-B");
    }
    cmd.arg(&path).args(fixed).args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Become the script: stdin, signals and the exit code pass through
        // untouched (ssh forced commands and git hooks rely on that).
        let err = cmd.exec();
        eprintln!("octoscode olp: 无法运行 {}: {err}", file);
        127
    }
    #[cfg(not(unix))]
    {
        match cmd.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(err) => {
                eprintln!("octoscode olp: 无法运行 {}: {err}", file);
                127
            }
        }
    }
}

/// `argv` is everything after `olp`. Returns the process exit code.
pub fn run(argv: &[String]) -> i32 {
    let mut words: Vec<String> = argv.to_vec();
    if words.is_empty() || matches!(words[0].as_str(), "-h" | "--help" | "help") {
        print!("{}", usage());
        return 0;
    }
    if let Some((_, g, a)) = ALIASES.iter().find(|(alias, _, _)| *alias == words[0]) {
        words.splice(0..1, [g.to_string(), a.to_string()]);
    }
    let group = words[0].clone();
    let in_group: Vec<&Entry> = COMMANDS.iter().filter(|e| e.group == group).collect();
    if in_group.is_empty() {
        eprintln!("octoscode olp: 没有这个组: {group}\n\n{}", usage());
        return 2;
    }
    let action = words.get(1).cloned();
    let entry = in_group
        .iter()
        .find(|e| e.action.is_some() && e.action == action.as_deref())
        .or_else(|| in_group.iter().find(|e| e.action.is_none()));
    let Some(entry) = entry else {
        let actions: Vec<&str> = in_group.iter().filter_map(|e| e.action).collect();
        match action.as_deref() {
            None | Some("-h") | Some("--help") => {
                println!("octoscode olp {group} <动作>:{}", actions.join(" / "));
                return if action.is_none() { 2 } else { 0 };
            }
            Some(other) => {
                eprintln!(
                    "octoscode olp {group}: 没有这个动作: {other}(可选:{})",
                    actions.join(" / ")
                );
                return 2;
            }
        }
    };
    // Arguments after the matched words.
    let rest: Vec<String> = if entry.action.is_some() {
        words[2..].to_vec()
    } else {
        words[1..].to_vec()
    };
    match &entry.target {
        Target::Script(file, fixed) => {
            if group == "init" {
                // `olp init [目录]`: the script works on its current directory.
                let (dir, rest) = match rest.split_first() {
                    Some((d, r)) if !d.starts_with('-') => (PathBuf::from(d), r.to_vec()),
                    _ => (PathBuf::from("."), rest.clone()),
                };
                if !dir.is_dir() {
                    eprintln!("octoscode olp init: 目录不存在: {}", dir.display());
                    return 2;
                }
                return run_script(file, fixed, &rest, Some(&dir));
            }
            run_script(file, fixed, &rest, None)
        }
        Target::Keepalive => {
            let file = if cfg!(target_os = "macos") {
                "olp-launchd.py"
            } else if cfg!(target_os = "linux") {
                "olp-systemd.py"
            } else {
                eprintln!("octoscode olp keepalive: 只支持 macOS(launchd)与 Linux(systemd)");
                return 2;
            };
            run_script(file, &[], &rest, None)
        }
        Target::Native(sub, fixed) => {
            let mut inner = vec!["octoscode".to_string(), sub.to_string()];
            inner.extend(fixed.iter().map(|s| s.to_string()));
            inner.extend(rest);
            match super::dispatch(inner) {
                Ok(Some(code)) => code,
                Ok(None) => 2,
                Err(e) => {
                    eprintln!("octoscode olp: {e}");
                    1
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_olp_script_is_bundled_or_deliberately_left_out() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if !name.starts_with("olp-") {
                continue;
            }
            let bundled = BUNDLE
                .iter()
                .any(|(path, _)| *path == format!("scripts/{name}"));
            assert!(
                bundled || NOT_BUNDLED.contains(&name.as_str()),
                "scripts/{name} is neither bundled nor listed in NOT_BUNDLED"
            );
        }
    }

    #[test]
    fn every_command_names_a_bundled_script() {
        for e in COMMANDS {
            if let Target::Script(file, _) = e.target {
                assert!(
                    BUNDLE
                        .iter()
                        .any(|(path, _)| *path == format!("scripts/{file}")),
                    "{} {:?} -> {file} is not bundled",
                    e.group,
                    e.action
                );
            }
        }
        for (_, g, a) in ALIASES {
            assert!(
                COMMANDS
                    .iter()
                    .any(|e| e.group == *g && e.action == Some(*a))
            );
        }
    }
}
