spec: task
name: "OctoLoop 统一命令入口 octoscode olp(第一步:打包现有脚本)"
id: task-olp-cli
status: implemented
tags: [olp, cli]
estimate: 1d
---

## Intent

OctoLoop 的工具散在 `scripts/olp-*` 里:用户与 agent 要先找到 octoscode 仓库才能用(单元的
项目里没有 `scripts/`,唤醒消息里只好写 `$(cat ~/.octos/outer/octoscode-home)/scripts/…`),
ssh 强制命令、git 钩子、保活任务都写死了脚本路径。目标是一个入口 `octoscode olp <组> <动作>`,
之后逐批把实现换成 Rust 而命令面不变。命令面见 `docs/OLP_CLI.md`(2026-11-14 operator 定稿);
参考了 #634。

## Decisions

- 按对象分组:init、skill、board、id、card、unit、remote、repo、ask、guard、keepalive、duty、
  evo、review;顶层别名 dispatch = card dispatch、inbox = board inbox、summary = board summary。
  `remote` 是单元写 hub 黑板的命令,不并进 `board`。`keepalive` 在 macOS 选 launchd、Linux 选
  systemd、其他平台退出 2。
- 第一步:每个命令运行打包进二进制的原脚本,参数原样传入,输出与退出码不变(Unix 上 exec
  替换进程,标准输入、信号原样传递)。`guard watch`、`ask mcp`、`duty` 已是 Rust,直接调用。
- 打包内容:全部 `scripts/olp-*`(除 Windows 构建诊断与已归档的 MCP 原型,列在
  `NOT_BUNDLED`)、两张上岗卡、skill 正文,按仓库布局解到
  `~/.octos/outer/olp-bundle/<内容哈希前 16 位>/`(`OCTOSCODE_OLP_BUNDLE` 可改根目录)。先写到
  同目录的临时目录再改名;同一版本复用,不同版本互不覆盖。#634 每次拷到临时目录且只拷部分
  脚本,丢了同目录依赖(hub 守卫、`../docs/` 上岗卡),本设计不再有这个问题。
- 解释器:`.py` 用 `python3 -B`(`OCTOSCODE_PYTHON`),`.sh` 用 `bash`(`OCTOSCODE_BASH`)。
- 列表与帮助:`octoscode olp` / `help` / `--help` 列出命令与别名;`octoscode olp <组>` 列出该组
  动作(退出码 2);未知组或动作退出码 2;`<组> <动作> --help` 由工具自己回答。
- 两个单元测试守住打包清单:`scripts/` 下每个 `olp-*` 要么打包、要么在 `NOT_BUNDLED`;每条
  命令指向的脚本都已打包。

## Boundaries

### Allowed Changes
- src/cmd/olp.rs、src/cmd/mod.rs、src/cli.rs(顶层帮助一行)
- tests/olp_cli.rs
- docs/OLP_CLI.md、docs/OLP_QUICKSTART.md、skills/octoloop/AGENTS.md(文档地图一行与"统一入口"一段)
- specs/task-olp-cli.spec.md

### Forbidden
- 不改任何 `scripts/olp-*` 的行为。
- 不改 ssh 强制命令、唤醒消息、保活任务里现有的写法(下一步再迁)。

## Out of Scope

- 把实现换成 Rust(之后按组逐批做)。
- Windows 上找 Git Bash(#634 有,本步不做)。

## Completion Criteria

Scenario: 列出命令面;未知的组与动作以退出码 2 拒绝
  Test: olp_lists_the_command_surface_and_rejects_unknown_words
  Given 一个空的沙箱
  When 不带参数、带 --help、带未知组、带已知组的未知动作、只带组名运行
  Then 列表含各组与别名、不含由 git 调用的内部动作;未知组与未知动作退出码 2 并给出可选项;只带组名时列出该组动作

Scenario: 打包的工具与仓库里的逐字节相同,工具目录按内容复用(critical)
  Tags: critical
  Test: olp_bundle_matches_the_repository_and_is_reused
  Given 一个空的打包根目录
  When 运行一条命令,再运行一次
  Then 只出现一个按哈希命名的目录,其中脚本、上岗卡、skill 正文与仓库逐字节相同;第二次运行复用同一目录、不重写文件

Scenario: 参数、输出、退出码原样透传,别名与完整写法等价(critical)
  Tags: critical
  Test: olp_passes_arguments_output_and_exit_codes_through
  Given 不在仓库里的工作目录
  When 用完整写法与别名触发工具自己的用法错误;用相对路径向黑板追加两张 {N} 卡;用完整写法与别名读收件箱 JSON
  Then 用法错误是工具自己的退出码 2 与提示;编号依次是 1、2;两种写法的输出完全相同

Scenario: 工具依赖的同目录文件都在(critical)
  Tags: critical
  Test: olp_tools_find_their_siblings_in_the_bundle
  Given 一个新的 git 项目,本机配置的 hub 主机名不是本机
  When olp init;olp evo harvest;olp skill install --home-only;以 hub = local 写一条 remote ack
  Then 上岗卡与仓库里的相同、两个助手已安装;采集被 hub 守卫以退出码 3 拒绝而不是找不到守卫;记录的 octoscode 位置里有 skill 正文;ACK 落到黑板(serve 找到了同目录的写板助手)

Scenario: Rust 实现的命令直接调用;保活按平台选工具
  Test: olp_native_commands_and_keepalive_pick_the_right_implementation
  Given 一个空的沙箱
  When guard watch --help;duty check;向 ask mcp 发 initialize;keepalive render 一个定时任务
  Then 分别得到运行时守护的帮助、VACANT、MCP 服务名;macOS 上是 plist,Linux 上是 systemd 定时器
