spec: task
name: "Composer paste chip 后续输入保持可见"
inherits: project
tags: [tui, composer, input, paste]
estimate: 0.25d
---

## 意图

大段 paste 会折叠为 `[paste …]` chip。即使整份草稿超过普通输入的安全折叠阈值，
用户随后键入的内容也必须显示在 chip 外，不能只改变 chip 的字符计数。

## 已定决策

- paste span 是 chip 的真实字节范围；普通字符或小型 paste event 不得无条件清除它。
- 光标位于原子 chip 内时，视觉上位于 chip 末尾，后续输入也插入到 span 末尾。
- chip 摘要只统计 paste span；后续输入作为可见前缀或后缀渲染。
- 低于普通输入安全阈值的已编辑 paste 仍按既有行为重新展开为 inline。

## 边界

### Allowed Changes
- src/model.rs
- specs/**

### Forbidden
- 不改变 paste 折叠阈值。
- 不改变提交、终端 bracketed-paste 或 paste 原子删除语义。
- 不引入新依赖。

## 完成条件

场景: 超大 paste 后连续键入的字符显示在 chip 外
  测试: typed_text_after_huge_paste_stays_visible_outside_the_chip
  假设 composer 含 40 行真实 paste，已超过普通输入的安全折叠阈值
  当 用户连续键入 "xy"
  那么 paste 仍折叠为 chip
  并且 "xy" 显示在 chip 后方
  并且 chip 的行数与字符数只统计原始 paste
