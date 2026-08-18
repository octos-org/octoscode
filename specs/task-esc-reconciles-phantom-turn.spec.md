spec: task
name: "Esc 在幻影 InProgress 态下的显式出口"
inherits: project
tags: [tui, reducer, interrupt, keybinding]
depends: [task-stuck-run-state-watchdog]
estimate: 0.5d
---

## 意图

事故中用户按 Esc 得到「No active turn to interrupt」，同时状态栏却显示
Working、消息持续排队——Esc 没有给出任何出口。本任务让 `interrupt_command()`
在幻影态（run state InProgress、无 live turn、无 pre-token 标记、无 staged
gate）下不再只报错，而是复用看门狗的证据驱动复位：有证据即复位并排空队列，
无证据先发 hydrate 探测，用户再次按 Esc 视为显式指令强制复位。退出路径
（`/exit`、Ctrl+Q）在该状态下必须保持可用。

## 已定决策

<!-- lint-ack: decision-coverage — `phantom_in_progress` 的判定本身由 task-stuck-run-state-watchdog 的场景证明，本 spec 的 esc 场景均以该谓词为前置 -->
<!-- lint-ack: verification-metadata-suggestion — 探测场景是纯 reducer 单元测试（命令只入 follow-up 队列，不发生外部 I/O） -->

- 幻影态判定复用 `phantom_in_progress(session)`（定义见 task-stuck-run-state-watchdog）。
- `interrupt_command()` 分支顺序：有 live turn → 现有中断路径不变；幻影态且有
  终态证据 → 立即复位（`set_run_state_idle()` + `submit_next_pending_if_idle()`）；
  幻影态、无证据、hydrate 可用且本 episode 未探测 → 返回
  `hydrate_session_state_command`，状态文案 `status.phantom_turn_probing`；
  幻影态且（已探测过 或 无 hydrate 能力）→ 用户显式操作视为证据，直接复位并写
  `status.phantom_turn_reset_by_user`；非幻影态且无 live turn → 保持
  `status.no_active_turn_interrupt`。
- event loop 的 `KeyCode::Esc` 分支在 `active_turn()` 为 None 时也要把幻影态
  交给 `interrupt_command()`（现状只在有 live turn 时调用它，幻影态被跳过）；
  Ctrl+C 分支已无条件调用 `interrupt_command()`。
- 用户显式复位（`status.phantom_turn_reset_by_user`）只由 `interrupt_command()` 触发，`reconcile_phantom_run_state` 本身不做这一步。
- `LocalAction::Exit`（`/exit`）与 Ctrl+Q 不读取 run state，本任务只补测试证明。
- 新增用户可见文案同时落 `locales/en.yml` 与 `locales/zh.yml`。

## 边界

### Allowed Changes
- src/store.rs
- src/model.rs
- src/event_loop.rs
- locales/en.yml
- locales/zh.yml
- specs/task-esc-reconciles-phantom-turn.spec.md

### Forbidden
- 不新增 crate 依赖。
- 不修改服务端协议或 `octos-core` 事件结构。
- 不改变有 live turn 时 Esc 的中断语义与 `turn/interrupt` 参数。
- 不改变 `Blocked` 状态下 Esc 的行为。

## 完成条件

### Rule: esc-reconciles-phantom — Esc 在幻影态下走证据驱动复位
场景: 有证据时 Esc 直接复位并排空队列（critical）
  标签: critical
  测试: esc_in_phantom_state_with_evidence_resets_and_drains
  假设 session 满足 `phantom_in_progress` 且 `last_started_turn` 已终态且队列中有一条消息
  当 调用 `interrupt_command()`
  那么 run state 变为 Idle
  并且 返回的命令是该消息的 `SubmitPrompt`
  并且 状态栏文案为 `status.phantom_turn_reconciled`

场景: 无证据时第一次 Esc 探测、第二次 Esc 强制复位（critical）
  标签: critical
  测试: esc_in_phantom_state_probes_then_second_esc_forces_idle
  假设 session 处于幻影态、无证据且服务端广告 `session/hydrate`
  当 第一次调用 `interrupt_command()`
  那么 返回 `HydrateSession` 命令且状态栏文案为 `status.phantom_turn_probing`
  并且 run state 仍为 InProgress
  当 第二次调用 `interrupt_command()`
  那么 run state 变为 Idle
  并且 状态栏文案为 `status.phantom_turn_reset_by_user`

