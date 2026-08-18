spec: task
name: "消费 turn/steer_dropped：服务端返还的 steer 精确重新入队"
inherits: project
tags: [tui, reducer, steer, turn-lifecycle, protocol]
depends: [task-steer-retained-until-echo]
estimate: 1d
---

## 意图

octos F4 已让服务端在 turn 退出时把受理但未消费的 steer 输入以
`turn/steer_dropped` 通知按顺序返还（含 `reason`）。本任务把 octos-core 重新 pin
到含该事件的 rev，并在客户端消费它，与 F5 的终态兜底组成端到端闭环：谁先消费到对应
的 retained steer，谁负责重新入队；另一方必须 no-op。这样无论
`turn/steer_dropped` 先于还是晚于终态到达，同一段文本都只会重新提交一次，且服务端
返还的文本永远不会在没有本地 retained 记录的情况下被注入队列。

## 已定决策

<!-- lint-ack: external-io-error-strength — "不注入"是纯 reducer 状态断言，与外部 I/O 无关；关键词命中系误报 -->
<!-- lint-ack: verification-metadata-suggestion — 解码与不注入两个场景均为进程内纯函数/reducer 测试，无外部 I/O -->

- 依赖：`Cargo.toml` 中 `octos-core` 的 `rev` 更新到 `f6d5ef550f49189850646e7421cf21063b771895`
  （octos 分支 `fix/return-unconsumed-steer-inputs`），`Cargo.lock` 同步；该 rev 新增的
  `MonitorUpdated`/`MonitorFired`/`MonitorExpired`/`BackgroundActivity` 通知本任务不消费，
  reducer 里显式 `=> None`。
- 版本配对：`backend_ensure::REQUIRED_OCTOS_CORE_REV` 同步更新为该 rev 以通过
  `octos_release_pin_matches_cargo_core_rev`；`REQUIRED_OCTOS_RELEASE` 暂保持
  `v2.0.3-rc.2`——尚无 octos release 包含 `1ff2e3d8`。本次协议变更是纯新增通知，
  老服务端只是不会发送 `turn/steer_dropped`（客户端退化为 F5 终态兜底），因此
  rev 领先 tag 是可接受的过渡态；octos 打出包含该 rev 的 release 后必须把
  `REQUIRED_OCTOS_RELEASE` 提到该 tag。
- 匹配即消费（原子竞态协议）：`UiNotification::TurnSteerDropped` 只在成功按
  `(session_id, turn_id, 文本)` 从 `retained_steers` reap 到对应条目时才执行
  re-stage（`withdraw_steered_user_prompt` + `restage_staged_prompt_front`，保持
  `inputs` 顺序，落到**所属 session** 的队列）；找不到对应 retained 条目的文本一律
  忽略，不注入本地队列。
- 数量语义：同一文本出现 N 次时按 FIFO 精确消费 N 条 retained（不能用集合去重）；
  多余的返还文本视为未匹配。
- 幂等：同一通知重复到达/replay 时，第二次已无 retained 可 reap，因此不新增队列项。
- 与终态的关系（v2）：客户端在 `appui_feature_header_for` 中请求
  `event.turn_steer_dropped.v1`（`APPUI_FEATURE_TURN_STEER_DROPPED_V1`）；服务端广告
  时，真实线序是 `turn/steer_dropped`（若有未消费）→ 终态：dropped 精确重排，终态只
  清除剩余 retained（已消费）而不重排；未广告（旧服务端）则终态走 F5 兜底，此时
  dropped 不会到达。两条路径互斥，同一文本只会重新提交一次。
- `reason` 不改变防丢语义，只进入状态文案
  `status.steer_returned_by_server`（含条数与 reason）；未匹配时写
  `status.steer_returned_unmatched`（诊断，不改队列）。
- 真实入口：transport 的 `notification_to_app_event` 把 `turn/steer_dropped` 解码为
  `AppUiEvent::Protocol(UiNotification::TurnSteerDropped)`；reducer 入口为
  `Store::apply_notification`。
- 同一 turn 多条 steer 的 echo 去重：`record_submitted_user_prompt` 调用的
  `restore_optimistic_user_messages_inner(false)` 不再因"行已存在"而丢弃兄弟 steer 的
  optimistic 记录（否则其 echo 无法 promote、会追加重复行）；snapshot/hydrate 路径的
  `restore_optimistic_user_messages()`（`drop_confirmed = true`）语义不变。
