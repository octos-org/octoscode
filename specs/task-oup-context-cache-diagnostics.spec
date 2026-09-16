spec: task
name: "OUP 语义边界与缓存诊断"
inherits: project
tags: [tui, oup, context, prompt-cache, compatibility]
estimate: 0.25d
---

## 意图

OctosCode 应显示由 OUP 决定的缓存代次、最近失效原因和语义头部，帮助操作者判断
上下文压缩后是否仍在复用稳定前缀。客户端只负责兼容解码和展示，不参与缓存、
compaction 或 semantic-boundary 策略，也不得把诊断信息写进对话正文。

## 已定决策

- 新字段是 `context_state` 上的可选加法字段；没有这些字段的旧 OUP 服务端的 lifecycle
  通知同样经 `ClientEvent::ContextLifecycle` 包装（诊断快照为显式空），内含的原 typed
  通知继续走原有 lifecycle reducer。
- 客户端只在双方协商 `context.semantic_cache.v1` 后解释和显示加法字段；modern
  WebSocket header 与 stdio client capability 都请求它，old-server 模式不请求。
- 当前 `octos-core` pin 尚不知道这些字段，因此 transport 在 raw JSON 层先提取它们，
  同时仍让原通知通过 pinned core 完成结构验证和既有状态更新。
- wire label 在接收时移除终端控制序列并限制长度。
- `/context` 的右侧诊断 preview 显示缩短后的 cache epoch、semantic head 以及人类可读的
  invalidation reason；正常 transcript 与可操作菜单行不显示这些值。
- OUP 是所有策略和状态的权威，OctosCode 不推断 epoch、边界或 cache hit。

## 边界

### Allowed Changes
- src/client_event.rs
- src/model.rs
- src/transport.rs
- src/store.rs
- src/menu/**
- locales/**
- tests/m16_context_lifecycle_contract.rs
- specs/task-oup-context-cache-diagnostics.spec

### Forbidden
- 不修改或复制 OUP 的 ContextManager/compaction/cache policy。
- 不把 cache epoch、semantic head 或失效原因注入 prompt、System message 或 transcript。
- 不因新字段缺失而拒绝旧服务端的 lifecycle 通知。
- 不显示完整的长 hash/epoch。

## 完成条件

场景: 新 OUP 通知跨越旧 core pin 保留诊断字段
  测试: lifecycle_notification_retains_additive_cache_diagnostics
  假设 pinned octos-core 不认识新的 context_state 字段
  当 transport 收到含 cache epoch、失效原因和 semantic head 的 normalization 通知
  那么原 typed notification 和四个加法字段都被保留

场景: 旧 OUP 通知同样走 ContextLifecycle 包装且诊断快照为显式空
  测试: legacy_lifecycle_notification_carries_an_explicit_empty_diagnostic_snapshot
  假设 lifecycle context_state 不含任何 cache/semantic 诊断字段
  当 transport 解码通知
  那么它同样被包装为 ClientEvent::ContextLifecycle，原 typed 通知原样内含并照常交给既有 reducer
  并且 diagnostics 为显式 None（不伪造诊断状态），store 据此原子清空上一代诊断

场景: 诊断状态不污染对话
  测试: context_cache_diagnostics_are_stored_outside_the_transcript
  假设双方已协商 context.semantic_cache.v1 且会话收到带诊断的 normalization 通知
  当 store 应用该 client event
  那么 lifecycle ledger 保存诊断且 transcript 消息保持不变

场景: 未协商服务端不解释加法诊断
  测试: unnegotiated_context_cache_diagnostics_are_ignored
  假设服务端 capabilities 未广告 context.semantic_cache.v1
  当 raw lifecycle 通知意外携带 cache/semantic 字段
  那么 typed lifecycle 状态照常更新但诊断字段不进入展示状态

场景: `/context` 只在诊断 preview 中简洁显示
  测试: context_menu_renders_compact_cache_diagnostics_in_preview_only
  假设 OUP 已报告长 cache epoch、semantic head 和失效原因
  当用户打开 `/context`
  那么右侧 preview 显示缩短的 epoch 与人类可读标签
  并且 action rows 不包含原始诊断值

场景: 不可信诊断标签安全且有界
  测试: context_diagnostic_wire_labels_are_sanitized_and_bounded
  假设服务端标签带终端控制序列或超长内容
  当 transport 提取诊断
  那么控制序列被移除且每个字段被限制为固定最大长度

场景: 诊断值不进入真实 transcript 渲染路径
  测试: cache_diagnostics_are_optional_and_do_not_leak_into_lifecycle_summary
  假设 store 应用了携带四个诊断字段的 normalization ContextLifecycle 事件
  当 通过真实 finalized_history_lines 渲染 transcript
  那么 transcript 行与 lifecycle summary 都不含任何诊断值
  并且 `/context` ledger 保留全部四个值
