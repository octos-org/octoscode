spec: task
name: "幻影 InProgress 状态的证据驱动看门狗"
inherits: project
tags: [tui, reducer, watchdog, interrupt, hydrate]
depends: [task-terminal-tool-event-monotonicity]
estimate: 1d
---

## 意图

事故（2026-08-17）中客户端在 turn 终态之后被迟到事件拨回 `InProgress`，而
`active_turn()` 为 None、无 staged gate、无 pre-token 标记——这种"幻影活跃态"
让所有输入永久排队且现有 TTL 自愈路径覆盖不到。本任务为 `Store` 增加一个
tick 驱动的看门狗：只在**服务端终态证据**成立时把幻影态复位为 Idle 并排空
队列；证据缺失时先向服务端发起 `session/hydrate` 核对，绝不凭纯时间推断
把未知的活跃 turn 强制置 Idle。

## 已定决策

- 幻影态定义 `phantom_in_progress(session)`：`run_state == InProgress`（不含
  `Blocked`）且 `active_turn().is_none()` 且该 session 无 `pre_token_turns`
  标记、无 `staged_submit_in_flight` gate。
- 终态证据必须带 turn 身份（任一即可）：(a) 该 session **最近一次
  `TurnStarted` 的 turn**（新增 `last_started_turn` 记录）已进入
  `completed_turns` 终态墓碑；(b) `session/hydrate` 结果的 `turns` 中不存在
  `Active`/`Interrupting` 状态。`session_orchestration active=false` 不携带 turn
  身份、可能是旧回合的迟到帧，**不作为复位证据**。
- 触发时机：`Store::reconcile_phantom_run_state(now)` 由 event loop 每 tick 调用；
  仅当幻影态持续 ≥ `PHANTOM_RUN_STATE_PROBE_SECS`（10 秒，读
  `run_state_elapsed_secs()`）才动作。
- 动作顺序：有证据 (a) → 立即 `set_run_state_idle()`、写状态文案
  `status.phantom_turn_reconciled`、调用 `submit_next_pending_if_idle()` 并把
  命令 `enqueue_autonomy_hydration`；无证据但服务端广告 `session/hydrate` →
  每个幻影态 episode 只发一次 `hydrate_session_state_command`（以
  `phantom_probe_sent` 标记去重），结果落地后凭证据 (b) 复位；无证据且无
  hydrate 能力 → 只写状态提示 `status.phantom_turn_hint`，不改 run_state。
- 复位路径复用 `apply_session_hydrate_result`：hydrate 结果满足证据 (b) 且本地
  仍处幻影态时执行同一复位动作。
- 新增用户可见文案同时落 `locales/en.yml` 与 `locales/zh.yml`。

## 边界

### Allowed Changes
- src/store.rs
- src/model.rs
- src/event_loop.rs
- locales/en.yml
- locales/zh.yml
- specs/task-stuck-run-state-watchdog.spec.md

### Forbidden
- 不新增 crate 依赖。
- 不修改服务端协议或 `octos-core` 事件结构。
- 不在无服务端证据、无用户显式操作时凭 TTL 直接置 Idle。
- 不改动 `Blocked`（approval/question 等待）状态的处理。
- 不改动活跃回合（`live_reply` 已绑定或 pre-token 窗口内）的 run-state 行为。

## 完成条件

### Rule: phantom-reset-on-evidence — 有服务端终态证据时复位幻影态
场景: 最近启动的 turn 已终态时看门狗复位幻影态并排空队列（critical）
  标签: critical
  测试: watchdog_resets_phantom_in_progress_when_last_started_turn_is_terminal
  假设 session 处于幻影态且其 `last_started_turn` 已在 `completed_turns` 中
  并且 幻影态已持续超过 `PHANTOM_RUN_STATE_PROBE_SECS`
  并且 队列中有一条待提交消息
  当 event loop 调用 `reconcile_phantom_run_state`
  那么 `set_run_state_idle()` 生效，幻影态解除（`phantom_in_progress` 为 false）
  并且 `submit_next_pending_if_idle()` 的 `SubmitPrompt` 被 `enqueue_autonomy_hydration` 排入 follow-up 队列，并以 pre-token 标记接管 run state
  并且 状态栏文案为 `status.phantom_turn_reconciled`

