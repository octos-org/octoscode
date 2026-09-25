spec: task
name: "Runtime guard 的账本读取模块（独立并行实现，仅 ledger 读取）"
id: task-olp-runtime-watch-ledger
status: implemented
tags: [olp, runtime-guard, ledger, observability]
satisfies: [REQ-OLP-EVO]
estimate: 0.5d
---

## Intent

#12 完整监控目标中的 **ledger 读取** 子模块：增量读取 runtime UI 的 JSONL ledger
（`{v,seq,event:{...}}`），把当前 turn 的 tool 失败/turn 错误/完成信号转成有限白名单
告警，供 guard 消费。本模块只做读取，不做进程探测/Herdr/CLI watch 循环/进度哈希，
不接受权威、不产生 task_accepted 裁定。

## Decisions

- 公共 API（guard 可直接消费）：
  - `pub struct LedgerCursor::new(ledger: PathBuf, session: String, turn: String) -> Self`
  - `pub fn poll(&mut self) -> LedgerPoll`
  - `LedgerPoll: Serialize + Debug`，字段 `path`、`byte_offset`、`initial_scan`、`last_seq`、
    `alerts`（有限，`Vec<LedgerAlert>`）、`turn_state`（`Option<LedgerTurnState>`）、
    `observation_unknown`（`Option<String>`）。全部白名单字段；无 reason 原文、无
    output_preview、无命令参数、无原始 JSON、无 secret。
  - `LedgerAlert: Serialize + Debug`：`kind`（`tool_failed`|`turn_error`）、`seq`、
    `session`、`turn`、`tool_name: Option<String>`、`tool_call_id: Option<String>`、
    `error_code: Option<String>`（固定枚举，非自由文本）。
- 作用域：严格同时匹配 `event.session_id == session && event.turn_id == turn`；foreign
  session/turn 一律忽略。首次 poll 允许扫描既有记录（`initial_scan=true`），历史读取不
  产生"新进展"；此后只读追加字节，不重扫全文件。
- 事件语义（仅观察）：
  - `kind == "tool_completed" && success == Some(false)` → `tool_failed`
  - `kind` 为真实失败类（`turn_error`、`error`、`task_failed`）→ `turn_error`
  - `kind` 为 `turn_completed`/`turn_cancelled`/`turn_aborted` → `turn_state` 变
    `CompletionUnverified`/`AwaitingReview`（Done/ACK **不**构成 accepted）
  - 未知 kind 忽略；读取事件不重置任何进度概念。
- 增量读取与预算：每 poll 有最大字节数与行数预算、单行字节上限；未完成 JSONL/UTF-8
  尾巴（无换行）保留到下次；只有完整换行行才解析。预算耗尽时状态保持可继续（下个
  poll 从断点继续），不 panic。
- 健壮性：完整行的坏 JSON/坏 UTF-8 → `observation_unknown`（一行一次，不无限吃同一
  坏行）；文件不可读/截短/替换（长度变小或 identity 变化 dev/ino）→ `observation_unknown`
  且 offset 保持可重试，不静默沿用旧 offset；文件恢复后继续。
- 轮转：只从 ledger 的直接父目录检查 `ledger-*.log` 后继（不 rglob、不跨 session 目录）。
  找到明确后继时切换（后继记录仍严格 scope 过滤 + seq 去重）；不确定时
  `observation_unknown` + `turn_state=LedgerRotationNeeded`，不把旧文件沉默当无进展。
  seq 去重：同 turn 内重复 seq 不重复告警；foreign seq 不推进当前 turn 的 last_seq。
- 模块不打印（caller 负责 NDJSON 输出）。

## Boundaries

### Allowed Changes
- src/cmd/olp_runtime_watch_ledger.rs
- src/cmd/mod.rs（仅一行 `pub mod olp_runtime_watch_ledger;`）
- specs/task-olp-runtime-watch-ledger.spec.md

### Forbidden
- 不碰其他 route/dispatch、其他 spec、harvest、fixtures、Cargo/权限/profile。
- 不写常驻 Python；不改 runtime-watch 独立树的任何源码。
- 本模块测试通过 ≠ 完整 guard 完成。

## Completion Criteria

### Rule: scoped-incremental-ledger-reading — 增量 scoped 读取

Scenario: 当前 turn 的 tool 失败可观察
  Test: olp_runtime_watch_ledger_module
  Given 临时 ledger 文件含当前 session+turn 的 `tool_completed success=false`
  When poll
  Then 产生 `tool_failed` 告警（白名单字段），不 panic

Scenario: foreign session/turn 不告警
  Test: olp_runtime_watch_ledger_module
  Given 记录属其他 session 或其他 turn
  When poll
  Then 无告警、无 turn_state

Scenario: seq 去重
  Test: olp_runtime_watch_ledger
  Given 同 seq 记录出现两次
  When poll
  Then 只告警一次

### Rule: partial-line-and-recovery — 半行/坏行与恢复

Scenario: 部分 UTF-/JSONL 尾巴跨 poll 后才触发
  Test: partial_utf8_tail_across_polls
  Given 行字节被分两次追加
  When 两次 poll
  Then 告警只在行完整后产生

Scenario: 完整坏行 unknown 且后续恢复
  Test: complete_bad_line_unknown_then_recovers
  Given 完整行是坏 JSON
  When poll → 追加好行 → poll
  Then 第一次 unknown（不 panic），第二次恢复告警

Scenario: 截短/替换/读失败 unknown
  Test: truncate_replace_and_read_failure_unknown
  Given 文件截短或被替换
  When poll
  Then observation_unknown，offset 可重试

### Rule: rotation — 同目录后继

Scenario: 同目录新 ledger 正常继续
  Test: sibling_successor_continues
  Given 同目录出现 `ledger-*.log` 后继且含当前 turn 记录
  When poll
  Then 切到后继继续读取

Scenario: 无后继但旧文件停更给 rotation-needed
  Test: rotation_needed_when_uncertain
  Given 旧文件无新增且无法明确后继
 When poll
  Then turn_state=RotationNeeded 或 observation_unknown，绝不静默