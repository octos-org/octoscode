spec: task
name: "Loop 指示行溢出可见化"
inherits: project
tags: [tui, autonomy, loop, layout]
estimate: 0.25d
---

## 意图

实测(2026-08-03):两个 loop 的 chip 一行放不下时,自主指示行被 ratatui 硬切,
第二个 chip 的右半("… · 167h 59m")无声消失,用户看不出还有内容被截断,也不
知道被截了多少。本任务让该行按可用宽度裁剪并给出明确的溢出提示:能放下的
chip 完整显示,放不下的折叠为 `+N more`(可用 `/loop list` 查看全部)。

## 已定决策

- 该行仍是**单行**(常驻行不长高,保持既有 `autonomy_indicator_height` 的
  预算契约);溢出以计数提示表达,而非换行或省略号。
- 裁剪单位是**整颗 chip**:优先保证每个显示出来的 chip 完整可读,而不是把
  末个 chip 切成半截字符。
- 溢出提示为 `+N more`,紧跟最后一个完整 chip;其后仍复用既有
  `clip_line_spans` 做最终硬边界防护(极窄终端下的兜底)。
- 头部("↻ Loops: N active …")永远优先保留;宽度不足以放下任何 chip 时,
  只渲染头部与溢出提示。

## 边界

### Allowed Changes
- src/app.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不改变该行的行数(仍为 1),不破坏 reserve==render。
- 不改变 chip 的字段构成与顺序。
- 不改变目标(Goal)行与计划(Plan)行的渲染。

## 排除范围

- 让自主行在多 loop 时折行显示。
- 把 chip 做成可交互的选择列表。

## 完成条件

场景: 宽度不足时以计数提示替代被截断的 chip
  测试: loop_row_replaces_overflowing_chips_with_a_count
  假设 三个活跃 loop,终端宽度只够容纳第一个 chip
  当 渲染自主指示行
  那么 该行包含第一个 loop 的标签
  并且 包含 more 溢出提示
  并且 行宽不超过给定宽度

场景: 宽度充足时不出现溢出提示
  测试: loop_row_without_overflow_has_no_more_hint
  假设 一个活跃 loop 且终端宽度充足
  当 渲染自主指示行
  那么 该行不包含 more 溢出提示

场景: 溢出时仍保留头部计数
  测试: loop_row_keeps_header_when_chips_overflow
  假设 三个活跃 loop 且宽度极窄
  当 渲染自主指示行
  那么 该行仍包含 Loops 头部文本
