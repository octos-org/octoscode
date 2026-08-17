spec: task
name: "steer 文本保留至 echo 落地，终态时未确认者重新入队"
inherits: project
tags: [tui, reducer, steer, turn-lifecycle]
depends: [task-terminal-tool-event-monotonicity]
estimate: 1d
---

## 意图

`turn/steer` 返回 `steered:true` 只表示服务端把输入放进了 pending-input
buffer（受理确认），不表示它被执行循环消费。事故中一条已受理的 steer 在
turn 被中断时被服务端 drain 丢弃，客户端因 RPC 成功已弹出
`pending_turn_steers`，transcript 显示"已发送"，文本静默丢失。本任务把客户端
的确认边界从受理提升到 **echo 落地**：`steered:true` 后把 steer 文本保留在
`retained_steers` 中，直到对应 `UserMessage` echo 到达；turn 终态时仍未确认的
steer 从 transcript 撤回并重新入队，由既有的终态 drain 作为新 turn 提交。

## 已定决策

- 新增 `AppState::retained_steers: Vec<RetainedSteer{session_id, turn_id, prompt}>`；
  `apply_turn_steered_event` 在 `steered:true` 时把弹出的 `PendingTurnSteer` 移入
  `retained_steers`；`steered:false` 不保留（它已按普通提交重新键控）。
- 确认（reap）：`apply_user_row_echo` 的 promotion 分支按 `(session_id, content)`
  取最早匹配的 retained 条目移除——与 `withdraw_steered_user_prompt` 一样以内容
  为唯一 join key（steer 没有 client_message_id，且与 live turn 共享 turn_id）。
- 协议分层（v2，随 octos `event.turn_steer_dropped.v1`）：服务端广告该 feature 时，
  它保证未消费的 steer 在终态帧之前以 `turn/steer_dropped` 返还，因此终态处
  **不再兜底 re-stage**，只把该 turn 剩余的 retained 视为已消费并清除（避免"已消费
  但 echo 丢失"被误重提）；未广告（旧服务端）才执行下面的终态 re-stage。
- 终态 re-stage（仅旧服务端）：`commit_live_reply` 与 `fail_live_reply` 在
  `release_staged_gate_for_turn` 之后、重复终态早退之前，取出该 `(session, turn)`
  的全部 retained 条目，逐条 `withdraw_steered_user_prompt` 后按原顺序放到队列
  **前部**（它们的输入时间早于任何终态后暂存的消息），写状态文案
  `status.steer_restaged_after_turn_end`；随后由函数尾部既有的
  `submit_next_pending_if_idle()` 提交。
- 重复终态（`is_turn_completed` 早退路径）不再有 retained 条目，天然幂等。
- 重连对账：`RetainedSteer` 记录保留时的 `prior_matching_user_count`（该内容在
  optimistic 插入前的用户行数）；snapshot 重建（`AppUiEvent::Snapshot`，在
  `restore_optimistic_user_messages` 之前）与 hydrate 替换 `messages` 之后，调用
  `settle_retained_steers_reflected_by_history`：若 canonical 历史中该内容的用户
  行数已超过基线，说明服务端已持久化该 steer，直接 reap，避免终态时二次提交。
- 新增用户可见文案同时落 `locales/en.yml` 与 `locales/zh.yml`。

## 边界

### Allowed Changes
- src/store.rs
- src/model.rs
- locales/en.yml
- locales/zh.yml
- specs/task-steer-retained-until-echo.spec.md

### Forbidden
- 不新增 crate 依赖。
- 不修改服务端协议、`TurnSteerParams` 或 `octos-core` 事件结构。
- 不改变 `steered:false` 与 attributed-error 回退的现有行为。
- 不改变 echo promotion 对 transcript 行的去重语义。

## 完成条件

### Rule: steer-retained-until-echo — 受理不等于确认
场景: steered:true 后文本保留，echo 到达后释放
  测试: steered_true_retains_prompt_until_echo
  假设 一个 live turn 已收到 `steered:true`
  那么 `retained_steers` 含该 steer 且 `pending_turn_steers` 为空
  当 `apply_user_row_echo` 收到同内容的 `UserMessage` echo 并完成 promotion
  那么 `retained_steers` 为空
  并且 `pending_messages` 为空

场景: steered:false 不保留
  测试: steered_false_is_not_retained
  假设 一个 live turn 上发出了 steer
  当 收到 `steered:false` 结果
  那么 `retained_steers` 为空

### Rule: unconfirmed-steer-restaged — 终态时未确认的 steer 重新入队
场景: 中断终态时未 echo 的 steer 重新入队并作为新 turn 提交（critical）
  标签: critical
  测试: unechoed_steer_is_restaged_on_interrupt_terminal
  假设 一个 live turn 已收到 `steered:true` 且没有 echo
  当 收到该 turn 的 `TurnError(interrupted)`
  那么 `fail_live_reply` 返回该 steer 文本的 `SubmitPrompt`
  并且 transcript 中该文本只出现一次
  并且 状态栏文案为 `status.steer_restaged_after_turn_end`
  并且 `retained_steers` 为空

