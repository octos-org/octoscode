spec: task
name: "预发布版升级顺序"
inherits: project
tags: [tui, release, update]
---

## 意图

`octoscode update --prerelease` 应选择已发布、非 draft 的最高 SemVer 预发布版本。
GitHub release 列表按创建时间排序，不能把该顺序当作 RC 版本顺序。

## 完成条件

场景: 发布顺序与 RC 版本顺序不同时仍选择最高版本
  测试: prerelease_channel_skips_newer_stable_and_draft_entries
  假设 release 列表包含 v0.3.0-rc.10、v0.3.0-rc.9、v0.3.0-rc.11，
  且还包含稳定版和 draft RC
  当客户端解析预发布频道
  那么选择已发布的 v0.3.0-rc.11

场景: 带数字的 RC 标识按数值递增
  测试: dotted_rc_numbers_advance_in_numeric_order
  假设当前版本依次为 v0.3.0-rc.9 或 v0.3.0-rc.10
  当预发布频道分别提供 v0.3.0-rc.10 或 v0.3.0-rc.11
  那么客户端报告可升级，且同版本不报告可升级

场景: 自升级使用选中的精确 tag
  测试: prerelease_channel_uses_specific_tag_semantics
  假设预发布频道选择了 v0.3.0-rc.11
  当客户端生成自升级请求
  那么目标是 v0.3.0-rc.11 这个精确 tag
