spec: task
name: "外环挂了自动拉起:锁空着时在它的 herdr 窗格里重新 hold"
id: task-olp-outer-autorestart
status: implemented
tags: [olp, outer, r7, keepalive]
estimate: 1d
---

## Intent

外环挂了以后,原来要人发现、登进 hub、敲一长串 hold 命令再走接班清单;半夜挂掉就停到天亮,
单元交了回执也没人采认。operator 决定(2026-11-15):指定的外环在锁空着时自动重启;锁被占着
时不动,接管活锁仍归 operator。锁同时是存活信号与防双外环的闸门,所以监督任务只看锁。

## Decisions

- 新命令组 `octoscode olp outer`(`scripts/olp-outer.py`):
  - `supervise`:检查一次。锁 HELD → 输出 `held`、退出码 0。锁 VACANT → 按 label(默认
    `olp-outer`)找 herdr 工作区,没有就新建;窗格前台除 shell 及其启动时的辅助程序外还有程序
    在跑 → 不重启、退出码 3;否则 `herdr pane run` 在窗格里执行
    `<octoscode 绝对路径> outer-duty hold … -- <外环命令>`,等锁 HELD(`--wait`,默认 20 秒)
    并保持 `--settle`(默认 2 秒)才算成功。锁读不出 → 退出码 1。
  - 防反复崩溃:重启记录存在 `~/.octos/outer/supervise/<项目哈希>.json`(`OLP_OUTER_STATE`
    可改)。`--window`(默认 1800 秒)内已重启 `--max-restarts`(默认 3)次 → 不再重启,黑板
    告警一次,退出码 4;`--reset` 清掉记录。
  - 给了 `--board`:每次自动重启记一行,暂停时告警一行;写板时去掉 `OLP_DUTY_*`(监督任务
    不是外环)。
  - `install`:经保活工具(macOS launchd、Linux systemd)注册每 `--interval` 秒运行一次的
    `<octoscode 绝对路径> olp outer supervise …`,herdr 路径记进 `OLP_HERDR`。
- 外环命令由使用者给,需能接上次会话(`claude --continue`、`codex resume --last --no-daemon`)。
- `docs/OLP_KEEPALIVE.md` 删去"不要把 hold 注册成常驻任务——自动重启等于自动抢锁",改为
  §4a;协议 R7 补一句锁空着时可由监督任务自动重新 hold。

## Boundaries

### Allowed Changes
- src/olp/outer.rs(2026-11-23:外环自动拉起与接班简报的 Rust 实现;本 spec 的场景原本就对 `octoscode olp outer …` 运行,行为、输出、退出码不变;接班简报的 `--board` 也可以是事件日志目录)
- scripts/olp-outer.py
- src/cmd/olp.rs
- tests/olp_outer.rs
- docs/OLP_KEEPALIVE.md、docs/OUTER_LOOP_PROTOCOL.md、docs/OLP_CLI.md
- skills/octoloop/AGENTS.md(文档地图与命令对照各一行)
- specs/task-olp-outer-autorestart.spec.md

### Forbidden
- 锁 HELD 时不做任何事;不接管活锁。
- 不改 `outer-duty` 本身与 BOOT §0–§6。

## Out of Scope

- 重启后的接班步骤(C)。
- herdr 服务本身的保活(已有 `olp keepalive install herdr`)。

## Completion Criteria

Scenario: 锁空着时在外环窗格里重新 hold,锁被占着时什么都不做(critical)
  Tags: critical
  Test: outer_supervise_restarts_a_vacant_outer_and_leaves_a_held_one_alone
  Given 一个假 herdr(`pane run` 真的执行命令)与空着的锁
  When 运行 supervise;再运行一次;杀掉外环后再运行一次
  Then 第一次新建工作区并在 w1:p1 重新 hold,锁 HELD,黑板记一行;第二次输出 held、不执行任何命令;第三次复用同一工作区再次 hold

Scenario: 外环一起来就挂时重启 N 次后暂停,黑板告警一次,--reset 后再试
  Test: outer_supervise_pauses_after_repeated_failures
  Given 外环命令立即退出,`--max-restarts 2`
  When 连续运行 supervise 四次,再带 `--reset` 运行一次
  Then 前两次退出码 1,后两次退出码 4 且不再执行命令,黑板上恰有一行暂停告警;`--reset` 后又执行一次

Scenario: 外环窗格里还有程序在跑而锁空着时不重启
  Test: outer_supervise_leaves_a_busy_pane_alone
  Given 已有 label 为 olp-outer 的工作区,窗格前台有 claude
  When 运行 supervise
  Then 退出码 3,stderr 提到 claude,没有执行任何命令

Scenario: install 注册的常驻任务调用 octoscode 的绝对路径并记下 herdr 的位置
  Test: outer_install_registers_supervise_as_a_keepalive_job
  Given 一个沙箱
  When 运行 `olp outer install … --no-load --dir <目录> -- claude --continue`
  Then 任务里有 octoscode 的绝对路径、supervise、署名、外环命令与 OLP_HERDR=<herdr 路径>
