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
  - `--board` 是事件日志目录时,留言写成日志里的一条事件(`kind: note`,`actor: 外环监督`,
    `data.text` 是留言,不属于任何卡);`olp board render` 把它显示成 `> 外环监督: …`。
    来源:2026-11-26 在 hub 上实测,原先对日志目录调用 `olp board append`,留言写不进去。
  - herdr 自己重启时(2026-11-26,operator 定:恢复命令按我们的来):窗格里的进程全没了,herdr
    0.9.2 起会重新运行窗格登记过的"恢复命令"。重启成功后 supervise 等 herdr 认出窗格里的
    agent(`pane get`,最多 `--ready` 秒),再用
    `herdr pane report-agent-session <窗格> --source olp-outer --agent <认出的名字> --seq <秒> -- <命令>`
    把带 hold 的整条命令登记进去;只登记命令,不报状态(状态仍由 herdr 从屏幕读)。登记不上
    只提醒,不算失败。
  - 外环一律以 `env -u HERDR_ENV <octoscode 绝对路径> outer-duty hold … -- <外环命令>` 启动,登记
    的恢复命令也是这一条:去掉 `HERDR_ENV` 后 agent 自带的 herdr 集成不出声。实测(herdr 0.9.5,
    hagency 分支 8d6f3a11):Claude Code 的集成(v11)只登记会话 id,与我们的恢复命令并存时 herdr
    运行我们的;但连状态一起报的来源会"占住"窗格,此后我们的登记被拒(`resume_not_accepted`,
    手工模拟所见)。去掉变量对哪种集成都有效。窗格创建时的 `--env` 变量 herdr 不恢复。
  - 锁空着而外环窗格里有程序在跑(退出码 3):连续第二次看到时在黑板上留言一次"外环没有持锁";
    计数记在重启记录里,锁被占着或重启成功后清零。
  - `install`:经保活工具(macOS launchd、Linux systemd)注册每 `--interval` 秒运行一次的
    `<octoscode 绝对路径> olp outer supervise …`,herdr 路径记进 `OLP_HERDR`。
  - `--board` 是事件日志目录时,`install` 另注册 `<name>-watch`:每 `--interval` 秒运行
    `olp outer watch <日志> --label <label> --once`(见 `specs/task-olp-outer-watch.spec.md`)。
    Markdown 黑板不注册。
- 外环命令由使用者给,需能接上次会话(`claude --continue`、`codex resume --last --no-daemon`)。
- `docs/OLP_KEEPALIVE.md` 删去"不要把 hold 注册成常驻任务——自动重启等于自动抢锁",改为
  §4a;协议 R7 补一句锁空着时可由监督任务自动重新 hold。

## Boundaries

### Allowed Changes
- src/olp/outer.rs(2026-11-23:外环自动拉起与接班简报的 Rust 实现;本 spec 的场景原本就对 `octoscode olp outer …` 运行,行为、输出、退出码不变;接班简报的 `--board` 也可以是事件日志目录)
- src/olp/render.rs(2026-11-26:Markdown 视图显示 note 事件)
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

Scenario: 黑板是事件日志时,监督的留言写成日志里的一条事件
  Test: outer_supervise_writes_its_notes_into_an_event_log
  Given `--board` 指向一个事件日志目录
  When 运行 supervise,再看 `olp board render` 与 `olp outer resume`
  Then 日志里有一条 note 事件写着"外环已自动重启";Markdown 视图里有这一行;接班简报列出"监督任务的留言",检查单指向 `olp outer watch` 而不是 `olp board watch`

Scenario: 黑板是事件日志时,install 另注册外环值守
  Test: outer_install_registers_the_watcher_for_an_event_log
  Given `--board` 分别指向事件日志目录与 Markdown 黑板,`--label my-outer`
  When 各运行一次 install --no-load
  Then 前者写出两个任务,其中 outer-watch 调用 `olp outer watch <日志> --label my-outer --once` 并记下 herdr 的位置;后者只有一个任务

Scenario: 重启后把带 hold 的完整命令登记成 herdr 的恢复命令,不报状态
  Test: outer_supervise_registers_the_hold_command_as_herdrs_resume_command
  Given 一个假 herdr 与空着的锁
  When 运行 supervise
  Then herdr 收到一次 report-agent-session:来源 olp-outer,agent 是 herdr 认出的名字,不带 --state;命令以 `env -u HERDR_ENV <octoscode 绝对路径> outer-duty hold` 开头、以外环命令结尾;没有调用 report-agent

Scenario: 锁空着而窗格里一直有程序在跑时,第二次看到就留言一次
  Test: outer_supervise_notes_a_pane_that_stays_busy_without_the_lock
  Given 外环窗格里有程序在跑,锁空着
  When 连续运行 supervise 三次
  Then 三次都退出码 3、不执行命令;黑板上第一次没有留言,第二次起恰有一行"外环没有持锁"

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
