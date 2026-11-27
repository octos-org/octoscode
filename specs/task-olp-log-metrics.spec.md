spec: task
name: "OctoLoop 事件日志的交付指标:每张卡多久、时间花在哪一段、打回多少"
id: task-olp-log-metrics
status: implemented
tags: [olp, store, jsonl, metrics]
estimate: 1d
---

## Intent

集群跑起来之后要能回答三件事:交付快不快、可不可信、被什么打断。2026-11-28 operator 定:指标
全部从事件日志算,不另建采集;先做只用现有日志就能算的部分。这是自进化闭环的第一步——
先能看见,再谈按它改策略。

## Decisions

- `octoscode olp log metrics <日志目录> [--since 日期] [--until 日期] [--by unit|impl|week]
  [--cards] [--json]`(`src/olp/metrics.rs`)。读完整历史(`Log::history`:归档 + 在用),
  所以压缩不改变任何数;不写任何东西,不需要主审权租约。
- 先按卡出一行(`rows`),汇总的每个数都从这些行算(`summarize`),`--cards` 列出明细,
  怪数可以查到是哪张卡。
- 一张卡的时间按它的事件切段(秒):
  - `to_claim`:派出 → 单元的 `claimed`;
  - `work`:领取(从未领取时为派出)与每次打回 → 下一条回执,多轮相加;
  - `outer_wait`:回执 → 外环的采认 / 打回 / 作废,多轮相加;
  - `total`:派出 → 采认,只有采认的卡有。
- `first_pass`:采认且从未打回。`reversed`:采认之后又被打回或作废。另数:打回次数、复验
  最大轮次、阻塞回执、守护告警、搁置、送达失败;单元提问(问了几个、答了几个、超时几个)。
- 时长只给中位数与 90 分位(最近秩),不给平均:卡的时长分布很偏。作废的卡不进时长。
- `--since` / `--until` 按卡的派出时间取(UTC,`YYYY-MM-DD` 或 RFC 3339,`--until` 不含当刻);
  `--by` 按单元、实现方或派出的 ISO 周分组。
- 是诊断不是考核:不判断好坏,退出码与数字无关(0;2 用法;4 日志读不了)。

## Boundaries

### Allowed Changes
- src/olp/metrics.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_log_metrics.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-log-metrics.spec.md

### Forbidden
- 不写日志、不改事件格式、不引入外部监控依赖。
- 不拿这些数当闸门或自动改策略的依据。

## Out of Scope

- 人工介入量:日志没有记一步是 agent 做的还是人代做的,要先给事件加"谁做的"。
- 产能(单元在手时间、失联时长):心跳不在日志里,要先把单元状态的变化记成事件。
- 成本(token、费用):要单元在回执里自报。
- 偏离基线自动生成症状卡(接 `olp evo harvest`)。
- 每次都重读整个归档;归档很大时把每张卡的一行存进归档摘要另做。

## Completion Criteria

Scenario: 每张卡一行,汇总的每个数都来自这些行(critical)
  Tags: critical
  Test: each_card_is_one_row_and_the_summary_comes_from_the_rows
  Given 四张卡:一次通过的、打回一次后采认的、作废的、阻塞并有守护告警的未了结卡;两个提问,一个已答一个超时
  When 带 --cards --json 取指标
  Then 采认 2、作废 1、未了结 1,一次通过 1;打回、阻塞、告警各 1;提问 2 答 1 超时 1;派出到采认的中位与 90 分位是那两张卡的时长;打回过的卡各段时长是多轮相加;作废的卡没有总时长,也不进任何时长统计

Scenario: 时间窗与分组只改变算哪些卡(critical)
  Tags: critical
  Test: a_window_and_a_grouping_narrow_what_is_counted
  Given 三张采认的卡,分属两台单元,其中一张晚八天派出
  When 按单元、按周、按实现方分组;用 --since、--until 取
  Then 各组的卡数与时长只来自本组的卡;跨周的卡在下一个 ISO 周;时间窗外的卡不算

Scenario: 压缩过的日志,指标与压缩前一样(critical)
  Tags: critical
  Test: metrics_read_the_archive_too
  Given 六张采认的卡
  When 压缩(只留最近一张)前后各取一次指标
  Then 两次的汇总与每张卡的明细完全相同

Scenario: 只读,用法错与读不了各有退出码
  Test: metrics_write_nothing_and_say_how_they_were_misused
  Given 一个有一张卡的日志
  When 取指标;不给目录、给不认识的分组、给读不懂的日期、给不存在的目录;日志中间有坏行
  Then 取指标后目录与分段逐字节不变;四种用法错退出码 2;坏日志退出码 4

Scenario: 一张卡的时间在它的事件处切段
  Test: a_cards_time_is_split_at_its_events
  Given 一张领取、回执、打回、复验第二轮、再回执、采认的卡
  When 算它的一行
  Then 等认领、在做、等裁决、总时长分别是对应事件之间的时间,在做与等裁决是两轮相加

Scenario: 采认之后又改判算作改判
  Test: a_ruling_changed_after_acceptance_is_counted_as_reversed
  Given 一张采认后被作废的卡、一张从未领取就回执并采认的卡
  When 算各自的一行与汇总
  Then 前者记为改判、结果为作废、没有总时长;后者一次通过,在做的时间从派出算起;作废卡的时长不进统计

Scenario: 时长给中位数与 90 分位
  Test: a_spread_is_the_median_and_the_ninetieth_percentile
  Given 1 到 10 十个数、一个数、没有数
  When 取分布
  Then 分别是 5 与 9、那个数本身、无数据;时长写成两级单位