场景: 无 hydrate 能力时 Esc 作为显式操作直接复位
  测试: esc_in_phantom_state_without_hydrate_resets_on_user_action
  假设 session 处于幻影态且服务端未广告 `session/hydrate`
  当 调用 `interrupt_command()`
  那么 run state 变为 Idle
  并且 状态栏文案为 `status.phantom_turn_reset_by_user`

场景: 看门狗不做用户显式复位（错误路径：探测失败仍需人按 Esc）
  测试: watchdog_never_forces_reset_without_evidence_even_after_probe
  假设 session 满足 `phantom_in_progress`、hydrate 探测已发出但结果未返回
  当 `reconcile_phantom_run_state` 再次运行
  那么 run state 仍为 InProgress
  但是 随后调用 `interrupt_command()` 时 run state 变为 Idle 且文案为 `status.phantom_turn_reset_by_user`

### Rule: esc-key-reaches-phantom-recovery — 真实按键入口必须走到恢复逻辑
场景: 真实 Esc 键在幻影态下先探测、再复位（critical）
  标签: critical
  测试: esc_key_in_phantom_state_probes_then_second_esc_resets
  假设 session 处于幻影态、无证据且服务端广告 `session/hydrate`
  当 event loop 处理第一次 `KeyCode::Esc`
  那么 返回 `KeyAction::Send(HydrateSession)`
  当 event loop 处理第二次 `KeyCode::Esc`
  那么 run state 变为 Idle 且文案为 `status.phantom_turn_reset_by_user`

场景: 真实 Esc 键在有证据时直接排空队列（critical）
  标签: critical
  测试: esc_key_in_phantom_state_with_evidence_sends_drained_submit
  假设 session 处于幻影态、`last_started_turn` 已终态且队列中有一条消息
  当 event loop 处理 `KeyCode::Esc`
  那么 返回该消息的 `KeyAction::Send(SubmitPrompt)`

场景: 真实 Ctrl+C 键在幻影态下同样进入恢复逻辑
  测试: ctrl_c_key_in_phantom_state_reaches_recovery
  假设 session 处于幻影态且服务端未广告 `session/hydrate`
  当 event loop 处理 Ctrl+C
  那么 run state 变为 Idle 且文案为 `status.phantom_turn_reset_by_user`

场景: 完全空闲时 Esc 键仍只是回到 composer 焦点
  测试: esc_key_when_idle_refocuses_composer_without_command
  假设 run state 为 Idle 且无 live turn
  当 event loop 处理 `KeyCode::Esc`
  那么 不返回命令且 run state 仍为 Idle

### Rule: esc-existing-paths-unchanged — 非幻影态的 Esc 语义不变
场景: 有 live turn 时 Esc 仍发 turn/interrupt
  测试: interrupt_command_targets_active_turn
  假设 session 有 live turn
  当 调用 `interrupt_command()`
  那么 返回 `InterruptTurn` 且 `turn_id` 为 live turn

场景: 完全空闲时 Esc 仍报无活动回合
  测试: interrupt_command_reports_when_no_turn_is_active
  假设 run state 为 Idle 且无 live turn
  当 调用 `interrupt_command()`
  那么 状态栏文案为 `status.no_active_turn_interrupt` 且不返回命令

场景: Blocked 等待时 Esc 不走幻影复位
  测试: esc_in_blocked_state_does_not_phantom_reset
  假设 run state 为 Blocked 且无 live turn
  当 调用 `interrupt_command()`
  那么 run state 仍为 Blocked

### Rule: exit-always-available — 退出不受幻影态影响
场景: /exit 在幻影态下仍请求退出
  测试: slash_exit_requests_exit_while_phantom_in_progress
  假设 session 处于幻影态
  当 通过 composer 派发 `/exit`
  那么 `exit_requested` 为 true

场景: 幻影态与探测/复位文案在两种语言包中都存在
  测试: phantom_esc_status_keys_exist_in_both_locales
  当 读取 `locales/en.yml` 与 `locales/zh.yml`
  那么 `status.phantom_turn_probing` 与 `status.phantom_turn_reset_by_user` 在两个文件中均有定义

## 排除范围

- 看门狗本身的 tick 逻辑与证据定义（task-stuck-run-state-watchdog）。
- Ctrl+Q 的按键路由（event loop 中已无条件返回 Quit，不改动）。
- 服务端返还未消费 steer（octos F4）。
