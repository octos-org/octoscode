//! `octoscode olp init [目录]` and `octoscode olp skill install` — lay the
//! two-loop scaffold in a project, and install the `/octoloop` entry.
//!
//! `init` is idempotent: what exists is left alone, nothing is overwritten.
//! It checks dependencies and says how to install what is missing; writes
//! `.octos/loop.md` and the blackboard `.octos/OUTER_LOOP_REVIEW.md`; adds
//! the working files to `.gitignore`; lays the self-contained onboarding
//! card as `AGENTS.md`; installs the single-file helpers under
//! `~/.octos/outer/`. It writes no API key and grants no sandbox bypass.
//! Exit codes: 0; 2 a dependency is missing.
//!
//! `skill install` records where the protocol text lives (one line in
//! `~/.octos/outer/octoscode-home`) and installs a thin card that holds no
//! path — only how to find that place and read the text.
//! Exit codes: 0; 2 usage; 3 the target exists and is not ours.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const CARD_EN: &str = include_str!("../../docs/OCTOLOOP_AGENTS.md");
const CARD_ZH: &str = include_str!("../../docs/OCTOLOOP_AGENTS.zh-CN.md");
const SENTINEL: &str = include_str!("../../scripts/olp-watch-board.sh");
const APPENDER: &str = include_str!("../../scripts/olp-board-append.sh");
const SKILL_BODY: &str = include_str!("../../skills/octoloop/AGENTS.md");

const LOOP_MD: &str = "# 维护循环(内环 master 每轮唤醒执行)

每次维护唤醒依次执行,全部完成才结束本轮:

1. 读 `.octos/OUTER_LOOP_REVIEW.md`(外环审查黑板)。
2. 若存在**未 ACK 的条目**(无 `ACK(` 定式行):取编号最小的一条,按其内容
   执行到完成(代码改动跑全量测试 + fmt + clippy 后原子 commit,只 add
   自己改的文件),然后在该条目下补一行 v1 定式 ACK:
   `ACK(done|wontdo|blocked): <说明>`。
3. 无未 ACK 条目:检查在途 goal 与测试基线,如实记录状态后结束本轮。

纪律:内环只 commit、不 push(推送权在外环,独立复验后代推);
黑板只追加、不改写既有行。
";

const BOARD_MD: &str = "# 外环审查通道(Outer-Loop Review)

> 外环审查员(强模型 agent)与内环(octos master 及其 peers)的持久黑板。
> **Master:每轮任务开始前读本文件;执行完每条意见后,在对应条目下追加
> v1 定式 ACK 行:`ACK(done|wontdo|blocked): <说明>`**——done 附
> commit/测试证据,wontdo 附理由(外环只能接受或升级 operator,不得重复
> 打回),blocked 附阻塞原因。
> 外环只追加带日期的条目,不删除历史;多外环时批注必须署名(如
> `外环(claude)` / `外环(codex)`),分歧升级 operator 裁决。

---

### 1. 黑板启用(由 olp-init.sh 生成)

本条无需执行,ACK 后即完成首次读写闭环验证。

ACK:
";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn ok(text: &str) {
    println!("  [ok] {text}");
}

fn git(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Add `line` to `.gitignore` unless git already ignores `path`.
fn ignore(path: &str, lead: &str, added: &str, already: &str) {
    if git(&["check-ignore", "-q", path]) {
        ok(already);
        return;
    }
    let existing = std::fs::read(".gitignore").unwrap_or_default();
    let mut text = String::new();
    // A last line without a newline would glue onto what is appended.
    if !lead.is_empty() {
        text.push_str(lead);
    } else if !existing.is_empty() && existing.last() != Some(&b'\n') {
        text.push('\n');
    }
    text.push_str(&format!("{path}\n"));
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(".gitignore")
        .and_then(|mut f| f.write_all(text.as_bytes()));
    match written {
        Ok(()) => ok(added),
        Err(e) => println!("  [--] 写不了 .gitignore: {e}"),
    }
}

/// Install a single-file helper under `~/.octos/outer/` unless it is there.
fn helper(name: &str, body: &str) {
    let dir = home().join(".octos/outer");
    let path = dir.join(name);
    if path.is_file() {
        ok(&format!(
            "{name} 已存在,跳过(如需更新:删掉 {} 后重跑 octoscode olp init)",
            path.display()
        ));
        return;
    }
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, body))
        .and_then(|()| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
            Ok(())
        });
    match written {
        Ok(()) => ok(&format!("已安装 {name} → {}/", dir.display())),
        Err(e) => println!("  [--] 没装上 {name}: {e}"),
    }
}

