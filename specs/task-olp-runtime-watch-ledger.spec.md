spec: task
name: "Runtime guard 的账本读取模块（独立并行实现，仅 ledger 读取）"
id: task-olp-runtime-watch-ledger
status: implemented
tags: [olp, runtime-guard, ledger, observability]
satisfies: [REQ-OLP-EVO]
estimate: 0.5d
---

## Intent

octos #12 完整监控目标中的 **ledger 读取** 子模块：增量读取 runtime UI 的 JSONL ledger
（`{v,seq,event:{...}}`），把当前 turn 的 tool 失败/turn 错误/完成信号转成有限白名单
告警，供 guard 消费。本模块只做读取，不做进程探测/Herdr/CLI watch 循环/进度哈希，
不接受权威、不产生 task_accepted 裁定。CLI 接线/filehash-stall/完整 guard 由 guard
owner 在其独立树完成，不属于本模块。

## Decisions

- 公共 API（guard 可直接消费）：
  - `pub struct LedgerCursor::new(ledger: PathBuf, session: String, turn: String) -> Self`
  - `pub fn poll(&mut self) -> LedgerPoll`
  - `LedgerPoll: Serialize + Debug`，字段 `path`、`byte_offset`、`initial_scan`、`last_seq`、
    `alerts`（有限，`Vec<LedgerAlert>`）、`turn_state`（`Option<LedgerTurnStateView>`）、
    `observation_unknown`（`Option<String>`）。全部白名单字段；无 reason 原文、无
    output_preview、无命令参数、无原始 JSON、无 secret。
  - `LedgerAlert: Serialize + Debug`：`kind`（`tool_failed`|`turn_error`）、`seq`、
    `session`、`turn`、`tool_name: Option<String>`、`tool_call_id: Option<String>`、
    `error_code: Option<LedgerErrorCode>`（固定枚举，非自由文本）。
- 作用域：严格同时匹配 `event.session_id == session && event.turn_id == turn`；foreign
  session/turn 与**合法无 turn_id** 的通知一律忽略（不是坏 JSON）。首次 poll 允许扫描
  既有记录，但积压未消费完期间 `initial_scan=true`（历史读入不是新进展）；消费完后的
  真实 append 才以 `initial_scan=false` 报告；首次读失败不消耗 first。
- 事件语义（仅观察）：`tool_completed success=false` → `tool_failed`；`turn_error`/
  `error`/`task_failed` → `turn_error`（固定错误码，绝不携带原 message/reason/
  reasoning/output_preview/secret）；`turn_completed` → `CompletionUnverified`、
  `turn_cancelled`/`turn_aborted` → `AwaitingReview`——Done/ACK **不**构成采认，普通
  idle 不抹终态；未知 kind 合法忽略（seq 水位仍推进），不当进度。
- 增量与预算：字节 cursor 只前进；`partial` 上限 = `MAX_BYTES_PER_POLL +
  MAX_LINE_BYTES`（持续追加压力下内存有固定界限）；每 poll 字节/行/告警预算，耗尽时
  保留断点下 poll 继续；行解析用已消费索引 + 一次 drain，无逐行整 Vec 复制。
- 超长行：未终止与换行分隔的超长行都限长；超限进入 **discard-until-newline** 状态，
  期间任何"恰好合法 JSON"的尾巴都不是事件；换行后新合法行正常恢复，固定错误码，
  不输出原始内容。
- 截短/替换：截短 → 安全重置 offset/partial，给一拍 `ledger_truncated` unknown 后
  恢复读取；目标 scope watermark 保留（旧 seq 重写不重复告警）。文件身份用**实际打开
  fd** 的 metadata（dev/ino）核对，路径 meta 与 fd 不符按替换处理，不静默读。
- 轮转：只从直接父目录解析 `ledger-<timestamp>-<pid>.log` 文件名（inode 不参与排序），
  选当前之后的明确后继并切换（watermark 跨正常轮转保留去重）；同 ts 不同 pid 候选 →
  `RotationNeeded`；目录读失败 → unknown；**普通无后继是正常 idle**，不永久遮蔽 stall。
- seq 去重：单调水位（同 seq 或更小=已处理），仅目标 scope 抬水位，foreign seq 不推进。
- 模块不打印（caller 负责 NDJSON）。

## Boundaries

### Allowed Changes
- src/cmd/olp_runtime_watch_ledger.rs
- src/cmd/mod.rs（仅一行 `pub mod olp_runtime_watch_ledger;`）
- specs/task-olp-runtime-watch-ledger.spec.md

### Forbidden
- 不碰其他 route/dispatch、其他 spec、harvest、fixtures、Cargo/权限/profile。
- 不写常驻 Python；不改 runtime-watch 独立树的任何源码。
- 模块完成 ≠ CLI/filehash-stall/完整 guard 完成（guard owner 接线）。