- 新增用户可见文案同时落 `locales/en.yml` 与 `locales/zh.yml`。
- 协议文档完整性：本次 octos-core re-pin 新增到 reducer exhaustive match 的
  `TurnSteerDropped`、`MonitorUpdated`、`MonitorFired`、`MonitorExpired`、
  `BackgroundActivity` 必须同步登记到 `docs/ARCHITECTURE.md` 的 Protocol
  Notifications 清单，并由 `tests/docs_drift.rs` 的通知完整性测试守住。

## 边界

### Allowed Changes
- Cargo.toml
- **/Cargo.toml
- Cargo.lock
- **/Cargo.lock
- src/backend_ensure.rs
- src/store.rs
- src/model.rs
- src/transport.rs
- locales/en.yml
- locales/zh.yml
- docs/ARCHITECTURE.md
- specs/task-consume-turn-steer-dropped.spec.md

### Forbidden
- 不改变 F5 的 echo reap 与终态 re-stage 逻辑（只在其前后加入服务端返还的消费）。
- 不把没有本地 retained 记录的服务端文本写入任何队列或 transcript。
- 不用集合/去重结构匹配返还文本。
- 不新增 crate 依赖（octos-core 仅更新 rev）。

## 完成条件

### Rule: dropped-consumes-retained — 匹配即消费，谁先谁负责
场景: 通知先于终态到达：立即重新入队，终态不重复提交（critical）
  标签: critical
  测试: steer_dropped_before_terminal_restages_once_and_terminal_does_not_resubmit
  假设 服务端广告 `event.turn_steer_dropped.v1`，一个 live turn 已收到 `steered:true` 且未 echo
  当 `apply_notification` 收到该 turn 的 `TurnSteerDropped`，随后收到 `TurnError(interrupted)`
  那么 通知到达时该文本已在队列前部且 `retained_steers` 为空
  并且 终态返回的 `SubmitPrompt` 恰好是该文本一次
  并且 transcript 中该文本只出现一次
  并且 状态栏文案为 `status.steer_returned_by_server`

场景: 终态先于通知到达（旧服务端线序）：迟到通知 no-op（critical）
  标签: critical
  测试: late_steer_dropped_after_terminal_is_a_noop
  假设 服务端未广告该 feature，一个未 echo 的 steer 已由终态兜底重新入队并提交
  当 同一 turn 的 `TurnSteerDropped` 迟到到达
  那么 `pending_messages` 不新增条目
  并且 transcript 中该文本只出现一次
  并且 不返回新的 `SubmitPrompt`

场景: 通知重复到达/replay 幂等
  测试: duplicate_steer_dropped_is_idempotent
  假设 一条 `TurnSteerDropped` 已被消费
  当 相同的通知再次到达
  那么 队列长度不变且 `retained_steers` 仍为空

### Rule: exact-count-fifo — 按数量与顺序精确消费
场景: 多条相同文本按数量精确消费
  测试: same_text_steers_are_consumed_by_count_not_by_set
  假设 同一 live turn 上有三条文本相同的 retained steer
  当 `TurnSteerDropped` 只返还其中两条
  那么 队列前部恰有两条该文本
  并且 `retained_steers` 仍剩一条

场景: 返还顺序即入队顺序
  测试: steer_dropped_preserves_input_order_in_queue
  假设 同一 live turn 上有 "A"、"B" 两条 retained steer
  当 `TurnSteerDropped` 以 `["A","B"]` 返还
  那么 `pending_messages` 前两项依次为 "A"、"B"

### Rule: real-server-order — 真实线序下的端到端闭环（审查 P0）
场景: 新服务端：已消费 + echo 缺失 + 终态 + 无 dropped → 不重复提交（critical）
  标签: critical
  测试: consumed_steer_with_lost_echo_is_not_resubmitted_when_server_settles_steers
  假设 服务端广告 `event.turn_steer_dropped.v1`，一个 live turn 已收到 `steered:true`、没有 echo、也没有 `turn/steer_dropped`
  当 收到该 turn 的 `TurnCompleted`
  那么 不返回 `SubmitPrompt`
  并且 `retained_steers` 为空且 `pending_messages` 为空

