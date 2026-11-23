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
//!
//! Scripts get `OCTOSCODE_BIN` (this binary), so that what they write into
//! hooks, git settings and keepalive jobs calls `<octoscode> olp …` rather
//! than a script inside one version's directory; `olp-bundle/current` links
//! to the directory of the binary that ran last (the skill reads it).

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
        "scripts/olp-cli-lib.py",
        include_bytes!("../../scripts/olp-cli-lib.py"),
    ),
    (
        "scripts/olp-dispatch.py",
        include_bytes!("../../scripts/olp-dispatch.py"),
    ),
    (
        "scripts/olp-doctor.py",
        include_bytes!("../../scripts/olp-doctor.py"),
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
        "scripts/olp-outer.py",
        include_bytes!("../../scripts/olp-outer.py"),
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
        "docs/OCTOLOOP_FEATURES.md",
        include_bytes!("../../docs/OCTOLOOP_FEATURES.md"),
    ),
    (
        "docs/OCTOLOOP_GUIDE.md",
        include_bytes!("../../docs/OCTOLOOP_GUIDE.md"),
    ),
    ("docs/OLP_CLI.md", include_bytes!("../../docs/OLP_CLI.md")),
    (
        "docs/OLP_CLUSTER.md",
        include_bytes!("../../docs/OLP_CLUSTER.md"),
    ),
    (
        "docs/OLP_INNER_AGENTS.md",
        include_bytes!("../../docs/OLP_INNER_AGENTS.md"),
    ),
    (
        "docs/OLP_KEEPALIVE.md",
        include_bytes!("../../docs/OLP_KEEPALIVE.md"),
    ),
    (
        "docs/OLP_LOCAL_GIT.md",
        include_bytes!("../../docs/OLP_LOCAL_GIT.md"),
    ),
    (
        "docs/OLP_LOCAL_PR.md",
        include_bytes!("../../docs/OLP_LOCAL_PR.md"),
    ),
    (
        "docs/OLP_OUTER_BOOT.md",
        include_bytes!("../../docs/OLP_OUTER_BOOT.md"),
    ),
    (
        "docs/OLP_QUICKSTART.en.md",
        include_bytes!("../../docs/OLP_QUICKSTART.en.md"),
    ),
    (
        "docs/OLP_QUICKSTART.md",
        include_bytes!("../../docs/OLP_QUICKSTART.md"),
    ),
    (
        "docs/OLP_REMOTE_BOARD.md",
        include_bytes!("../../docs/OLP_REMOTE_BOARD.md"),
    ),
    (
        "docs/OLP_REVIEW_EVIDENCE.md",
        include_bytes!("../../docs/OLP_REVIEW_EVIDENCE.md"),
    ),
    (
        "docs/OUTER_LOOP_PROTOCOL.md",
        include_bytes!("../../docs/OUTER_LOOP_PROTOCOL.md"),
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
    /// A command written in Rust from the start (`src/olp/`).
    Rust(fn(&[String]) -> i32),
    /// A command moved to Rust (`src/olp/`), with the script it replaced.
    /// `OCTOSCODE_OLP_LEGACY=1` runs the script instead, for one release.
    Moved(fn(&[String]) -> i32, &'static str),
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
        "doctor",
        None,
        "olp-doctor.py",
        &[],
        "检查钩子、保活任务、ssh 强制命令等处写下的命令还能不能用:olp doctor [--repo R] [--work W] [--json]",
    ),
    Entry {
        group: "board",
        action: Some("append"),
        target: Target::Moved(crate::olp::append::board_append, "olp-board-append.sh"),
        help: "在锁内向黑板追加(分配 {N}、按键去重)",
    },
    Entry {
        group: "board",
        action: Some("inbox"),
        target: Target::Moved(crate::olp::inbox::run, "olp-inbox.py"),
        help: "收件箱:等谁决定的卡;--units 看单元占用",
    },
    Entry {
        group: "board",
        action: Some("import"),
        target: Target::Rust(crate::olp::import::run),
        help: "把 Markdown 黑板一次性导入成事件日志(JSON Lines):olp board import <板> --to <目录>",
    },
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
    Entry {
        group: "board",
        action: Some("summary"),
        target: Target::Moved(crate::olp::summary::run, "olp-summary.py"),
        help: "汇总进展,交给项目全景台",
    },
    Entry {
        group: "id",
        action: Some("alloc"),
        target: Target::Moved(crate::olp::append::id_alloc, "olp-id-alloc.sh"),
        help: "按自然键分配黑板以外的编号",
    },
    Entry {
        group: "card",
        action: Some("open"),
        target: Target::Rust(|argv| crate::olp::card::run("open", argv)),
        help: "事件日志:立一张卡,分配卡号(正文从 stdin):card open --log <目录> --title T [--unit U]",
    },
    Entry {
        group: "card",
        action: Some("ack"),
        target: Target::Rust(|argv| crate::olp::card::run("ack", argv)),
        help: "事件日志:内环回执 done / wontdo / blocked",
    },
    Entry {
        group: "card",
        action: Some("accept"),
        target: Target::Rust(|argv| crate::olp::card::run("accept", argv)),
        help: "事件日志:外环采认",
    },
    Entry {
        group: "card",
        action: Some("return"),
        target: Target::Rust(|argv| crate::olp::card::run("return", argv)),
        help: "事件日志:外环打回",
    },
    Entry {
        group: "card",
        action: Some("void"),
        target: Target::Rust(|argv| crate::olp::card::run("void", argv)),
        help: "事件日志:外环改判作废",
    },
    Entry {
        group: "card",
        action: Some("snooze"),
        target: Target::Rust(|argv| crate::olp::card::run("snooze", argv)),
        help: "事件日志:外环搁置到某一天",
    },
    Entry {
        group: "board",
        action: Some("stall"),
        target: Target::Rust(crate::olp::stall::run),
        help: "事件日志:集群是否整体静默(有卡未结、没有内环在工作、日志无新事件)",
    },
    Entry {
        group: "board",
        action: Some("render"),
        target: Target::Rust(crate::olp::render::run),
        help: "事件日志:输出成 Markdown 黑板(只读视图)",
    },
    Entry {
        group: "card",
        action: Some("dispatch"),
        // The event log is dispatched to in Rust; a Markdown board
        // (`--board`) still goes through the script that writes it.
        target: Target::Rust(|argv| {
            if argv.iter().any(|w| w == "--board") {
                run_script("olp-dispatch.py", &[], argv, None)
            } else {
                crate::olp::dispatch::run(argv)
            }
        }),
        help: "一步派卡(查健康、查复验方、查空闲、写入);--log 事件日志,--board Markdown 板",
    },
    Entry {
        group: "unit",
        action: None,
        target: Target::Moved(crate::olp::unit::run, "olp-unit.py"),
        help: "单元:register / set-state / status / check / where / receive / heartbeat",
    },
    Entry {
        group: "remote",
        action: Some("ack"),
        target: Target::Rust(|argv| {
            if crate::olp::client::log_mode() {
                crate::olp::client::ack(argv)
            } else {
                run_script("olp-board-remote.py", &["ack"], argv, None)
            }
        }),
        help: "单元回执 done / wontdo / blocked(hub 用事件日志时走新通道,否则写 Markdown 黑板)",
    },
    Entry {
        group: "remote",
        action: Some("mirror"),
        target: Target::Rust(|argv| {
            if crate::olp::client::log_mode() {
                crate::olp::client::mirror(argv)
            } else {
                run_script("olp-board-remote.py", &["mirror"], argv, None)
            }
        }),
        help: "把 hub 上本单元的卡同步成本机的只读 Markdown",
    },
    Entry {
        group: "remote",
        action: Some("progress"),
        target: Target::Rust(crate::olp::client::progress),
        help: "事件日志:单元上报进度节点 claimed / review <轮次> / rework <轮次>",
    },
    Entry {
        group: "remote",
        action: Some("inbox"),
        target: Target::Rust(crate::olp::client::inbox),
        help: "事件日志:派给本单元、还没领取的卡;领取用 remote progress <N> claimed",
    },
    Entry {
        group: "remote",
        action: Some("watch"),
        target: Target::Rust(crate::olp::client::watch),
        help: "事件日志:单元值守,有新派卡且 agent 空闲时往窗格发一句固定短句",
    },
    Entry {
        group: "remote",
        action: Some("serve-log"),
        target: Target::Rust(crate::olp::serve::run),
        help: "事件日志:ssh 强制命令,单元只能回执、报进度、读自己的卡",
    },
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
    Entry {
        group: "ask",
        action: None,
        target: Target::Moved(crate::olp::answer::run, "olp-answer.py"),
        help: "提问:list / show / answer(hub),serve(单元)",
    },
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
    script(
        "outer",
        None,
        "olp-outer.py",
        &[],
        "外环挂了自动拉起:supervise(检查一次,锁空着就在外环窗格里重新 hold)/ install(注册成常驻任务)",
    ),
    Entry {
        group: "duty",
        action: None,
        target: Target::Native("outer-duty", &[]),
        help: "主审权锁:hold / check / lease",
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

/// Where a command is meant to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunsOn {
    Hub,
    Unit,
    Any,
    /// Called by git or by an ssh forced command, never typed.
    Internal,
}

/// What an agent needs to know before it tries a command. `None` on
/// `writes` / `needs_lease` means "not recorded here": the answer depends
/// on a subcommand or has not been established yet, and the command's own
/// `--help` is the place to look. Never guess `false`.
struct Meta {
    runs_on: RunsOn,
    writes: Option<bool>,
    needs_lease: Option<bool>,
}

const fn meta_of(runs_on: RunsOn, writes: Option<bool>, needs_lease: Option<bool>) -> Meta {
    Meta {
        runs_on,
        writes,
        needs_lease,
    }
}

/// The catalog facts for one entry. A command missing here fails
/// `every_command_has_catalog_facts`.
fn meta(group: &str, action: Option<&str>) -> Option<Meta> {
    use RunsOn::*;
    let (yes, no) = (Some(true), Some(false));
    Some(match (group, action) {
        ("init", None) | ("skill", Some("install")) => meta_of(Any, yes, no),
        ("doctor", None) => meta_of(Any, no, no),
        ("board", Some("append" | "archive" | "import")) => meta_of(Hub, yes, yes),
        ("board", Some("inbox" | "watch" | "summary")) => meta_of(Hub, no, no),
        ("id", Some("alloc")) => meta_of(Hub, yes, no),
        ("card", Some("dispatch" | "open" | "accept" | "return" | "void" | "snooze")) => {
            meta_of(Hub, yes, yes)
        }
        ("card", Some("ack")) => meta_of(Any, yes, no),
        ("board", Some("render" | "stall")) => meta_of(Hub, no, no),
        ("unit", None) | ("keepalive", None) | ("ask", None) => meta_of(Any, None, None),
        ("remote", None) => meta_of(Unit, None, no),
        ("repo", Some("init")) => meta_of(Hub, yes, no),
        ("repo", Some("status")) => meta_of(Hub, no, no),
        ("repo", Some("promote")) => meta_of(Hub, yes, yes),
        ("repo", Some("connect-local" | "push")) => meta_of(Unit, yes, no),
        ("repo", Some("serve" | "hook" | "local-pack")) => meta_of(Internal, None, no),
        ("remote", Some("serve-log")) => meta_of(Internal, None, no),
        ("remote", Some("ack" | "progress" | "mirror")) => meta_of(Unit, yes, no),
        ("remote", Some("inbox" | "watch")) => meta_of(Unit, no, no),
        ("ask", Some("mcp")) | ("guard", Some("watch")) => meta_of(Unit, no, no),
        ("guard", Some("forward")) => meta_of(Unit, yes, no),
        ("outer", None) | ("duty", None) => meta_of(Hub, None, None),
        ("evo", Some(_)) | ("review", _) => meta_of(Hub, None, None),
        _ => return None,
    })
}

fn route(e: &Entry) -> String {
    match e.action {
        Some(a) => format!("octoscode olp {} {a}", e.group),
        None => format!("octoscode olp {}", e.group),
    }
}

/// One catalog record: what `olp commands --json` lists and what
/// `olp <命令> --help --json` prints for a single command.
fn describe(e: &Entry) -> serde_json::Value {
    let m = meta(e.group, e.action);
    let runs_on = match m.as_ref().map(|m| m.runs_on) {
        Some(RunsOn::Hub) => "hub",
        Some(RunsOn::Unit) => "unit",
        Some(RunsOn::Internal) => "internal",
        Some(RunsOn::Any) | None => "any",
    };
    let route = route(e);
    serde_json::json!({
        "route": route,
        "summary": e.help,
        // The next word is the tool's own subcommand; its --help lists them.
        "subcommands": e.action.is_none() && e.group != "init" && e.group != "doctor",
        "runs_on": runs_on,
        "writes": m.as_ref().and_then(|m| m.writes),
        "needs_lease": m.as_ref().and_then(|m| m.needs_lease),
        "implementation": match e.target {
            Target::Native(..) | Target::Moved(..) | Target::Rust(..) => "rust",
            Target::Script(..) | Target::Keepalive => "script",
        },
        "help": format!("{route} --help"),
    })
}

fn catalog() -> serde_json::Value {
    let aliases: Vec<serde_json::Value> = ALIASES
        .iter()
        .map(|(alias, g, a)| {
            serde_json::json!({
                "alias": format!("octoscode olp {alias}"),
                "route": format!("octoscode olp {g} {a}"),
            })
        })
        .collect();
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commands": COMMANDS.iter().map(describe).collect::<Vec<_>>(),
        "aliases": aliases,
    })
}

fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

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
    out.push_str("命令目录(给程序读):`octoscode olp commands --json`;单条命令:`octoscode olp <组> <动作> --help --json`。查帮助不执行命令。\n");
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
        point_current(&root, &tag);
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
        point_current(&root, &tag);
        Ok(dir)
    } else {
        Err(std::io::Error::other(format!("无法写入 {}", dir.display())))
    }
}

/// Keep `<root>/current` pointing at this binary's directory: a path that
/// stays valid across upgrades (`olp skill install` records it). Best effort.
fn point_current(root: &Path, tag: &str) {
    #[cfg(unix)]
    {
        let link = root.join("current");
        if std::fs::read_link(&link).is_ok_and(|target| target == Path::new(tag)) {
            return;
        }
        // A fresh link renamed over the old one: readers never see none.
        let staged = root.join(format!(".current-{}", std::process::id()));
        let _ = std::fs::remove_file(&staged);
        if std::os::unix::fs::symlink(tag, &staged).is_ok()
            && std::fs::rename(&staged, &link).is_err()
        {
            let _ = std::fs::remove_file(&staged);
        }
    }
    #[cfg(not(unix))]
    let _ = (root, tag);
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
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("OCTOSCODE_BIN", exe);
    }
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
    if words[0] == "commands" {
        // The catalog of this version's commands. Read-only.
        return match words.get(1).map(String::as_str) {
            None | Some("-h") | Some("--help") => {
                print!("{}", usage());
                0
            }
            Some("--json") if words.len() == 2 => {
                print_json(&catalog());
                0
            }
            Some(other) => {
                eprintln!(
                    "octoscode olp commands: 不认识的参数: {other}(用法:octoscode olp commands [--json])"
                );
                2
            }
        };
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
    // `olp <组> --help` where a script handles the rest of the group: say
    // the actions this binary has first — the script's help cannot know them.
    if words.len() == 2
        && matches!(action.as_deref(), Some("-h") | Some("--help"))
        && entry.is_some_and(|e| e.action.is_none())
        && in_group.iter().any(|e| e.action.is_some())
    {
        println!("octoscode olp {group} <动作>:");
        for e in in_group.iter().filter(|e| e.action.is_some()) {
            println!("  {:<10} {}", e.action.unwrap_or_default(), e.help);
        }
        println!("\n其余动作(下面是它们的帮助):\n");
    }
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
    // `--help --json`, in either order and with nothing else: describe the
    // command and stop. Help never runs the command; `--json` on its own
    // belongs to the command and still runs it.
    if rest.len() == 2
        && rest.iter().any(|w| w == "--help" || w == "-h")
        && rest.iter().any(|w| w == "--json")
    {
        print_json(&describe(entry));
        return 0;
    }
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
        Target::Rust(native) => native(&rest),
        Target::Moved(native, file) => {
            if std::env::var_os("OCTOSCODE_OLP_LEGACY").is_some_and(|v| !v.is_empty()) {
                return run_script(file, &[], &rest, None);
            }
            native(&rest)
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

    /// Retrospective #05: the skill read from the bundle (`olp skill install`
    /// records `olp-bundle/current`) pointed at docs that were not there.
    /// Every `docs/*.md` a bundled text names must be bundled too.
    #[test]
    fn every_doc_a_bundled_text_names_is_bundled() {
        let bundled: Vec<&str> = BUNDLE.iter().map(|(path, _)| *path).collect();
        let mut missing = Vec::new();
        for (path, bytes) in BUNDLE {
            if !(path.ends_with(".md")) {
                continue;
            }
            let text = String::from_utf8_lossy(bytes);
            let mut rest = text.as_ref();
            while let Some(at) = rest.find("docs/") {
                let tail = &rest[at..];
                let end = tail
                    .find(|c: char| !(c.is_ascii_alphanumeric() || "/_.-".contains(c)))
                    .unwrap_or(tail.len());
                let name = tail[..end].trim_end_matches('.');
                if name.ends_with(".md")
                    && !name[5..].contains('/')
                    && !bundled.contains(&name)
                    && !missing.contains(&(name.to_string(), *path))
                {
                    missing.push((name.to_string(), *path));
                }
                rest = &tail[end.max(5)..];
            }
        }
        assert!(
            missing.is_empty(),
            "named but not bundled (doc, named in): {missing:?}"
        );
    }

    #[test]
    fn every_command_names_a_bundled_script() {
        for e in COMMANDS {
            if let Target::Script(file, _) | Target::Moved(_, file) = e.target {
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

    #[test]
    fn every_command_has_catalog_facts() {
        for e in COMMANDS {
            let m = meta(e.group, e.action)
                .unwrap_or_else(|| panic!("{} {:?} has no catalog facts", e.group, e.action));
            // Commands git calls are the ones the listing hides.
            assert_eq!(
                e.help.starts_with("(由 git"),
                // ssh forced commands are internal too, but an operator
                // writes them into authorized_keys: those stay listed.
                m.runs_on == RunsOn::Internal && !e.action.is_some_and(|a| a.starts_with("serve")),
                "{} {:?}",
                e.group,
                e.action
            );
            assert!(
                COMMANDS.iter().all(|other| other.group != "commands"),
                "`commands` is the catalog, not a group"
            );
        }
    }
}