## Completion Criteria

### Rule: scoped-incremental-ledger-reading — 增量 scoped 读取

Scenario: 当前 turn 的 tool 失败可观察
  Test: current_turn_tool_failure_is_observable
  Given 临时 ledger 文件含当前 session+turn 的 `tool_completed success=false`
  When poll
  Then 产生 `tool_failed` 告警（白名单字段），不 panic

Scenario: foreign session/turn 与缺 turn_id 的合法通知不告警
  Test: foreign_session_or_turn_no_alerts
  Given 记录属其他 session、其他 turn，或合法但无 turn_id
  When poll
  Then 无告警，foreign seq 不抬水位

Scenario: seq 去重
  Test: duplicate_seq_deduped_by_monotonic_watermark
  Given 同 seq 记录出现两次
  When poll
  Then 只告警一次

### Rule: partial-line-and-recovery — 半行/坏行与恢复

Scenario: 多字节 UTF-8 字符中间切分跨 poll 后才触发
  Test: partial_utf8_tail_across_polls
  Given 中文 payload 在一个多字节字符中间切分，剩余字节以 append 续写
  When 两次 poll
  Then 告警只在行完整后产生

Scenario: 完整坏行 unknown 且后续恢复
  Test: complete_bad_line_unknown_then_recovers
  Given 完整行是坏 JSON
  When poll → 追加好行 → poll
  Then 第一次 unknown（不 panic），第二次恢复告警

Scenario: 截短/替换/读失败 unknown
  Test: truncate_replace_and_read_failure_unknown
  Given 文件截短、被替换或不存在
  When poll
  Then observation_unknown，offset 可重试

### Rule: rotation — 同目录后继

Scenario: 同目录 `ledger-<ts>-<pid>.log` 后继正常继续
  Test: sibling_successor_continues
  Given 同目录出现合法命名后继且含当前 turn 记录
  When 多次 poll
  Then 切到后继继续读取，seq 去重不重发

Scenario: 普通无后继是正常 idle（不误报 rotation）
  Test: terminal_states_persist_across_idle_and_done_means_nothing
  Given 旧文件无新增、目录可读且无合法后继
  When poll
  Then 无 observation_unknown、终态不被抹——静默不被当作异常，也不当作进展

### Rule: history-and-budget — 历史标记与预算（#9）

Scenario: 1001 条失败跨 poll 无丢失，积压期间 initial_scan=true
  Test: thousand_one_failures_across_polls_and_initial_scan_semantics
  Given 1001 条当前 turn tool 失败已存在（超单 poll 告警预算）
  When 反复 poll 至消化完，再真实 append 一条
  Then 总告警 1001+1 无丢失/重复；积压期间 initial_scan=true，append 后 false；partial 有固定上限

Scenario: 初始积压跨多个 poll 始终标历史
  Test: initial_backlog_spans_polls_marks_history
  Given 2100 条积压（必然跨 ≥3 个 poll）
  When 反复 poll
  Then 积压未消费完期间 initial_scan 恒 true

### Rule: oversize-recovery — 超长行丢弃与恢复（#9）

Scenario: 超长行丢弃后换行恢复，内部尾巴不当事件
  Test: oversize_line_discards_until_newline_then_recovers
  Given 未终止超长行（>1MiB）后 append 一段含合法 JSON 形状的尾巴+换行+新合法行
  When 分段 poll
  Then 超长行内部"合法"seq 不产生告警；换行后新合法行恢复告警，错误码固定

Scenario: 截短后一拍 unknown、恢复后旧 seq 不重复告警
  Test: truncate_resets_once_and_dedups_old_seqs
  Given 已消费 seq=1,2 后文件截短为仅 seq=1，再 append seq=2（重复）与 seq=3（新）
  When poll 序列
  Then 截短当拍 ledger_truncated unknown；恢复后 seq=1/2 不重复告警，仅 seq=3 一次（watermark 保留）

### Rule: privacy-and-terminal — 隐私与终态（#9）

Scenario: Serialize 输出不泄漏 quota/permission payload
  Test: serialize_output_never_leaks_error_payload
  Given turn_error/error 行含 quota message、permission reason、reasoning/output_preview/api_key 哨兵
  When poll 并序列化为 JSON
  Then 输出含固定 turn_failed/turn_error 码，绝不含任何哨兵原文（非仅 Debug 检查）

Scenario: 终态观察持久、Done/ACK 无效、未知 kind 忽略
  Test: terminal_states_persist_across_idle_and_done_means_nothing
  Given turn_completed 后 turn_cancelled，随后 ack_recorded（未知）与再 turn_completed
  When poll 序列含普通 idle
  Then 终态保持 AwaitingReview 不被 idle 抹掉；未知 kind 无告警且推进水位；Done/ACK 不产生任何采认