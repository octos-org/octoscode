spec: task
name: "配额用尽的友好提示卡"
inherits: project
tags: [tui, error-ux, quota, kimi]
estimate: 0.5d
---

## 意图

实测(2026-08-02 17:13):Kimi k3 计费周期额度用尽时,TUI 展示的是截断的原始
JSON("Provider quota exhausted (moonshot@api/k3) — … HTTP 403 - {\"error\":{\"mes…"),
与普通崩溃无差别,无任何行动指引。配额用尽是**可预期的资源状态**而非故障,
应渲染为专属的友好卡片:说明发生了什么、显示提供商/模型、抽取续费链接、
给出三个行动项(充值/换模型/稍后重试),并把状态栏 state 标为 Quota 而非
通用 Error。

## 已定决策

- 分类收敛为纯函数 `classify_turn_failure(code, message)`:终态 code 为
  `rate_limited` 或 `quota`,或 message 含 "quota exhausted"(不区分大小写)
  → 判为配额用尽;其余一律走既有渲染。
- 卡片以预格式化文本生成于 `turn_error_fallback_message` 的配额分支
  (提交进转录,scrollback 持久):⏳ 标题行含从 message 括号段解析的
  `provider@route/model`;正文丢弃 "HTTP" 起的原始 JSON 尾巴;续费链接从
  message 中首个 `http(s)://` 抽取,单独一行;固定三个行动项(购买加量/
  升级 → `/model` 切换 → 稍后重发)。文案入 en+zh。
- 状态栏:`AppState.quota_exhausted: bool`,配额终态置位,
  `set_run_state_in_progress`(下一回合启动)清除;render_status 在该位为真
  且回合已终态时,state 标签显示 Quota(警示样式)而非通用错误标签。
- 不改变 server 端错误载荷与分类;不做重试自动化。

## 边界

### Allowed Changes
- src/store.rs
- src/model.rs
- src/app/render.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 非配额错误的渲染与文案不变。
- 不改变 TurnError 事件的应用副作用(中断标记清理、审批清理等)。
- 不新增 crate 依赖。

## 排除范围

- 主/子代理两条摘要的去重(另行处理)。
- 自动切换 fallback 模型。

## 完成条件

场景: 配额错误渲染为友好卡片
  测试: quota_error_renders_friendly_card_without_raw_json
  假设 一个 code=quota、message 含 "Provider quota exhausted (moonshot@api/k3) … https://… HTTP 403 - {…}" 的回合错误
  当 生成回合失败摘要
  那么 文本包含 ⏳ 标题与 moonshot@api/k3
  并且 包含抽取出的 https 链接与 /model 行动项
  并且 不包含 "HTTP 403" 与原始 JSON 片段

场景: rate_limited 终态同样命中
  测试: rate_limited_code_renders_friendly_card
  假设 一个 code=rate_limited 的回合错误
  当 生成回合失败摘要
  那么 文本包含 ⏳ 配额卡标题

场景: 普通错误保持原渲染
  测试: generic_error_keeps_legacy_summary
  假设 一个 code=runtime_error、message 为普通报错的回合错误
  当 生成回合失败摘要
  那么 文本为既有 Session Summary 格式
  并且 不包含配额卡标题

场景: 状态栏显示 Quota 而非通用错误
  测试: status_bar_shows_quota_state_after_quota_terminal
  假设 配额终态已应用且回合结束
  当 渲染状态栏
  那么 state 段包含 Quota 字样
