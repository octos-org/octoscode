spec: task
name: "模型切换的待重启状态"
inherits: project
tags: [tui, oup, model, runtime, ux]
estimate: 0.125d
---

## 意图

当 OUP 接受 `profile/llm/select` 但返回 `restart_required=true` 时，OctosCode 必须明确
说明选择已保存、尚未生效；footer 与 model catalog 继续显示当前进程实际使用的模型，
直到重连后的 status/catalog 刷新确认新 runtime。

## 已定决策

- `restart_required` 是向后兼容的可选响应字段，旧服务端缺失时默认为 false。
- applied + restart_required 显示 `Model saved; restart required`，不显示 `Model selected`。
- pending 选择不覆盖 session runtime status、runtime policy stamp 或 catalog selected marker。

## 边界

### Allowed Changes
- src/model.rs
- src/transport.rs
- src/store.rs
- specs/task-model-selection-restart-required.spec

### Forbidden
- 不自行重启 OUP。
- 不把 persisted selection 当作当前进程的 effective model。

## 完成条件

场景: wire response 保留待重启语义
  测试: model_select_restart_required_is_preserved_and_reported_as_pending
  假设 OUP 返回 applied=true 与 restart_required=true
  当 transport 解码 profile/llm/select 响应
  那么事件保留 restart_required 并提示模型已保存、需要重启

场景: footer 继续显示 effective model
  测试: restart_required_model_selection_keeps_effective_model_until_reconnect
  假设当前 runtime/catalog 是旧模型且用户保存了新模型
  当 select 结果要求重启
  那么 runtime status、runtime policy stamp 与 catalog selected marker 仍指向旧模型
  并且状态栏明确提示 restart required
