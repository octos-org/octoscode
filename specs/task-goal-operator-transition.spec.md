spec: task
name: "/goal archive|reopen：在线 operator_transition 与暂存意图的授权边界"
inherits: project
tags: [tui, reducer, goal, autonomy, protocol]
estimate: 1d
---

## 意图

后端在 `coding.goal_runtime.v1` 下与 `session/goal/{get,set,clear}` 并列暴露了
operator 专用的 `session/goal/operator_transition`，octoscode 此前没有客户端。本任务
把它接成 `/goal archive` 与 `/goal reopen` 两个子命令：这是**在线**归档/重开路径，
直接改运行中 orchestrator 的目标记录，归档时同时退掉排队中的 `GoalContinue` 续跑，
补上"目标一直 active 却无法在会话内退役"的治理缺口（离线的 `octos goal archive`
会被 `serve` 的实时缓存回写覆盖）。

同时收敛一个更关键的安全性质：archive 在固定后端版本上是**终态**，误伤不可逆。一个
`/goal archive` 暂存的意图只被它自己发出的那一次 `session/goal/get` 授权，任何别的
结局（刷新失败、目标被替换、被后续指令取代、状态重放）都必须让它作废并告知操作者，
而只读的 `/goal` 查询永远不得触发变更。

## 已定决策

<!-- lint-ack: external-io-error-strength — 本合约全部断言为进程内 reducer/解码纯函数，错误路径即协议错误帧与能力门控，不涉及外部 I/O -->

- 方法常量 `APPUI_METHOD_SESSION_GOAL_OPERATOR_TRANSITION = "session/goal/operator_transition"`，
  经 `require_mutating_appui_method` 门控；老服务端未广告即不可用，不本地伪造。
- 结果类型是独立的 `SessionGoalOperatorTransitionResult`，**不复用** `SessionGoalSetResult`：
  后端返回 `{session_id, profile_id, goal, transition_actor: "operator"}` 且**没有** `ok` 字段，
  用 set 的结果类型会把"缺席"解码成 `ok: false`（即"被拒绝"）。
- 两步刷新是强制的，不是保守：后端把 `goal_id` 当作身份断言，会话的活跃目标不是同一个
  时直接拒绝。所以 `start_goal_transition` 先发 `session/goal/get`，在 `GoalGet` 响应上
  用**新鲜**的 `goal_id` 发起 follow-up。
- 状态守卫按动作分开，不用一张共享白名单：`SessionGoalOperatorAction::allows_from` 镜像后端
  守卫——archive 除 `archived` 外皆可（含 `complete`，而 set 路径永不可动 `complete`），
  reopen 仅限 `blocked|paused|budget_limited`。后端仍是最终权威，客户端只是免掉一次往返。
- `PendingGoalTransition` 携带 `staged_goal_id`（授权它的那个目标）与 `verb`（作废文案用）；
  `follow_up` 是 `PendingGoalFollowUp` 枚举，operator 变体不携带 set 路径才有意义的字段。
- **一次授权、一次结局**：`pending_goal_transition` 只能被它自己那次刷新的**一个**结局解决。
  - `GoalGet` 返回的 `goal_id` 与 `staged_goal_id` 不一致 → 作废（`status.goal_transition_target_changed`）。
  - 任意 `AppUiEvent::Error`（刷新失败、`request_cancelled`、重连拆链）→ 作废并推 Warning
    活动行（`status.goal_transition_abandoned`）。
  - `/goal <objective>`（Set）与 `/goal clear` 在**派发时**取代它，推 Warning 活动行
    （`status.goal_transition_superseded`）。Set 必须在派发时取代：`session/goal/set` 是
    原地更新、`goal_id` 不变（只有替换 `complete` 记录才铸新 id），所以两侧身份断言都会通过，
    刚被重新定义的目标会被不可逆归档。Clear 同理——刷新响应先到，会赶在 clear 之前发出 RPC。
  - `AppUiEvent::Snapshot` 全量重放（重连/relaunch/开关会话）**不**把它保留过去，并推 Warning
    活动行（`status.goal_transition_dropped_by_replay`）；保留会让重放后的刷新消费一个针对
    重放前世界暂存的意图，静默丢弃则让不可逆动作无声消失。