场景: 有证据时优先复位而不发 hydrate 探测（动作顺序）
  测试: watchdog_prefers_evidence_over_probe
  假设 session 处于幻影态、`last_started_turn` 已终态且服务端广告 `session/hydrate`
  当 看门狗超过 `PHANTOM_RUN_STATE_PROBE_SECS`
  那么 run state 变为 Idle
  并且 follow-up 队列中没有 `HydrateSession` 命令

场景: hydrate 结果显示全部 turn 终态时复位幻影态（critical）
  标签: critical
  测试: hydrate_all_terminal_turns_resets_phantom_in_progress
  假设 session 处于幻影态且 `last_started_turn` 尚未终态
  当 `apply_session_hydrate_result` 收到 `turns` 全部为 `Completed`/`Errored`/`Interrupted` 的结果
  那么 幻影态解除且队列中的消息作为 `SubmitPrompt` 被提交（由 pre-token 标记接管 run state）

### Rule: phantom-probe-before-reset — 无证据时先向服务端核对
场景: 无证据时看门狗只发一次 hydrate 探测
  测试: watchdog_probes_hydrate_once_when_no_terminal_evidence
  假设 session 处于幻影态且无任何终态证据且服务端广告 `session/hydrate`
  当 看门狗连续两个 tick 都超过 `PHANTOM_RUN_STATE_PROBE_SECS`
  那么 follow-up 队列中恰有一条 `HydrateSession` 命令
  并且 run state 仍为 InProgress

场景: hydrate 结果仍有活跃 turn 时不复位
  测试: hydrate_with_active_turn_keeps_in_progress
  假设 session 处于幻影态
  当 收到 `turns` 含 `Active` 的 `SessionHydrate` 结果
  那么 run state 仍为 InProgress
  并且 队列不被排空

场景: 无证据且无 hydrate 能力时不强制置 Idle
  测试: watchdog_without_evidence_or_hydrate_only_hints
  假设 session 处于幻影态且服务端未广告 `session/hydrate`
  当 看门狗超过 `PHANTOM_RUN_STATE_PROBE_SECS`
  那么 run state 仍为 InProgress
  并且 状态栏文案为 `status.phantom_turn_hint`
  并且 follow-up 队列为空

场景: 状态文案在两种语言包中都存在
  测试: phantom_status_keys_exist_in_both_locales
  当 读取 `locales/en.yml` 与 `locales/zh.yml`
  那么 `status.phantom_turn_reconciled` 与 `status.phantom_turn_hint` 在两个文件中均有定义

### Rule: phantom-guard-excludes-live — 合法活跃态不受看门狗影响
场景: 首 token 前的真实提交不被复位
  测试: watchdog_leaves_pre_token_turn_alone
  假设 session 有新鲜的 `pre_token_turns` 标记且 run state 为 InProgress
  并且 `last_started_turn` 已终态
  当 看门狗运行
  那么 run state 仍为 InProgress
  并且 follow-up 队列为空

场景: Blocked 等待不被复位
  测试: watchdog_leaves_blocked_turn_alone
  假设 run state 为 Blocked 且无 live turn
  当 看门狗超过 `PHANTOM_RUN_STATE_PROBE_SECS`
  那么 run state 仍为 Blocked

场景: 新 TurnStarted 之后旧回合的迟到 settlement 不构成证据（乱序）
  测试: late_orchestration_settlement_after_new_turn_start_is_not_evidence
  假设 已收到 `TurnStarted(B)`，随后到达旧回合 A 的迟到 `session_orchestration active=false`
  并且 B 的 pre-token 标记过期、B 仍未终态
  当 看门狗超过 `PHANTOM_RUN_STATE_PROBE_SECS`
  那么 run state 仍为 InProgress
  并且 follow-up 队列中没有 `SubmitPrompt`
  并且 只允许出现一条 `HydrateSession` 探测

## 排除范围

- Esc/Ctrl+C 在幻影态下的显式出口（task-esc-reconciles-phantom-turn）。
- steer 文本保留与终态 re-stage（task-steer-retained-until-echo）。
- 服务端返还未消费 steer（octos F4）。
