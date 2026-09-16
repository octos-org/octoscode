spec: task
name: "stdio 重连保持 workspace scope"
inherits: project
tags: [tui, oup, reconnect, stdio, workspace]
estimate: 0.25d
---

## 意图

OctosCode 重启本地 OUP stdio 子进程时，必须先完成带原 workspace cwd 的
`session/open`，再放行自动续跑或用户请求。OUP 会并发处理 JSON-RPC 请求，因此仅靠
写入顺序不能保证 scope affinity 先建立。

## 已定决策

- 每次 session/open 都建立 scope barrier，包括本地选择和 stdio 重连；WebSocket 重连不重启服务端。
- 同一连接可复用主会话与 peer 会话。重连依次恢复所有已确认的 scope（最近目标最后），
  每个 scope 使用自己的 workspace root 和 resume cursor；所有确认到齐后才发出
  BackendRelaunched 并放行 deferred 命令，自动 peer open 不得覆盖主会话恢复记录。
- hydrate 没有 scope 授权语义：已确认会话只复用原完整 open 参数（包括 profile、cwd、
  topic、sandbox narrowing）；未知会话必须先由显式 scoped open 的成功响应建立记录，
  不因发出 hydrate 或 hydrate 失败提前写入可信恢复缓存。
- 重连期间 capability probe 与内部 `session/open` 可通过；其他 typed command 进入有界
  FIFO，收到匹配 session 的 `session/opened` 后依次放行。
- server-confirmed workspace root 仍优先用于后续 reopen；不改变 custom stdio command
  的 process cwd 或 instance-data-dir 语义。
- 不把客户端请求内容序列化后缓存；deferred queue 保留 typed command，放行时正常建立
  request id、pending correlation 与 resume cursor。
- 连接代次（P1-3）：transport 在 replacement stdio child 接入时、读取它的任何帧之前，先向
  store 投递 `ClientEvent::BackendConnectionEpoch`；store 给每次 `live_reply` latch 盖上
  当前代次戳，`BackendRelaunched` 只判死旧代次的 latch。新 child 自己在 scoped open 之前
  启动的续跑 turn（durable continuation 的 process-wide `turn/started`）保持存活。
- 已到 terminal 的 turn 再收到重放的 `turn/started` 不重新 latch、不重置 Working。
- 明确拒绝（P1-4）：server 对 barrier 的 `session/open` 返回 RPC error 且连接健康时，
  释放 barrier、原样透传错误事件、不断开，但在下次成功 open 前拒绝 session-bound 命令。
  turn/start 使用匹配 session/turn 的 session_open_rejected terminal，保留 prompt/error，
  不通过 request_cancelled 触发自动重试；全局 discovery、profile/auth 和 corrective open 可用。
  已排队的 corrective open 会重新建立 barrier，其后的 FIFO 只在确认 scope 后放行。reconnect 的
  reopen 目标回退到最近一次 server 确认过的 session（`session/opened` 经 barrier 确认；
  `/resume` hydrate 只可选择已确认记录），没有则回退 launch session。只有含糊情形（成功响应的 session 不匹配、
  `opened` 结构异常、超时）才走断开并重连的路径。
- 本地 tab 切换生成的 `session/open` 若无 cwd（P2-20），transport 在唯一出口用已捕获的
  server-confirmed workspace root 填充；未捕获时保持不带 cwd，显式 cwd 不被覆盖。
- hydration follow-up 队列溢出（P2-19）绝不淘汰 `session/open`，改淘汰最旧的非 open 项；
  队列全是 open 时丢弃新命令并在状态栏提示。

## 边界