场景: 新服务端：未消费 → dropped 先于终态 → 恰好恢复一次（critical）
  标签: critical
  测试: unconsumed_steer_is_recovered_exactly_once_in_real_server_order
  假设 服务端广告 `event.turn_steer_dropped.v1`，一个 live turn 上有两条未 echo 的 steer，其中一条已被服务端消费（echo 已到）
  当 依次收到只含未消费那条的 `TurnSteerDropped` 与 `TurnError(interrupted)`
  那么 终态恰好返回未消费那条的 `SubmitPrompt`
  并且 已消费那条不出现在队列中，transcript 中每条文本只出现一次（`restore_optimistic_user_messages_inner(false)` 保住其 echo promotion）

场景: 断线重连 replay：dropped → connection_closed 终态 → 恰好恢复一次（critical）
  标签: critical
  测试: replayed_connection_closed_terminal_after_dropped_recovers_exactly_once
  假设 服务端广告 `event.turn_steer_dropped.v1`，一条 steer 在连接断开时仍在 buffer
  当 重连后按 ledger 顺序 replay `TurnSteerDropped` 与 `TurnError(connection_closed)`
  那么 终态恰好返回该文本的 `SubmitPrompt` 一次
  并且 重复 replay 同样两帧不再提交

### Rule: routing-and-safety — 归属 session 与不注入
场景: 后台 session 的返还进入其自身队列
  测试: steer_dropped_for_background_session_stays_in_its_own_queue
  假设 retained steer 属于非当前可见的 session
  当 该 session 的 `TurnSteerDropped` 到达
  那么 文本进入 `pending_messages_by_session[该 session]`
  并且 当前 session 的 `pending_messages` 不变

场景: 找不到对应 retained 的文本不得注入（错误路径）
  测试: unmatched_steer_dropped_text_is_never_injected
  假设 本地没有任何 retained steer
  当 收到含任意文本的 `TurnSteerDropped`
  那么 `pending_messages` 与 `pending_messages_by_session` 均不变
  并且 transcript 不新增行
  并且 状态栏文案为 `status.steer_returned_unmatched`

场景: reason 不改变防丢语义但保留在文案中
  测试: steer_dropped_reason_is_surfaced_but_does_not_change_behavior
  假设 两个等价的 retained steer 场景
  当 分别收到 `reason = "interrupted"` 与 `reason = "turn_ended"` 的通知
  那么 两者都重新入队
  并且 状态栏文案分别包含各自的 reason

### Rule: real-entry — 覆盖解码与 reducer 真实入口
场景: transport 把 turn/steer_dropped 解码为 Protocol 事件
  测试: transport_decodes_turn_steer_dropped_notification
  当 `notification_to_app_event("turn/steer_dropped", params)` 被调用
  那么 返回 `AppUiEvent::Protocol(UiNotification::TurnSteerDropped)` 且字段完整

场景: 新 pin 引入的其他通知在 reducer 中显式忽略
  测试: monitor_and_background_activity_notifications_are_ignored
  当 reducer 收到 `MonitorUpdated`/`MonitorFired`/`MonitorExpired`/`BackgroundActivity`
  那么 状态不变且不返回命令

场景: 新 pin 引入的通知全部登记在架构文档
  测试: architecture_documents_every_handled_notification
  当 扫描 `Store::apply_notification` 处理的全部 `UiNotification` variant
  那么 `docs/ARCHITECTURE.md` 的 Protocol Notifications 清单包含 `TurnSteerDropped`、`MonitorUpdated`、`MonitorFired`、`MonitorExpired`、`BackgroundActivity`

场景: rev 与 release 配对常量同步
  测试: octos_release_pin_matches_cargo_core_rev
  当 读取 `Cargo.toml` 的 octos-core rev
  那么 与 `REQUIRED_OCTOS_CORE_REV` 一致

场景: 客户端在能力协商中请求该 feature
  测试: feature_header_requests_turn_steer_dropped
  当 构造现代 `X-Octos-Ui-Features` 头
  那么 其中包含 `event.turn_steer_dropped.v1`；旧服务端基线头不包含

场景: 文案在两种语言包中都存在
  测试: steer_dropped_status_keys_exist_in_both_locales
  当 读取 `locales/en.yml` 与 `locales/zh.yml`
  那么 `status.steer_returned_by_server` 与 `status.steer_returned_unmatched` 在两个文件中均有定义

## 排除范围

- 服务端发送逻辑（octos F4，已完成）。
- Monitor/BackgroundActivity 通知的实际 UI 消费。
- interrupt/steer 关联日志（F7）、fd 累积（F8）。