- 只读语义：`/goal`（Show）既不暂存也不取代任何意图；没有被授权的 `GoalGet` 一律不返回命令。
- `reason` 只在动词单独出现或后接 `--reason` 时绑定，不做自由尾随取词——`/goal archive the
  2025 logs` 是一个合法的目标描述，读成"归档，理由：the 2025 logs"会静默退役一个活跃目标。
  与既有 `--budget` 写法一致。
- 默认 `reason` 是 locale 无关常量 `crate::model::GOAL_DEFAULT_ARCHIVE_REASON` /
  `GOAL_DEFAULT_REOPEN_REASON`，**不**走语言包：它写进后端持久化的目标账本，由其他 locale
  的操作者与 agent 读取，随客户端语言变化会让同一动作在共享记录里读起来不同。
- 新增用户可见文案同时落 `locales/en.yml` 与 `locales/zh.yml`。
- `OperatorTransitionSessionGoal` 登记进 `docs/ARCHITECTURE.md` 的命令→方法映射表。

## 边界

### Allowed Changes
- src/store.rs
- src/model.rs
- src/autonomy.rs
- src/client_event.rs
- src/transport.rs
- locales/en.yml
- locales/zh.yml
- docs/ARCHITECTURE.md
- specs/task-goal-operator-transition.spec.md

### Forbidden
- 不改 `/goal pause|resume|stop` 的 set-path 状态白名单（`complete` 仍不可由 set 路径转换）。
- 不用 `SessionGoalSetResult` 解码 operator_transition 的响应。
- 不把缓存镜像里的 `goal_id` 直接发给后端（必须来自刷新响应）。
- 不让 `/goal`（Show）产生任何变更命令。
- 不把默认 `reason` 放进 `locales/*.yml`。
- 不新增 crate 依赖。

## 完成条件

### Rule: fresh-identity — 身份来自刷新，不来自缓存
场景: archive 用刷新回来的 goal_id 发起 operator_transition（critical）
  标签: critical
  测试: goal_archive_dispatches_operator_transition_with_fresh_goal_id
  假设 缓存镜像里有一个 `active` 目标，服务端广告 `session/goal/operator_transition`
  当 `/goal archive` 派发后 `session/goal/get` 返回一个 goal_id 不同于缓存的记录
  那么 不返回 `OperatorTransitionSessionGoal`
  并且 暂存意图被清空

场景: operator_transition 结果更新目标镜像（无 ok 字段）
  测试: goal_operator_transition_result_updates_the_goal_mirror
  当 `AutonomyResult::GoalOperatorTransition` 携带 `{goal, transition_actor: "operator"}` 到达
  那么 `SessionGoalOperatorTransitionResult` 解码后目标镜像替换为该记录且归因为 `operator`
  并且 不因缺少 `ok` 字段而被当作拒绝

### Rule: per-action-guard — 状态守卫按动作分开
场景: archive 可从 complete 发起，而 pause 不可（critical）
  标签: critical
  测试: goal_archive_is_allowed_from_complete_where_pause_is_not
  假设 缓存镜像里是一个 `complete` 目标
  当 分别派发 `/goal archive` 与 `/goal pause`
  那么 `SessionGoalOperatorAction::allows_from` 放行 archive，返回 `GetSessionGoal` 并暂存 operator 意图
  并且 pause 不返回命令且状态栏写明 `complete` 不可转换

场景: reopen 拒绝一个已经 active 的目标（错误路径）
  测试: goal_reopen_refuses_an_already_active_goal
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  假设 缓存镜像里是一个 `active` 目标
  当 派发 `/goal reopen`
  那么 不返回命令且不暂存意图

场景: 刷新期间目标已变成 archived 时中止（错误路径）
  测试: goal_archive_aborts_when_the_fresh_record_is_already_archived
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  假设 `/goal archive` 已按 `active` 缓存通过派发守卫
  当 刷新返回的记录状态为 `archived`
  那么 不返回 `OperatorTransitionSessionGoal`
  并且 状态栏写明该状态不可转换

### Rule: one-authorization — 一次授权只解决一次，只读查询不变更
场景: 刷新失败后，后续只读查询保持只读（critical）
  标签: critical
  测试: a_failed_refresh_abandons_the_intent_so_a_later_query_stays_read_only
  假设 `/goal archive` 暂存意图后其刷新以 `goal_unavailable` 失败
  当 稍后一次普通 `/goal` 对**同一个**目标返回 `GoalGet`
  那么 不返回任何命令
  并且 暂存意图在错误到达时即已清空