/// `argv` is what follows `olp init`: an optional project directory.
pub fn init(argv: &[String]) -> i32 {
    let dir = match argv {
        [] => None,
        [flag] if flag == "-h" || flag == "--help" => {
            println!(
                "用法: octoscode olp init [目录]\n\n在项目目录铺设 OctoLoop 双环脚手架:依赖体检、.octos/loop.md、黑板、.gitignore、AGENTS.md 上岗卡、\n~/.octos/outer/ 下的单文件助手。幂等,不覆盖已有内容;不写 API key,不给免沙箱权限。\nOLP_INIT_LANG=zh 取中文上岗卡。退出码:0 成功;2 有缺失的依赖"
            );
            return 0;
        }
        [dir] if !dir.starts_with('-') => Some(dir),
        _ => {
            eprintln!("用法: octoscode olp init [目录]");
            return 2;
        }
    };
    if let Some(dir) = dir {
        if !Path::new(dir).is_dir() {
            eprintln!("octoscode olp init: 目录不存在: {dir}");
            return 2;
        }
        if let Err(e) = std::env::set_current_dir(dir) {
            eprintln!("octoscode olp init: 进不了 {dir}: {e}");
            return 2;
        }
    }
    let mut missing = false;
    println!("== OLP init: 依赖体检 ==");
    if on_path("git").is_some() {
        ok("git");
    } else {
        println!("  [!!] git 未安装——请先安装 git");
        missing = true;
    }
    match on_path("octoscode") {
        Some(path) => ok(&format!("octoscode ({})", path.display())),
        None => {
            println!(
                "  [!!] octoscode 未安装:npm install -g @octos-org/octoscode(或 brew / shell installer,见 README)"
            );
            missing = true;
        }
    }
    if on_path("octos").is_some() || home().join(".octos/bin/octos").is_file() {
        ok("octos server(已装或已自动拉起过)");
    } else {
        println!(
            "  [--] octos server 未见——首次运行 octoscode 会自动下载到 ~/.octos/bin(需网络);离线环境请手装:npm i -g @octos-org/octos"
        );
    }
    let has_key = ["MOONSHOT_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    if has_key {
        ok("检测到模型 API key 环境变量");
    } else {
        println!("  [--] 未检测到常见 API key 环境变量——onboarding 向导里粘贴亦可");
    }

    println!("\n== 铺设 .octos/ 脚手架 ==");
    if let Err(e) = std::fs::create_dir_all(".octos") {
        eprintln!("octoscode olp init: 建不了 .octos: {e}");
        return 2;
    }
    for (path, body) in [
        (".octos/loop.md", LOOP_MD),
        (".octos/OUTER_LOOP_REVIEW.md", BOARD_MD),
    ] {
        if Path::new(path).is_file() {
            ok(&format!("{path} 已存在,跳过"));
        } else if std::fs::write(path, body).is_ok() {
            ok(&format!("生成 {path}"));
        }
    }
    if git(&["rev-parse", "--is-inside-work-tree"]) {
        // The working files are the same on every branch: not in the tree.
        ignore(
            ".octos/OUTER_LOOP_REVIEW.md",
            "\n# OLP 黑板:分支无关的工作文件,不入库(防跨分支裂脑)\n",
            "黑板已加入 .gitignore",
            "黑板已在 .gitignore 中",
        );
        ignore(
            ".octos/EVOLUTION.md",
            "",
            "进化黑板已加入 .gitignore",
            "进化黑板已被忽略",
        );
        ignore(
            ".octos/UNIT_BOARD.md",
            "",
            "单元复验板已加入 .gitignore",
            "单元复验板已被忽略",
        );
    }

    println!("\n== agent 上岗卡(AGENTS.md) ==");
    let chinese = matches!(
        std::env::var("OLP_INIT_LANG").as_deref(),
        Ok("zh" | "zh-CN" | "zh_CN")
    );
    let (name, card) = if chinese {
        ("OCTOLOOP_AGENTS.zh-CN.md", CARD_ZH)
    } else {
        ("OCTOLOOP_AGENTS.md", CARD_EN)
    };
    if Path::new("AGENTS.md").is_file() {
        ok(&format!(
            "AGENTS.md 已存在,跳过(如需上岗卡另存: curl -fsSL https://raw.githubusercontent.com/octos-org/octoscode/main/docs/{name} -o OCTOLOOP_AGENTS.md)"
        ));
    } else if std::fs::write("AGENTS.md", card).is_ok() {
        ok(&format!("生成 AGENTS.md({name})"));
    } else {
        println!("  [--] AGENTS.md 未铺设(写不了文件)");
    }

    println!("\n== 外环侦听哨与写板助手(~/.octos/outer/) ==");
    helper("watch-board.sh", SENTINEL);
    helper("board-append.sh", APPENDER);

    println!("\n== 下一步(按序) ==");
    println!("  1. 启动内环(标准命令;--solo 是单人盒子安全门):");
    println!("       octoscode --stdio-command 'octos serve --stdio --solo'");
    println!("     需要跑构建/工具链时,操作者显式追加 --danger-full-access");
    println!("     (权限档 1-4 是 bwrap 沙箱,~/.cargo 不可见——见 QUICKSTART 0b 节)。");
    println!("  2. 首次进入 TUI 完成 onboarding(选 provider、贴 key)。");
    println!("  3. 外环接入:把本项目的 AGENTS.md 交给外环 agent(自包含上岗卡:");
    println!("     身份选择、ACK 定式、派单/唤醒/观测/复验、红线全在卡内)。");
    if missing {
        println!("\n  ⚠ 存在缺失依赖(上方 [!!] 行),先补齐再启动。");
        return 2;
    }
    0
}

