spec: task
name: "OctoLoop 事件日志:外环值守(往上的收件箱 + 唤醒)"
id: task-olp-outer-watch
status: implemented
tags: [olp, store, jsonl, hub, wake]
estimate: 1d
---

## Intent

往下的方向已经是"收件箱 + 唤醒":卡写进日志就是送达,单元值守在 agent 空闲时发一句固定短句。
往上的方向还缺:单元的回执、推送、提问写进了日志,外环却要靠自己去看。本任务补上外环一侧的
值守,做法与单元值守对称;并把"整体静默"的提醒(`olp board stall`)接到同一条唤醒上。

"读到哪条由读的人自己记"借鉴自 Antigravity Swarm 的 `swarm_wait{since}`:游标是值守自己交出去
的最大事件序号,不是日志的头,所以外环忙的时候到的事件不会被跳过。

## Decisions

- 命令 `octoscode olp outer watch <日志目录> --pane <外环的窗格> [--once] [--interval 30]
  [--quiet 600] [--idle-wait 5] [--session <herdr 会话>]`,在 hub(外环所在机器)上跑。
- `--label <工作区标签>` 代替 `--pane`(二者给且只给一个):每轮按标签找外环工作区的第一个
  窗格,与 `olp outer supervise` 找外环窗格的办法相同;没有这个工作区时这一轮不提醒,游标不动。
  `olp outer install` 在 `--board` 是事件日志时用它把值守注册成常驻任务(`<name>-watch`,
  每 `--interval` 秒跑一次 `--once`)。来源:2026-11-26 实测,外环自己在后台启动的值守在外环
  死后还活着,接班的外环再启动一个就成了两个。
- 等外环处理的事件:`ack`、`pushed`、`asked`,以及节点为 `undelivered` 的 `progress`。派卡、
  领取(`claimed`)、复验轮次、外环自己的判词不算。
- 已经了结的不算:卡已在"已了结"分区的事件、已有回答或已超时的提问不计入。值守第一次在一份
  旧日志上跑时不会为历史发话(2026-11-26 实测:新注册的值守为 18 条早已了结的事件叫醒了外环,
  外环在收件箱里什么也找不到)。
- 每轮:读日志;序号大于游标的上述事件有几条;另问一次 `olp board stall --json --quiet <秒>`
  有没有新的静默提醒(它自己记着哪些已经报过)。两者有任何一项:
  先 `herdr agent wait <窗格> --until idle --until done --timeout <毫秒>`,空闲才
  `herdr agent prompt <窗格> "<短句>" --wait --until working --until blocked --timeout 10000`。
- 短句固定:`有等外环处理的新事件,请执行:octoscode olp inbox <日志目录>;整体静默时另见
  octoscode olp board stall <日志目录>`。不含卡号、标题、正文、回执内容。
- 发话被接住后,游标推进到日志的头并写入状态文件(`OLP_OUTER_WATCH_STATE`,默认
  `~/.octos/outer/watch/outer-<日志路径摘要>.json`)。外环在忙、或发话没被接住:游标不动,下一轮
  再试。
- 已知局限:静默提醒由 `olp board stall` 记账。外环在忙的那一轮它已把这次静默记为"报过",
  要等它自己的重报间隔(3 次运行后,最多 3 次)才会再出现。新事件不受影响。
- 退出码:0;2 用法(缺窗格、窗格标识不合法、日志目录不存在);4 日志读不了。

## Boundaries

### Allowed Changes
- src/olp/outerwatch.rs、src/olp/outer.rs、src/olp/mod.rs、src/olp/cards.rs(2026-11-26:已了结的卡)
- tests/olp_outer_watch.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-outer-watch.spec.md

### Forbidden
- 不读、不转述卡的内容;不替外环做任何判定。

## Out of Scope

- 把值守注册成常驻任务(`olp keepalive`)。
- Markdown 板上的外环唤醒:那条通道仍用 `olp board watch` 挂哨。
- 真实 herdr 与真实外环窗格上的验证:自动化测试用的是记录调用的假 herdr。

## Completion Criteria

Scenario: 等外环处理的新事件唤醒外环一次,短句不带内容(critical)
  Tags: critical
  Test: events_for_the_outer_loop_wake_its_pane_once
  Given 一张派出的卡,外环的 agent 空闲
  When 只有派卡时跑一轮;单元领取后跑一轮;单元 ACK(done) 后跑一轮;再跑一轮
  Then 派卡与领取都不唤醒;ACK 后发一句固定短句,指向 olp inbox,其中没有卡号、标题、正文、回执内容;同一条事件不再唤醒

Scenario: 按工作区标签找外环的窗格;没有这个工作区时不提醒,事件留到下一轮
  Test: the_watcher_finds_the_outer_pane_by_its_workspace_label
  Given 日志里有一条等外环处理的回执,herdr 里只有标签为 my-outer 的工作区
  When 先按不存在的标签跑一轮,再按 my-outer 跑一轮;另试同时给 --pane 与 --label、两个都不给
  Then 第一轮不发话;第二轮往该工作区的窗格发话;后两种退出码 2

Scenario: 已了结的卡、已有结局的提问不再叫醒外环
  Test: what_is_already_over_does_not_wake_the_outer_loop
  Given 一份日志:一张卡已回执并采认,一个提问已超时;值守从未跑过
  When 跑一轮;再记一个新提问后跑一轮
  Then 第一轮不发话;第二轮为这一个新提问发话

Scenario: 外环在忙时不打扰,它空闲后补上,忙的时候到的事件不会被跳过(critical)
  Tags: critical
  Test: a_busy_outer_loop_is_told_when_it_is_idle_again
  Given 两张派出的卡,外环的 agent 在忙
  When 单元报告一张送达失败后跑一轮;单元又对另一张回执 blocked;外环空闲后跑一轮,再跑一轮
  Then 忙时不发话并说明下一轮再试;空闲后一句话交出两条事件;之后不再发

Scenario: 整体静默也唤醒外环,同一段沉默不反复唤醒
  Test: a_stall_wakes_the_outer_loop_and_is_not_repeated
  Given 一张没人领的卡,没有内环在工作
  When 静默时间未到时跑一轮;以 --quiet 0 跑三轮;再用缺窗格、缺日志、不存在的日志、不合法的窗格调用
  Then 未到时间不唤醒;到了发一句指向 olp board stall 的短句;同一段沉默的后两轮不再发;参数不对退出码 2
