spec: task
name: "迟到工具事件不得复活终态回合"
inherits: project
tags: [tui, reducer, interrupt, tool-lifecycle]
estimate: 0.5d
---

## 意图

修复回合已经以 interrupted/error 终止后，同一回合的迟到工具生命周期帧——
`UiNotification::ToolStarted`/`ToolProgress`、`progress/updated`
（`UiProgressEvent`，含 tool_progress 与 status_word）、以及 v2 envelope 的
`PayloadV2::ToolStart`/`ToolProgress`——再次把客户端状态拨为 `InProgress` 的竞态。终态必须对同一
`session_id` 与 `turn_id` 保持单调，使排队输入可以继续提交，并避免产生
无法被终态归档的 running activity。

## 已定决策

- 终态墓碑：以现有 `AppState::completed_turns` 作为 `(session_id, turn_id)` 的真相，
  不另建基于时间或 interrupt marker 的第二套真相。
- reducer 守卫：在 `UiNotification` reducer 的工具事件入口、`apply_progress`
  （带 `turn_id` 的 `progress/updated`）以及 `apply_envelope_v2` 的
  `ToolStart`/`ToolProgress` 分支检查终态墓碑；终态后的帧不改变 run state、
  status 或 activity。会话级簿记（usage、retry）与回合无关，保持更新。
- 守卫位置：在各 reducer 入口按 `(session_id, turn_id)` 查墓碑，而不是改
  `set_run_state_in_progress()`——终态处理恰好会清除该 setter 依赖的
  interrupt marker 与 live reply（由"会话切换后迟到帧仍保持惰性"场景覆盖）。
- 活跃工具兼容：尚未终止的工具通知保持现有 activity 与 run-state 行为。

## 边界

### Allowed Changes
- src/store.rs
- specs/task-terminal-tool-event-monotonicity.spec.md

### Forbidden
- 不新增 crate 依赖。
- 不修改服务端协议或 `octos-core` 事件结构。
- 不以固定 TTL 推断合法长跑工具已经终止。
- 不改变活跃回合的工具 activity 更新行为。

## 完成条件

场景: interrupt 终态后的迟到工具事件被忽略（critical）
  标签: critical
  测试: tool_events_after_interrupt_terminal_do_not_resurrect_the_turn
  假设 一个回合已收到 `TurnError(interrupted)` 并写入 `AppState::completed_turns` 终态墓碑
  当 `UiNotification` reducer 收到同一 `session_id` 与 `turn_id` 的 `ToolStarted` 和 `ToolProgress`
  那么 run state 保持 Idle
  并且 activity 不新增孤儿 running 行
  并且 下一条 prompt 立即产生 `SubmitPrompt` 而不是进入永久队列

场景: interrupt 终态后的迟到 progress/updated 帧被忽略（critical）
  标签: critical
  测试: late_progress_frames_after_interrupt_terminal_do_not_resurrect_the_turn
  假设 一个回合已收到 `TurnError(interrupted)` 并写入终态墓碑
  当 `apply_progress` 收到同一 `session_id` 与 `turn_id` 的 tool_progress 与 status_word 帧
  那么 run state 保持 Idle
  并且 activity 不新增孤儿行
  并且 下一条 prompt 立即产生 `SubmitPrompt`

场景: interrupt 终态后的迟到 envelope 工具帧被忽略（critical）
  标签: critical
  测试: late_envelope_tool_frames_after_interrupt_terminal_do_not_resurrect_the_turn
  假设 一个回合已收到 `TurnError(interrupted)` 并写入终态墓碑
  当 `apply_envelope_v2` 收到同一回合的 `PayloadV2::ToolStart` 与 `PayloadV2::ToolProgress`
  那么 run state 保持 Idle
  并且 activity 不新增孤儿 running 行
  并且 下一条 prompt 立即产生 `SubmitPrompt`

场景: 会话切换后迟到帧仍保持惰性（守卫位置）
  测试: late_turn_events_stay_inert_across_a_session_switch
  假设 终态之后 interrupt marker 与 live reply 已被清除且用户切换到另一会话再切回
  当 三种形态的迟到工具帧全部到达
  那么 run state 保持 Idle
  并且 interrupt marker 保持已清除
  并且 队列可以继续提交

场景: 迟到帧不干扰后继回合
  测试: late_tool_events_for_a_dead_turn_do_not_touch_a_successor_turn
  假设 死亡回合之后已经有一个新的活跃回合
  当 死亡回合的迟到 progress 与 envelope 帧到达
  那么 activity 不新增行
  并且 后继回合仍是唯一的 live turn

场景: 活跃回合的工具通知保持原行为
  测试: tool_notifications_update_activity_card_state
  假设 活跃工具兼容要求一个尚未终止且没有 completed-turn 墓碑的回合
  当 收到 `ToolStarted`、`ToolProgress` 与 `ToolCompleted`
  那么 activity 记录工具名称、进度、输出与完成状态

## 排除范围

- octos 服务端在 turn 退出时返还未消费 steer（F4）。
- 客户端将 steer 保留至 consumed/committed（F5）。
- 基于 hydrate/status 的 stuck-state watchdog（F3）。
- interrupt 关联日志、Esc/exit 出口与 fd 累积调查。