场景: 完成终态时未 echo 的 steer 同样重新入队（critical）
  标签: critical
  测试: unechoed_steer_is_restaged_on_completed_terminal
  假设 一个 live turn 已收到 `steered:true` 且没有 echo
  当 收到该 turn 的 `TurnCompleted`
  那么 `commit_live_reply` 返回该 steer 文本的 `SubmitPrompt`
  并且 `retained_steers` 为空

场景: 已 echo 的 steer 在终态时不重复提交
  测试: echoed_steer_is_not_restaged_on_terminal
  假设 一个 live turn 已收到 `steered:true` 且 echo 已落地
  当 收到该 turn 的 `TurnCompleted`
  那么 `commit_live_reply` 不返回 `SubmitPrompt`
  并且 `pending_messages` 为空

场景: 未确认 steer 排在终态后暂存消息之前（FIFO）
  测试: restaged_steer_precedes_messages_staged_after_it
  假设 一个 live turn 已收到 `steered:true` 且之后又有一条消息被暂存
  当 收到该 turn 的 `TurnError(interrupted)`
  那么 首先提交的是 steer 文本
  并且 `pending_messages` 中剩下后暂存的消息

场景: 重复终态不会二次 re-stage
  测试: duplicate_terminal_does_not_restage_steer_twice
  假设 一个未 echo 的 steer 已在第一次终态时重新入队并提交
  当 同一 turn 的终态事件再次到达
  那么 `pending_messages` 不新增条目
  并且 `retained_steers` 为空

场景: 新服务端：已消费但 echo 丢失的 steer 在终态时不重复提交（critical）
  标签: critical
  测试: consumed_steer_with_lost_echo_is_not_resubmitted_when_server_settles_steers
  假设 服务端广告 `event.turn_steer_dropped.v1`，一个 live turn 已收到 `steered:true`、没有 echo、也没有 `turn/steer_dropped`
  当 收到该 turn 的 `TurnCompleted`
  那么 `commit_live_reply` 不返回 `SubmitPrompt`
  并且 `retained_steers` 为空且 `pending_messages` 为空

场景: 旧服务端：终态兜底仍然生效（兼容）
  测试: legacy_server_without_steer_dropped_feature_keeps_terminal_restage
  假设 服务端未广告 `event.turn_steer_dropped.v1`，一个未 echo 的 steer
  当 收到该 turn 的 `TurnError(interrupted)`
  那么 `fail_live_reply` 返回该 steer 文本的 `SubmitPrompt`

场景: 与原 prompt 同内容的 steer 也能被正确 re-stage
  测试: same_content_steer_is_restaged_without_touching_original_row
  假设 steer 文本与 live turn 的原始 prompt 完全相同且未 echo
  当 收到该 turn 的 `TurnError(interrupted)`
  那么 原始 prompt 行保留
  并且 steer 文本作为 `SubmitPrompt` 提交

### Rule: reconnect-reconciles-retained — 重连后已持久化的 steer 不再重复提交
场景: snapshot 中已有该 steer 的 canonical 行时 reap（critical）
  标签: critical
  测试: snapshot_with_persisted_steer_row_reaps_retained_steer
  假设 一个 live turn 已收到 `steered:true`、未收到 echo
  当 收到的 `AppUiEvent::Snapshot` 历史中该 steer 文本的用户行数超过 `RetainedSteer` 的 `prior_matching_user_count`
  那么 `settle_retained_steers_reflected_by_history` 使 `retained_steers` 为空
  并且 随后的终态不返回该文本的 `SubmitPrompt`
  并且 transcript 中该文本只出现一次

场景: snapshot 中没有该 steer 行时继续保留
  测试: snapshot_without_steer_row_keeps_retained_steer
  假设 一个 live turn 已收到 `steered:true`、未收到 echo
  当 收到的 `AppUiEvent::Snapshot` 历史中不包含该 steer 文本
  那么 `retained_steers` 仍含该 steer
  并且 随后的终态返回该文本的 `SubmitPrompt`

场景: hydrate 历史中已有该 steer 行时 reap
  测试: hydrate_with_persisted_steer_row_reaps_retained_steer
  假设 一个 live turn 已收到 `steered:true`、未收到 echo
  当 `apply_session_hydrate_result` 替换的 `messages` 中已包含该 steer 文本的用户行
  那么 `retained_steers` 为空

场景: re-stage 文案在两种语言包中都存在
  测试: steer_restage_status_key_exists_in_both_locales
  当 读取 `locales/en.yml` 与 `locales/zh.yml`
  那么 `status.steer_restaged_after_turn_end` 在两个文件中均有定义

## 排除范围

- 服务端在 turn 退出时返还未消费 steer（octos F4）——落地后可把返还的输入
  作为更强的确认/否认信号接入本机制。
- attributed-error 回退路径的既有行为（已由 `steer_error_frame_restages_the_prompt` 覆盖）。
- 幻影态看门狗与 Esc 出口（另两个 task spec）。
