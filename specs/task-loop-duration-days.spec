spec: task
name: "Loop 时长显示进位到天"
inherits: project
tags: [tui, autonomy, loop, ux]
estimate: 0.1d
---

## 意图

实测(2026-08-03):新建 loop 的到期时间显示为 `167h 58m`——约 7 天,却要读者
心算换算。`format_loop_duration` 只进位到小时,超过一天就退化为难读的大数字。
本任务补上天级进位:≥24 小时显示 `6d 23h`,其余档位不变。

## 已定决策

- 仅扩展 `format_loop_duration` 一个函数:≥86400 秒显示 `%dd %dh`(天 + 不足
  一天的小时数),≥3600 秒沿用 `%dh %dm`,其余沿用既有 `format_elapsed_secs`。
- 不引入本地化的"天/小时"词汇差异:`d`/`h`/`m` 单位符号与既有 `h`/`m` 一致,
  中英文共用,避免为一个单位再开两条 locale。
- 不改变 `format_elapsed_secs`(其他 UI 依赖它的分钟/秒形态)。

## 边界

### Allowed Changes
- src/app.rs
- src/app/tests.rs
- specs/**

### Forbidden
- 不改变 `format_elapsed_secs` 的输出。
- 不改变 loop chip 的字段构成与顺序。

## 排除范围

- 自主行 chip 超宽截断问题(独立处理)。

## 完成条件

场景: 超过一天显示天与小时
  测试: loop_duration_rolls_over_into_days
  假设 一个 604680 秒(约 6 天 23 小时)的时长
  当 格式化为 loop 时长
  那么 输出为 6d 23h

场景: 不足一天沿用小时分钟
  测试: loop_duration_under_a_day_keeps_hours
  假设 一个 10800 秒(3 小时)的时长
  当 格式化为 loop 时长
  那么 输出包含 3h 且不包含 d 单位

场景: 不足一小时沿用分钟秒
  测试: loop_duration_under_an_hour_keeps_minutes
  假设 一个 134 秒的时长
  当 格式化为 loop 时长
  那么 输出为 2m 14s