const MARKER: &str =
    "<!-- generated by scripts/olp-install-skill.sh; do not edit, re-run the script -->";

/// The thin card: no fixed path, only how to find the text at run time.
fn thin_card() -> Result<String, String> {
    let description = SKILL_BODY
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or("规程正文缺少 description 行")?;
    Ok(format!(
        r#"---
name: octoloop
description: {description}。任意项目可用。
---
{MARKER}

# /octoloop(全局入口)

本卡是薄封装,不含规程正文,也不含固定路径。正文对所有 agent 通用,始终以
octoscode 仓库里的最新版为准。

## 第一步:找到 octoscode 仓库(OCTOSCODE_HOME)

按顺序取第一个**含 `skills/octoloop/AGENTS.md`** 的目录:

1. 环境变量 `OCTOSCODE_HOME`;
2. 文件 `~/.octos/outer/octoscode-home` 的内容(安装时记录的仓库位置);
3. 当前项目的根目录(当前项目就是 octoscode 时)。

一条命令完成解析(输出仓库路径;找不到时无输出、退出码非 0):

```bash
for h in "${{OCTOSCODE_HOME:-}}" "$(cat ~/.octos/outer/octoscode-home 2>/dev/null)" "$(git rev-parse --show-toplevel 2>/dev/null)"; do [ -n "$h" ] && [ -f "$h/skills/octoloop/AGENTS.md" ] && {{ echo "$h"; break; }}; done | grep .
```

三处都找不到:**停下告诉用户**,不要猜路径、不要凭记忆操作。让用户克隆
octoscode 仓库后在其中执行 `scripts/olp-install-skill.sh`,或设置
`OCTOSCODE_HOME`。

## 第二步:加载规程正文

完整读取 `$OCTOSCODE_HOME/skills/octoloop/AGENTS.md`,按其中规程执行。

## 路径解析规则(贯穿全程)

| 正文写法 | 解析到 |
|---|---|
| `docs/…`、`knowledge/…` 等规程文档 | `$OCTOSCODE_HOME/…` |
| `scripts/olp-*` | `$OCTOSCODE_HOME/scripts/…`(脚本自带定位,可从任意目录调用) |
| `.octos/…`(黑板、loop.md、信箱) | **当前项目**根目录 |
| 脚本参数 `<repo>`、`<板>` | **当前项目**(用绝对路径) |

当前项目就是 octoscode 本身时,一切相对路径直接按当前目录解析。

## 保持最新

正文随 `git -C "$OCTOSCODE_HOME" pull` 更新,本卡不用重装。执行 init 或
outer 前,若 `git -C "$OCTOSCODE_HOME" status -sb` 显示落后于 origin,先问
用户是否拉取。仓库搬家后,在新位置重跑 `scripts/olp-install-skill.sh`。
"#
    ))
}

