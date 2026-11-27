spec: task
name: "OctoLoop 事件日志:JSON Lines 取代 Markdown 作为数据的存放处(第一步:日志、导入、收件箱读日志)"
id: task-olp-event-log
status: implemented
tags: [olp, store, jsonl]
estimate: 2d
---

## Intent

黑板是一个 Markdown 文件:每个工具每次从头重读,卡的状态靠"最后一行是什么"推断,正文与
机器读的行混在一起(卡片正文里的代码示例曾清空复验方名单,#757)。2026-11-21 operator 定稿:
数据的准确来源改为只追加的 JSON Lines 事件日志,Markdown 变成由它生成的阅读视图;数据库
(若以后加)只当可重建的查询索引,不参与写入、不参与任何决定。本任务是第一步。

## Decisions

- 日志(`src/olp/log.rs`):每个项目一个目录,`events-NNNNNN.jsonl` 分段,一行一条事件:
  `seq`(从 1 起、跨分段连续)、`ts`(UTC)、`kind`、`card`、`actor`、`unit`、`key`(幂等键)、
  `data`。卡的正文是 `data.body` 里的一个字符串,其中的任何内容都不会被当成机器读的行。
- 写入只有一条路:拿目录锁(`events.lock`)→ 读日志 → 判断 → 追加 → 落盘(`append_with`)。
  幂等键、只领取一次、分配卡号都在锁内对着当时的日志判断。等锁 15 秒超时报错。
- 末行没有换行符是没写完的写入:读时忽略,下次追加前截掉。完整但解析不了的行、序号不连续
  (含缺分段)是错误,绝不跳过。分段到 8 MiB 封存。
- 卡的状态只算一次:`src/olp/cards.rs` 把事件重放成与扫描 Markdown 相同的结构(行号换成
  `seq`),收件箱的分类规则(`src/olp/inbox.rs`)原样复用。事件种类:dispatched、assigned、ack、
  pushed、accepted、returned、voided、snoozed、archived(仅导入旧归档桩)、asked、answered、
  timed_out;不认识的种类忽略。
- `octoscode olp board import <板.md> --to <目录> [--dry-run]`(`src/olp/import.rs`):一次性导入。
  事件按来源行的顺序编号(`data.line` 记来源行),所以日志对每张卡的结论与原板相同。日志里
  已有事件时拒绝(退出码 1);黑板不被改动。板上没有对应卡的判词、重复的编号不导入,在
  stderr 逐条说明。已闭合的 `## Historical record` 区不导入。带主审权租约检查(退出码 6,
  `src/olp/duty.rs`,与脚本的 `duty_gate` 同一规则,直接调用 Rust 的租约校验)。
- `octoscode olp board inbox <路径>`:路径是目录时读事件日志,是文件时读 Markdown。两者的
  JSON 结构相同;日志的警告与提问的 `line` 用事件序号。日志损坏时退出码 1,不输出。
- 手工不编辑日志;人读的 Markdown 视图(`olp board render`)与写入命令(派卡、ACK、判词写
  日志)是后续任务。

## Boundaries

### Allowed Changes
- src/olp/log.rs、src/olp/cards.rs、src/olp/import.rs、src/olp/duty.rs、src/olp/inbox.rs、
  src/olp/board.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_event_log.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-event-log.spec.md

### Forbidden
- 不改任何 `scripts/olp-*`;Markdown 黑板仍是现有命令的准确来源。
- 不引入数据库依赖。

## Out of Scope

- 把派卡、ACK、判词、推送登记写进日志(后续任务)。
- `olp board render`、单元的增量同步、跨项目占用登记表。
- 状态快照与归档:另立任务 `specs/task-olp-log-compact.spec.md`(已实现)。压缩之后 `seq`
  仍只增不重排,但读到的事件之间有空洞(归档掉的事件);"跨分段连续"此后指在用的分段之内、
  以及归档加在用的完整历史。每条命令读的是"基线留下的 + 之后写入的",不再是全部历史。
  2026-11-27 在 Mac mini 上用正式构建实测(未压缩):20 万条事件(约 3600 张卡、38 MB、5 个
  分段)读一遍或追加一条都是 0.13 秒、占内存 224 MB;100 万条是 0.68 秒、1 GB,随事件数线性
  增长。锁内只读新增的行,写入前找末行的结尾只读分段尾部,所以持锁时间不随日志变长。

## Completion Criteria

Scenario: 导入后的事件日志与原黑板得出同样的收件箱(critical)
  Tags: critical
  Test: imported_log_classifies_every_card_as_the_board_did
  Given 一块含全部定式的黑板:单元与角色、裸 ACK 与署名 ACK、采认、打回、搁置、作废、推送、归档桩、围栏里的示例、提问与回答、已闭合的历史区
  When 导入成事件日志,再分别对黑板和日志执行收件箱(多种参数)
  Then 两边的分区、原因、计数、占用、待答提问逐项相同;板上没有对应卡的判词在导入时说明未导入

Scenario: 事件按来源行的顺序编号,卡的正文原样保存为一个字符串
  Test: imported_events_keep_line_order_and_card_text
  Given 同一块黑板
  When 导入
  Then 事件序号从 1 连续、来源行不降序;第一条是带单元、实现方、复验方的派卡事件;围栏里的示例留在正文里,不产生事件

Scenario: 导入只做一次;dry-run 不写;黑板不被改动
  Test: import_runs_once_and_never_touches_the_board
  Given 一块黑板和一个空目录
  When 先 dry-run,再导入,再导入一次
  Then dry-run 不创建任何文件;第二次导入退出码 1 且日志不变;黑板内容不变;缺参数或板不存在退出码 2

Scenario: 损坏的日志不被悄悄读成较短的日志
  Test: inbox_refuses_a_corrupt_log
  Given 一份末尾多了一行非 JSON 的日志
  When 对它执行收件箱
  Then 退出码 1,提示事件日志损坏,不输出报告

Scenario: 仓库里冻结的真实黑板导入后结论不变
  Test: the_frozen_campaign_board_survives_the_import
  Given docs/OUTER_LOOP_REVIEW.md(2026-08 战役的冻结快照)
  When 导入并对两边执行收件箱
  Then 报告相同

Scenario: 接着上次读到的地方读,不重读整个日志
  Test: a_read_goes_on_from_where_the_last_one_stopped
  Given 一个分段很小的日志,已读到第 3 条
  When 没有新事件时再读;追加 4 条(跨了分段)且末尾有一行没写完后再读;下一次写入之后再读
  Then 没有新事件时什么都不读;之后只读到新增的 4 条,没写完的那行不算;写入切掉残行后接着读到新的一条,结果始终与从头读一致

Scenario: 等锁期间别人写入的事件,拿到锁后的判定看得到(critical)
  Tags: critical
  Test: a_decision_sees_what_was_appended_while_it_waited_for_the_lock
  Given 锁被占着,一次追加已读完日志、在等锁
  When 持锁者写入一条事件后放开锁
  Then 等锁的那次追加在判定时看得到这条事件,它写入的序号接在其后

Scenario: 写入前找末行的结尾只读分段尾部
  Test: the_end_of_the_last_whole_line_is_found_from_the_end
  Given 空文件、没有换行的文件、以整行结尾的文件、带残行的文件、残行比一次读取还长的文件
  When 从文件末尾往前找最后一个换行
  Then 得到整行部分的字节数,与从头读的结果一致