场景: 作废的意图不得挂到替换后的新目标上（critical）
  标签: critical
  测试: abandoned_archive_intent_is_not_replayed_against_a_replacement_goal
  假设 `/goal archive` 的刷新失败，随后一个新目标 `new-goal` 取代了原目标
  当 一次只读 `/goal` 查询返回 `new-goal`
  那么 不返回 `OperatorTransitionSessionGoal`

场景: 重新定义目标在派发时取代暂存的 archive（critical）
  标签: critical
  测试: redefining_the_goal_supersedes_a_staged_archive
  假设 `/goal archive` 已暂存意图并发出刷新
  当 操作者接着派发 `/goal a different objective`
  那么 暂存意图在 `SetSessionGoal` 派发时即被清空
  并且 随后到达的、goal_id 未变的 `GoalGet` 不返回任何命令

场景: 取代必须告知操作者，不得静默（错误路径）
  测试: superseding_a_staged_archive_tells_the_operator
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  假设 `/goal archive` 已暂存意图
  当 操作者接着派发 `/goal a different objective`
  那么 活动流新增一条包含 `superseded` 的 Warning 行

场景: /goal clear 在派发时取代暂存的 archive（错误路径）
  测试: clearing_the_goal_supersedes_a_staged_archive_at_dispatch
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  假设 `/goal archive` 已暂存意图并发出刷新
  当 操作者接着派发 `/goal clear`
  那么 暂存意图在 `ClearSessionGoal` 派发时即被清空
  并且 随后到达的 `GoalGet` 不返回任何命令

场景: 快照重放丢弃暂存意图并出声（错误路径）
  测试: a_snapshot_replay_abandons_a_staged_archive_loudly
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  假设 `/goal archive` 已暂存意图并发出刷新
  当 `AppUiEvent::Snapshot` 全量重放到达
  那么 `pending_goal_transition` 为空
  并且 活动流新增一条包含 `replayed` 的 Warning 行
  并且 重放后到达的 `GoalGet` 不返回任何命令

### Rule: verb-binding — 动词绑定不劫持目标描述
场景: 裸动词解析为 archive / reopen
  测试: goal_operator_verbs_parse_bare
  当 输入 `/goal archive` 与 `/goal reopen`
  那么 分别解析为 `GoalCommand::Archive { reason: None }` 与 `Reopen { reason: None }`

场景: --reason 标志承载理由
  测试: goal_operator_verbs_take_reason_flag
  当 输入 `/goal archive --reason 迁移到新目标`
  那么 解析为 `Archive { reason: Some("迁移到新目标") }`

场景: 以动词开头的目标描述不是动词（critical，错误路径）
  标签: critical
  测试: goal_objective_starting_with_operator_verb_is_not_the_verb
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  当 输入 `/goal archive the 2025 logs`
  那么 解析为 `GoalCommand::Set`，objective 为 `archive the 2025 logs`
  并且 不解析为 `Archive`

场景: --reason 后为空报错（错误路径）
  测试: goal_operator_verb_empty_reason_errors
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  当 输入 `/goal archive --reason` 且其后无内容
  那么 解析失败并给出用法提示

### Rule: durable-ledger — 写进共享账本的值不随语言变化
场景: 未给 --reason 时发送 locale 无关的默认值
  测试: goal_archive_without_reason_flag_sends_a_default
  当 `/goal archive` 未带 `--reason` 且刷新返回可归档记录
  那么 `OperatorTransitionSessionGoal` 的 `reason` 等于 `GOAL_DEFAULT_ARCHIVE_REASON`

场景: 默认理由不得进入语言包（错误路径）
  测试: default_operator_reasons_are_not_localized
  层级: 单元
  替身: 无（进程内 reducer/解析器纯函数，事件与协议帧由测试直接构造）
  当 读取 `locales/en.yml` 与 `locales/zh.yml`
  那么 两个文件都不定义 `goal_default_archive_reason` 与 `goal_default_reopen_reason`
  并且 两个常量的值分别为 `archived by operator from octoscode` 与 `reopened by operator from octoscode`

## 排除范围

- 后端 `operator_transition` 的实现与调度器 fencing（octos 侧，已完成）。
- 离线 `octos goal archive` CLI 的缓存回写行为。
- `/goal` 的菜单入口与快捷键。
- 归档目标的列表/检索界面。