/// `argv` is what follows `olp skill install`; `bundle` is this binary's
/// tool directory, which holds the protocol text.
pub fn skill_install(argv: &[String], bundle: &Path) -> i32 {
    let prog = "olp-install-skill";
    let (mut target, mut print, mut home_only, mut force) = (None, false, false, false);
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--to" => match words.next() {
                Some(path) => target = Some(PathBuf::from(path)),
                None => {
                    eprintln!("{prog}: --to 需要一个文件路径");
                    return 2;
                }
            },
            "--print" => print = true,
            "--home-only" => home_only = true,
            "--force" => force = true,
            "-h" | "--help" => {
                println!(
                    "用法: octoscode olp skill install [--to <文件> | --print | --home-only] [--force]\n\n记下规程正文的位置(~/.octos/outer/octoscode-home),并装一张不含固定路径的薄卡\n(默认 ~/.claude/skills/octoloop/SKILL.md)。\n退出码:0 成功;2 用法错误;3 目标已存在且不是本工具生成的(未加 --force)"
                );
                return 0;
            }
            other => {
                eprintln!("{prog}: 未知参数: {other}");
                return 2;
            }
        }
    }
    let card = match thin_card() {
        Ok(card) => card,
        Err(why) => {
            eprintln!("{prog}: {why}");
            return 2;
        }
    };
    if print {
        print!("{card}");
        return 0;
    }
    // The one place a path is kept. The `current` link next to this
    // version's directory always points at the version last run.
    let current = bundle.parent().map(|root| root.join("current"));
    let recorded = match current {
        Some(link) if std::fs::canonicalize(&link).ok() == std::fs::canonicalize(bundle).ok() => {
            link
        }
        _ => bundle.to_path_buf(),
    };
    let home_file = home().join(".octos/outer/octoscode-home");
    let written = home_file
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| std::fs::write(&home_file, format!("{}\n", recorded.display())));
    if let Err(e) = written {
        eprintln!("{prog}: 写不了 {}: {e}", home_file.display());
        return 2;
    }
    println!(
        "recorded octoscode home: {}  ({})",
        recorded.display(),
        home_file.display()
    );
    if home_only {
        return 0;
    }
    let target = target.unwrap_or_else(|| home().join(".claude/skills/octoloop/SKILL.md"));
    if target.exists() && !std::fs::read_to_string(&target).is_ok_and(|text| text.contains(MARKER))
    {
        if !force {
            eprintln!("{prog}: {} 已存在,而且不是本脚本生成的。", target.display());
            eprintln!(
                "  看过它的内容后,加 --force 覆盖(原文件会备份为 {}.bak)。",
                target.display()
            );
            return 3;
        }
        let backup = PathBuf::from(format!("{}.bak", target.display()));
        if let Err(e) = std::fs::copy(&target, &backup) {
            eprintln!("{prog}: 备份不了原文件: {e}");
            return 2;
        }
        println!("backed up the existing file to {}", backup.display());
    }
    let dir = target.parent().unwrap_or(Path::new("."));
    let installed = std::fs::create_dir_all(dir).and_then(|()| {
        let mut tmp = tempfile::Builder::new()
            .prefix(".olp-skill.")
            .tempfile_in(dir)?;
        tmp.write_all(card.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o644))?;
        }
        tmp.persist(&target).map_err(|e| e.error)?;
        Ok(())
    });
    match installed {
        Ok(()) => {
            println!("installed: {}", target.display());
            0
        }
        Err(e) => {
            eprintln!("{prog}: 写不了 {}: {e}", target.display());
            2
        }
    }
}
