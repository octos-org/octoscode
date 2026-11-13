spec: task
name: "运行时自动守卫：olp-runtime-watch 观测哨（#12a/#12b 独立切片）"
id: task-olp-runtime-watch
status: implemented
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
- 账本层的实现:本命令不自己解析 ledger,而是每个周期调用 `olp_runtime_watch_ledger` 的
  `LedgerCursor::poll()`(解析、作用域、轮转、隐私规则都在那个模块里)。记录里新增
  `ledger`(白名单的 poll 结果)与 `progress`;账本告警并入 `alerts` 并带
  `"source": "ledger"`。账本模块只在 Unix 上构建(文件身份用 dev/ino),其他平台该层报告
  `observation_unknown: ledger_unsupported_platform`。读入启动时已有的积压不算新活动。
- 进度层的实现:对 `--progress-file` 的每个文件取字节哈希(超过 16 MiB 的用长度与修改
  时间),连同有界探测得到的 Git HEAD 组成指纹。首次观察是基线,不算进展;指纹变化即重置
  进度龄;龄 ≥ `--stall-seconds` 时发 `stalled_no_progress`(`advisory: true`),并用
  `activity` 区分 `events_continuing`(只读活锁)与 `no_events`(静默)。没有给
  `--progress-file` 时不做停滞判断(只凭提交太稀疏)。文件缺失列在 `missing_files`。
- 转发到 hub:`scripts/olp-alert-forward.py` 读本命令的 NDJSON,把值得大外环知道的事经
  `olp-board-remote.py` 追加到 hub 的黑板:
  `> 单元(<U>)·负哨(#<任务>): <种类> turn=<turn> <细节>`。默认转发 agent_missing、
  backend_missing、watcher_missing、process_identity_changed、turn_error、
  stalled_no_progress;tool_failed 在正常工作中很常见,默认不转发(`--include` 打开)。同一
  件事在持续期间只转发一次,消失后再出现算新的一次;键稳定,排队重发不重复。只输出白名单
  字段,所有值先清洗成单行安全字符。转发器不判断、不处置。
- 接手说明(2026-11-13):进程层由车道 inner_octoscode_watch 在分支 fix/olp-runtime-watch
  上完成(5 个提交,未合并);经 operator 同意,在其之上 rebase 到当前 main 并补完账本层、
  进度层与 hub 转发。原分支保留未动。
- 完成/Herdr Done/idle/exit 只产生 awaiting_review/completion_unverified，即使有
  ACK 也不自接受。默认 stdout-only，不落盘（显式 own-state 目录除外）。

## Boundaries

### Allowed Changes
- src/cmd/olp_runtime_watch.rs、src/cmd/mod.rs（dispatch/route/CLI help）
- tests/olp_runtime_watch.rs、docs/OLP_OUTER_BOOT.md、本 spec
- scripts/olp-alert-forward.py、tests/olp_alert_forward.rs、docs/OLP_CLUSTER.md(hub 转发)

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

### Rule: ledger-scan — 目标 turn 过滤与轮转
Scenario: 目标 turn 工具失败被告警，旧/异 turn 记录被忽略；滚动后继续
  Test: olp_runtime_watch_target_turn_error_alerted_rotation_followed
  Given 真实形状 JSONL（event.session_id/event.turn_id 嵌套）
  When 目标 turn tool_completed success=false；轮转到同父目录后继
  Then 告警含 kind+身份；后继被增量跟踪；部分行保留；无 reasoning/preview 输出

### Rule: stall-detection — 事件持续但文件不变 => stalled_no_progress
Scenario: 重复读事件+哈希不变触发停滞；真实字节变更重置
  Test: olp_runtime_watch_stall_and_progress_reset
  Given progress-file 与持续事件流
  When 超过 stall_seconds 无字节/HEAD 变化
  Then stalled_no_progress（advisory 标注）；文件变更后重置

### Rule: never-accept — Done/ACK 也不自接受
Scenario: Done 无证据、或存在有效 ACK
  Test: herdr_whitelist_done_idle_unknown_and_mismatch_fixtures
  Test: terminated_agent_is_process_missing_never_acceptance
  Test: olp_runtime_watch_target_turn_error_alerted_rotation_followed
  When ledger 出现 Done/turn 完成/板上已有 ACK
  Then 仅 awaiting_review/completion_unverified，task_accepted 仍 false

### Rule: hub-forward — 告警转发到 hub 的黑板
Scenario: 告警转成 hub 黑板上的行,持续期间只报一次
  Test: alert_forward_reports_each_event_once_per_episode
  Given 一串守护记录:工具失败、turn 报错、连续三次停滞、恢复、再次停滞、agent 进程消失并持续
  When 交给转发器;以及带 `--include tool_failed` 只交前两条
  Then 板上依次是 turn_error、两次停滞(两个不同的持续期各一次)、agent_missing 各一行,没有工具失败;打开后工具失败有一行

Scenario: hub 不可达时告警排队,恢复后补上且不重复
  Test: alert_forward_queues_while_the_hub_is_unreachable
  Given hub 不可达,后端进程成了僵尸
  When 转发器处理两条相同的记录;hub 恢复后重发队列;转发器重启后再次看到同一件事
  Then 转发器不退出、板上先没有行;重发后恰好一行 backend_missing;重启后仍是一行

Scenario: 只转发白名单字段,坏行与未知种类不影响运行
  Test: alert_forward_only_emits_whitelisted_fields
  Given 混有非 JSON 行、非对象行、未知种类,以及在任务名、turn、序号里夹带换行与伪造判词的记录
  When 交给转发器
  Then 板上只多一行 turn_error,没有夹带的内容、没有新的编号条目、没有外环署名的行;未知的 `--only` 种类与缺配置都以退出码 2 结束
