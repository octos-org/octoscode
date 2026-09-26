spec: task
name: "运行时自动守卫：olp-runtime-watch 观测哨（#12a/#12b 独立切片）"
id: task-olp-runtime-watch
status: draft
tags: [olp, runtime, guard, observation]
satisfies: [REQ-OLP-RUNTIME-WATCH]
estimate: 2d
---

## Intent

`octoscode olp-runtime-watch` 是一个只观测的生产 Rust 命令：对给定 agent/backend/
monitor 进程、真实 UI ledger、任务自有源文件做有界探测，及时输出 NDJSON 告警，
供外环发现本轮出现的真实失败。它永不接受任务（`task_accepted:false` 恒定，无任何
置真路径）、永不写板/杀进程/重启/推送。正向 ACK 通道仍是既有 olp-watch-board.sh，
接受权威仍是外环独立复审。

## Decisions

- 精确 CLI 身份入参（agent/agent_pid/backend_pid/monitor_pid*/repo/ledger/session/
  turn_id/task*/progress_file*/stall_seconds/interval_ms/once）；启动时冻结已验证
  快照；正 PID/阈值校验，repo 为目录、ledger 为常规文件。
- 进程探测：只读有界 `ps`（pid+出生身份检测复用）；缺失/僵尸子进程 =>
  process_missing/watcher_missing；超时/解析失败/权限拒绝 => observation_unknown，
  绝不从超时推断死亡。
- ledger 只读目标 session+turn 的记录（event.session_id/event.turn_id 嵌套形状）；
  增量字节游标，部分 JSONL 行保留重试；同父目录 ledger-*.log 轮转后继（不跨目录、
  不 rglob、seq 去重）；无法明确后继 => observation_unknown/ledger_rotation_needed。
  不输出 reasoning/output_preview 等负载内容。
- tool_completed success=false、目标 turn error、权限/供应商拒绝 => 带 kind 与
  tool/turn 身份的可操作告警；失败不蕴含任务完成。
- 停滞与只读活锁区分：事件持续但 progress-file 字节哈希/Git HEAD 不变，过阈值发
  stalled_no_progress；真实字节变更/提交重置进度龄；文件移除显式表示；小只读任务
  可给保守 advisory（明确标注）。
- 完成/Herdr Done/idle/exit 只产生 awaiting_review/completion_unverified，即使有
  ACK 也不自接受。默认 stdout-only，不落盘（显式 own-state 目录除外）。

## Boundaries

### Allowed Changes
- src/cmd/olp_runtime_watch.rs、src/cmd/mod.rs（dispatch/route/CLI help）
- tests/olp_runtime_watch.rs、docs/OLP_OUTER_BOOT.md、本 spec

### Forbidden
- 不写板、不接受、不杀/重启 agent、不 push/merge；不改 Cargo 依赖、harvest 文件、
  回放夹具、既有断言、.claude/.octos 配置或其他 worktree。

## Completion Criteria

### Rule: identity-snapshot — 快照验证后冻结，task_accepted 恒 false
Scenario: 零 PID/坏路径被拒，正常快照不含敏感负载
  Test: cmd::olp_runtime_watch::tests::validates_and_freezes_snapshot_unaccepted
  Test: cmd::olp_runtime_watch::tests::rejects_zero_pid_and_missing_paths
  Test: empty_task_is_rejected_with_usage_error
  Test: zero_pid_is_rejected_with_usage_error
  Test: herdr_stub_real_envelope_surfaces_agent_status_only
  Given 合法进程/文件参数与一个目标 turn
  When 启动
  Then 身份 NDJSON 输出且 task_accepted:false；无置真代码路径

### Rule: process-probe — 有界 ps 探测，缺失/僵尸/未知三分
Scenario: 合法 stdout 但带任何 stderr => observation_unknown（绝不错判 alive/missing）
  Test: ps_boundary_matrix_through_cli
Scenario: 真实子进程终止 => process_missing；探测超时 => unknown 非死亡
  Test: terminated_agent_is_process_missing_never_acceptance
  Test: live_agent_alive_and_terminated_monitor_is_watcher_missing
  Test: real_ps_multi_heartbeat_no_identity_alert
  Test: identity_change_alert_lives_in_snapshot_and_repeats_per_role
  Test: hung_herdr_stub_yields_unknown_without_breaking_process_probes
  Test: herdr_whitelist_done_idle_unknown_and_mismatch_fixtures
  Test: ps_boundary_matrix_through_cli
  Test: cmd::olp_runtime_watch::tests::parse_ps_listing_accepts_real_shape_and_rejects_garbage
  Test: cmd::olp_runtime_watch::tests::herdr_envelope_parses_agent_status_exact_name
  Given 测试自有子进程与 monitor 进程
  When 子进程退出 / 单独 monitor 退出 / 探测超时注入
  Then 分别 process_missing、watcher_missing、observation_unknown；不杀不重启

### Rule: ledger-scan — 目标 turn 过滤与轮转（PENDING：#8 独立账本模块集成后实现与测试）
Scenario: 目标 turn 工具失败被告警，旧/异 turn 记录被忽略；滚动后继续
  Test: olp_runtime_watch_target_turn_error_alerted_rotation_followed
  Given 真实形状 JSONL（event.session_id/event.turn_id 嵌套）
  When 目标 turn tool_completed success=false；轮转到同父目录后继
  Then 告警含 kind+身份；后继被增量跟踪；部分行保留；无 reasoning/preview 输出

### Rule: stall-detection — 事件持续但文件不变 => stalled_no_progress（PENDING：文件哈希/Git HEAD 进度接入后实现与测试）
Scenario: 重复读事件+哈希不变触发停滞；真实字节变更重置
  Test: olp_runtime_watch_stall_and_progress_reset
  Given progress-file 与持续事件流
  When 超过 stall_seconds 无字节/HEAD 变化
  Then stalled_no_progress（advisory 标注）；文件变更后重置

### Rule: never-accept — Done/ACK 也不自接受
Scenario: Done 无证据、或存在有效 ACK
  Test: herdr_whitelist_done_idle_unknown_and_mismatch_fixtures
  Test: terminated_agent_is_process_missing_never_acceptance
  Pending: 板面有效 ACK 存在时也不自接受（待 ledger/ACK 观测集成，#8 后）
  When ledger 出现 Done/turn 完成/板上已有 ACK
  Then 仅 awaiting_review/completion_unverified，task_accepted 仍 false