### Allowed Changes
- src/transport.rs
- src/client_event.rs
- src/store.rs
- src/model.rs
- locales/**
- specs/task-stdio-reconnect-scope-barrier.spec

### Forbidden
- 不修改 OUP 服务端或 session affinity 算法。
- 不把所有 reconnect 都改成同步阻塞 UI。
- 不让 background continuation 在 matching session/opened 前越过 barrier。

## 完成条件

场景: peer 最后打开也不丢失主会话 scope
  测试: stdio_reconnect_reopens_master_and_peer_before_releasing_either_turn
  假设 原 stdio child 依次确认不同 cwd 的 master 和 peer，会话各有 resume cursor
  并且 master 有 restricted profile 与 sandbox narrowing，随后普通 hydrate 两个会话
  当 child 重启且两个会话都有后续 turn/start
  那么 replacement 依次确认 master 和 peer 的原 scope 与 cursor
  并且 每个 session/opened 都早于 BackendRelaunched 和两个 turn/start
  并且 master 的 profile 与完整 sandbox narrowing 未被 launch 默认值覆盖

场景: 未知 hydrate 不能伪造可信 scope
  测试: failed_unknown_hydrate_never_poisoned_confirmed_reopen_scopes
  假设 已有一个经 open 确认的会话
  当 未知会话发出 hydrate 且返回失败
  那么 既有 confirmed scopes 与 reconnect 目标保持不变

场景: 未知 resume 先建立 scope
  测试: unknown_resume_hydrate_waits_for_explicit_scoped_open
  当 用户 resume 一个尚未确认 scope 的会话
  那么 wire 先发送显式 session/open 且在匹配响应前不发送 hydrate
  并且 只有 open 成功后才记录该 scope 并继续 hydrate

场景: 多会话恢复中途拒绝 scope
  测试: rejected_mid_multiplex_reopen_fails_closed_until_corrective_open
  假设 恢复 master 成功而 peer 被拒绝，还有未发送的后续 scope
  那么 后续 scope 不自动发送，所有 deferred turn/start 被明确拒绝且不写入 wire
  并且 连接保持健康，corrective open 成功之前不放行新的 scoped turn

场景: replacement stdio child 先恢复 scope 再续跑
  测试: stdio_reconnect_waits_for_scoped_open_before_releasing_background_continuation
  假设 CLI 带显式 --cwd 且原 stdio child 已死亡
  当自动 continuation 的发送触发 replacement child 启动
  那么 replacement 的 session/open.cwd 保留原 workspace
  并且 turn/start 只在 matching session/opened 响应之后写入 child stdin

场景: 连接代次标记先于新 child 的任何事件
  测试: stdio_reconnect_waits_for_scoped_open_before_releasing_background_continuation
  假设 replacement stdio child 已接入
  当 store 依次接收 transport 事件
  那么 BackendConnectionEpoch 早于 startup turn/started
  并且 BackendRelaunched 晚于 session/opened

场景: replacement child 先于 scoped open 启动的续跑不被 relaunch reconcile 误杀
  测试: backend_relaunch_spares_the_turn_latched_after_the_new_connection_epoch
  假设 store 已收到 BackendConnectionEpoch，随后新 child 发出 turn/started(C)
  当 session/opened 与 BackendRelaunched 依次到达，之后 C 的 delta 与 terminal 到达
  那么 C 没有 "backend relaunched" 失败卡片、不被 tombstone
  并且 delta 正常累积、terminal 消费 live_reply、run state 收敛而非停在 Working

场景: 旧 child 代次的 latch 仍恰好失败一次
  测试: backend_relaunch_still_fails_the_turn_latched_before_the_connection_epoch
  假设 turn 在 BackendConnectionEpoch 之前 latch
  当 BackendRelaunched 到达
  那么 该 turn 被 tombstone 并只产生一张失败卡片

场景: 已 terminal 的 turn 重放 turn/started 不再重新 latch
  测试: replayed_turn_started_after_terminal_does_not_rebind_or_set_working
  假设 turn 已收到 terminal
  当 同一 turn 的 turn/started 被重放
  那么 live_reply 保持为空且 run state 不回到 Working

场景: 健康连接上被拒绝的 session/open 不拆连接、拒绝未确认 scope 的 turn
  测试: session_open_rpc_error_keeps_connection_but_requires_scope_before_turns
  假设 stdio child 已确认 launch session，用户随后打开一个 server 拒绝的 session
  当 server 对该 session/open 返回 RPC error
  那么 连接保持、barrier 释放、deferred 的 session/list 实际写入 child stdin
  并且 deferred 和后续 turn/start 都收到 typed terminal 且不写入 child stdin
  并且 corrective open 成功后才放行其后排队的 turn/start
  并且 没有可自动重试的 request_cancelled，reconnect 目标仍是已确认的 session

场景: scope 拒绝只结算对应 staged submit、不自动重试
  测试: rejected_scope_terminal_settles_only_its_submit_without_retrying_it
  假设 A 与 B 各有一个 staged submit in flight
  当 A 收到匹配 turn 的 session_open_rejected terminal
  那么 A gate 释放、prompt/error 保留且不重新入队，B gate 保持

场景: 被拒绝的 open 把 reopen 目标回退到最近确认的 session
  测试: rejected_session_open_reverts_reopen_target_to_last_confirmed_session
  假设 session A 已通过 barrier 得到 server 确认
  当 打开 B 的请求收到 RPC error
  那么 reopen 目标回到 A，pending 请求清空，队列中没有 cancel 或断开状态事件

场景: 含糊的 open 结果仍走断开重连
  测试: malformed_session_open_success_fails_deferred_commands_instead_of_wedging
  假设 session/open 的成功响应未包含期望的 session
  当 transport 解码该响应
  那么 barrier 与 deferred 队列被清空并断开、由 reconnect 重开记忆中的 session

场景: session/open 超时仍走断开重连
  测试: timed_out_session_open_fails_deferred_commands_instead_of_wedging
  假设 barrier 超过 SESSION_OPEN_RESPONSE_TIMEOUT 无响应
  当 超时检查运行
  那么 barrier 与 deferred 队列被清空并断开重连

场景: 无 cwd 的本地 session/open 用已捕获 workspace root 定界
  测试: local_session_open_without_cwd_is_scoped_to_the_captured_workspace_root
  假设 该 session 的 workspace_root 已从 session/opened 捕获
  当 store 生成 cwd 为空的 OpenSession
  那么 wire 请求携带该 root
  并且 未捕获 root 的 session 保持不带 cwd，显式 cwd 不被覆盖

场景: hydration 队列溢出不淘汰 session/open
  测试: hydration_queue_overflow_never_evicts_a_session_open
  假设 队列已满且 tab 切换把 OpenSession 推到队首
  当 再入队一个 staged turn/start
  那么 OpenSession 仍在队首、submit 排在其后、队列仍有界

场景: 队列全是 session/open 时丢弃新命令并提示
  测试: hydration_queue_full_of_opens_drops_the_incoming_command_and_reports_it
  假设 队列中 16 项全是 OpenSession
  当 再入队任意命令
  那么 该命令被丢弃、没有 open 被淘汰、状态栏显示队列已满
