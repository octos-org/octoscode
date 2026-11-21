spec: task
name: "OctoLoop 事件日志:卡的写入命令与 Markdown 只读视图"
id: task-olp-card-writes
status: implemented
tags: [olp, store, jsonl, card]
estimate: 2d
---

## Intent

事件日志(task-olp-event-log)有了读取和导入,还没有写入的命令。本任务让一张卡的一生——立卡、
内环回执、外环采认/打回/搁置/改判作废——都成为日志里的事件,并把日志输出成 Markdown 供人和
agent 阅读。Markdown 黑板上这些动作靠手工或脚本追加定式行;日志不手工编辑,写入一律走命令。

## Decisions

- 命令(`src/olp/card.rs`,都带 `--log <日志目录>`):
  - `card open --title T [--unit U] [--impl A] [--review A,B] [--key K] [--by 署名] [--json]`,
    正文从 stdin;卡号 = 日志里最大的卡号 + 1,在锁内分配;stdout 最后一行是卡号。
  - `card ack <N> done|wontdo|blocked <说明…> [--by 署名]`——署名为 `内环(<署名>)`,默认 `内环(本机)`。
  - `card accept|return <N> <说明…>`、`card void <N> [说明…]`、
    `card snooze <N> --until YYYY-MM-DD <等什么…>`——署名为 `外环(<署名>)`,默认 `外环(hub)`。
- 规则在锁内对着当时的日志判断,不合规则当场拒绝(退出码 1,不写入):没有这张卡;卡已被
  改判作废。带 `--key` 的写入同一个键只写一次;`card open` 同键而单元或内容不同是错误。
- 外环的写入(open、accept、return、void、snooze)先查主审权租约(退出码 6);内环回执不查。
- 退出码:0 写入(或同键已写过);1 被规则拒绝;2 用法错误;4 日志读写失败;6 租约无效。
- 本任务的 `card open` 只立卡:不查单元健康与占用、不投递、不唤醒——那是派卡(`card dispatch`)
  迁移时的事。谁可以代表哪台单元写(单元身份由 hub 的 ssh 强制命令决定)在远程写入迁移时
  接上;现在 `--by` 是调用方给的署名。
- `board render <日志目录>`(`src/olp/render.rs`):把日志输出成 Markdown 到 stdout,只读视图。
  卡与署名行按事件顺序出现,像一块只追加过的黑板;导入的卡原样输出其正文,正文里已有的行不
  重复输出。**不变量**:收件箱读这份 Markdown 得出的结论与读日志相同(测试守住)。
- 导入的事件多记两项:`data.text`(来源行原文)、派卡事件的 `data.span`(正文行数),供渲染判断
  一行是否已在某张卡的正文里。

## Boundaries

### Allowed Changes
- src/olp/card.rs、src/olp/render.rs、src/olp/import.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_card.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-card-writes.spec.md

### Forbidden
- 不改任何 `scripts/olp-*`;现有命令仍读写 Markdown 黑板。
- 不引入数据库依赖。

## Out of Scope

- 派卡的检查、投递与唤醒;单元经 ssh 的远程写入;推送登记;进度节点;改派。
- 项目与单元的身份、隔离(`--log` 现在由调用方给出)。

## Completion Criteria

Scenario: 一张卡从立卡到采认,每一步都是一条事件,收件箱跟着变(critical)
  Tags: critical
  Test: a_card_moves_through_its_life_as_events
  Given 一个空的日志目录
  When 立两张卡,第一张经回执、打回、再回执、采认,第二张搁置后改判作废
  Then 收件箱在每一步给出对应的分区(进行中、待复验、返工、已采认、搁置、到期回收件箱、已作废);日志里是八条按序的事件,带署名、单元、复验方与原样的正文

Scenario: 不合规则的写入当场拒绝,日志不变
  Test: writes_that_break_a_rule_are_refused_and_nothing_is_written
  Given 只有一张已作废的卡
  When 对不存在的卡或已作废的卡回执、采认;以及各种用法错误
  Then 规则拒绝退出码 1 并说明原因,用法错误退出码 2,日志的事件数不变

Scenario: 同一个键只立一张卡;同键不同内容是错误
  Test: the_same_key_opens_one_card
  Given 用键立过一张卡
  When 以同键同内容再立,再以同键不同单元立;带键的采认执行两次
  Then 第一种返回原卡号且不新增事件;第二种退出码 1;采认只记录一次

Scenario: 多个进程同时立卡,卡号不重不漏(critical)
  Tags: critical
  Test: concurrent_processes_get_distinct_consecutive_numbers
  Given 一个空的日志目录
  When 12 个进程同时立卡
  Then 卡号恰为 1 到 12,事件序号连续

Scenario: 外环会话已结束时,外环的写入被拒(退出码 6);内环回执不受影响
  Test: outer_writes_need_a_live_lease
  Given 环境里有一份租约,而该项目的主审权锁空着
  When 立卡、采认、作废,以及内环回执
  Then 前三者退出码 6 且不写入;回执成功

Scenario: 渲染出的 Markdown 被收件箱读成与日志相同的结论(critical)
  Tags: critical
  Test: the_rendered_board_reads_the_same_as_the_log
  Given 一份有三张卡的日志,其中一张的正文里有围栏包着的标题、ACK 与采认示例
  When 渲染成 Markdown,再分别对日志和这份 Markdown 执行收件箱
  Then 两边的报告相同;视图标明只读,含各卡的 unit/impl/review 行与署名行
