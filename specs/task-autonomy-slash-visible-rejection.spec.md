spec: task
name: "autonomy slash 拒绝可见化：拒绝理由必须进入 transcript 渲染"
inherits: project
tags: [tui, reducer, slash, autonomy, transcript]
estimate: 0.5d
---

## 意图

`dispatch_slash_command` 在派发前就清空了 composer，而各 `dispatch_*_command`
以「写 `state.status` + 返回 `None`」表示拒绝。状态栏与 turn 状态、审批模式、用户
chrome 共享，因此一次被拒绝的 `/goal archive` 在用户看来就是「回车没反应」：输入
消失了，transcript 里没有任何一行取代它。

本任务把 autonomy slash 的三个拒绝出口（`Ok(None)`、解析 `Err`、dispatcher 返回
`None`）统一为一条**在 transcript 中真实渲染**的路径，并让拒绝理由通过显式通道
传递，而不是靠比较状态栏字符串来推断。

不限于能力门：每一种 `/goal` 拒绝（无缓存 goal、非法状态迁移、readonly、解析错误）
以及 `/agents`、`/loop`、`/task`、`/turn`、`/thread` 的拒绝原本都同样不可见。

## 已定决策

<!-- lint-ack: external-io-error-strength — 全部为进程内 reducer/渲染断言，无外部 I/O -->

- **渲染路径用 `ActivityKind::Report`，不用 `Warning`。** `flow_activity_items`
  （`src/app.rs`）把 Warning 送进 agent-task flow，并按**活跃 turn id** 过滤：
  turn 运行时无 turn 归属的本地拒绝被整条排除；空闲时它落进 settled 分组，子项
  默认折叠（`src/app/transcript_build.rs`），只剩分组摘要。两种状态下理由都到不了
  屏幕。`flow_report_items` 两个过滤都不适用：report 有自己的全保真区块，不参与
  分组、不被折叠——这正是「客户端本地拒绝」需要的契约：它不是 agent 动作，也不属于
  任何 turn。
- **归属**：report 绑定用户输入时所在的 session（`with_session`），使其随该 session
  移动，切换 session 后不串台。`flow_report_items` 对**无归属**的 report 全局可见，
  那是全局 `/loop list` 需要的语义，对本场景是错的。
- **拒绝理由走显式通道** `AppState::pending_autonomy_rejection`，由
  `Store::reject_with_reason` 写入（同时仍写状态栏），由
  `Store::reject_autonomy_slash` `take()` 消费。不能用「派发前后状态栏是否相等」
  推断：同一条非法命令连续提交两次会写入**完全相同**的文本，按相等判断会被读成
  「dispatcher 没有给理由」，于是把已有的具体理由替换成通用文案。
- **派发前清空**该通道，使上一条命令记录的理由永远不会被归到下一条命令。
- 未记录理由的拒绝路径回退到 `status.command_unavailable`：不够具体，但绝不会张冠李戴。
- 共享能力门 `require_appui_method` / `require_appui_feature` /
  `require_mutating_appui_method` 统一改用 `reject_with_reason`，覆盖本 PR 的动因场景
  （旧服务端未广告 `session/goal/operator_transition`）。
- 断言必须落在**真实渲染输出**（`crate::app::debug_render_text`）上。仅断言
  activity 模型中存在一条记录，在修复前也能通过——正是本任务要消除的假阳性。

## 边界

### Allowed Changes
- src/store.rs
- src/model.rs
- specs/task-autonomy-slash-visible-rejection.spec.md

### Forbidden
- 不改变任何 dispatcher 的接受（成功）语义与其返回的 `AppUiCommand`。
- 不把被拒绝的命令写入命令历史（保持 fail-closed 的 `Rejected`）。
- 不改动 `flow_activity_items` / `flow_report_items` 的过滤规则本身（渲染器行为先于
  本 PR 存在；本任务只选择正确的既有通道）。
- 不用状态栏字符串相等性推断 dispatcher 是否给出了理由。

## 完成条件

### Rule: rejection-is-rendered — 拒绝理由必须出现在真实渲染的 transcript 中
场景: 空闲态下被能力门拒绝的 `/goal archive` 渲染出具体理由（critical）
  标签: critical
  测试: goal_archive_on_a_backend_without_the_rpc_pushes_a_visible_warning
  假设 服务端广告 goal runtime 但未广告 `session/goal/operator_transition`，且会话已有 active goal
  当 用户提交 `/goal archive --reason retiring stale goal`
  那么 不产生任何后端命令
  并且 activity 中最后一条为 `ActivityKind::Report`，标题为 `local slash command`
  并且 该条 status 含 `session/goal/operator_transition`
  并且 将状态栏置为无关文本后，`debug_render_text` 输出仍包含该理由

场景: turn 运行中被拒绝同样渲染（critical）
  标签: critical
  测试: goal_rejection_renders_in_the_transcript_during_an_active_turn
  假设 存在一个 live turn，服务端同样未广告 operator transition
  当 用户在 turn 运行期间提交 `/goal archive --reason ...`
  那么 将状态栏置为无关文本后，`debug_render_text` 输出仍包含该拒绝理由
  并且 该条不因活跃 turn id 过滤而被排除

### Rule: reason-is-explicit — 理由经显式通道传递，重复提交不退化
场景: 同一条非法命令连续提交两次，第二次保留具体理由（critical）
  标签: critical
  测试: repeated_goal_rejection_keeps_its_specific_reason
  假设 `/goal archive --reason`（缺少 reason 取值）会产生一个具体解析错误
  当 连续两次提交同一条命令
  那么 两次拒绝 report 的 status 完全相同
  并且 第二次不等于 `status.command_unavailable`

场景: 上一条命令的理由不被后一条复用
  测试: stale_rejection_reason_is_not_reused_by_a_later_command
  假设 一条 `/goal archive --reason` 已被拒绝并消费了其理由
  当 随后提交一条因自身原因被拒绝的 `/turn`
  那么 第二条拒绝的 status 与第一条不同
  并且 不包含第一条的解析错误文本
  并且 `pending_autonomy_rejection` 已被消费为 `None`
