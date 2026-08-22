spec: task
name: "Global 运行时阶段 1:驾驶舱脚本(herdr 优先 / tmux 回退)"
tags: [olp, lifecycle, herdr, tmux, octoscode]
satisfies: [REQ-OLP-LIFE]
estimate: 1d
---

## 意图

外环能观测、能指导,但拉起运行时仍要 operator 手动开终端。本任务落地
LEP-002 阶段 1:`scripts/octos-global.sh` 驾驶舱脚本,封装
launch/inject/read/attach 四原语,把注入纪律与锁检查做成机制。零上游
依赖,octoscode 仓库即可完成;阶段 2(--headless)见
task-req-olp-life-headless-client。

## 已定决策

- 脚本子命令即四原语:`launch`、`inject <text>`、`read`、`attach`,外加
  `status` 与 `stop`;session 名固定 `octos-global`,`launch` 以仓库根为
  cwd 启动 octoscode。
- 后端抽象与优先序(2026-08-23 二次修订):**herdr 优先,tmux 回退**。
  herdr 的注入闸门(named-agent 名单 + 前台进程签名,均硬编码)已通过
  fork 补丁解决:Ti-Agent-OS/herdr feat/octoscode-agent(fc414dd8)把
  octoscode 加入 Agent 枚举并附 bundled 屏幕检测清单;实测端到端通过
  (自动识别 idle/working/blocked、`agent prompt` 注入 → composer 提交
  → turn 启动)。herdr 后端要求 herdr ≥ 该 fork 构建,探测不到能力时
  回退 tmux(`send-keys`/`capture-pane`);两后端四原语行为契约一致,
  都不可用时明确报错退出。
- `launch` 前置检查:目标 instance 的 `.octos-serve.lock` 被持有则拒绝
  启动并输出持有者 PID(经 lsof/fuser),绝不 kill 或抢锁;同名驾驶舱
  session 已存在同样拒绝。
- `inject` 纪律(机制化,不靠调用方自觉):注入前 `read` 当前画面,
  匹配到 approval/question 界面特征(边框标题 "Approval" / "Question"
  或其 i18n 对应)时拒绝注入并以特殊退出码返回;无论画面状态,拒绝注入
  单键审批字符(y/s/n 及其大写)作为完整输入。
- `stop` 是唯一的计划内关停路径(先 Esc/收尾再退出);脚本不含任何
  自动重启逻辑(REQ-OLP-LIFE-WATCH:非计划终止只告警,重启是人的决定)。
- 测试载体:`tests/global_runtime_contract.rs` 以 PATH 注入 stub 的
  herdr/tmux 假命令驱动脚本,断言原语行为;不依赖真实 TUI。

## 边界

### Allowed Changes
- scripts/octos-global.sh
- tests/global_runtime_contract.rs
- docs/OUTER_LOOP_PROTOCOL.md
- specs/**

### Forbidden
- 不改 src/**(阶段 1 零生产代码)。
- 脚本不得包含 kill/抢锁路径。
- 审批键注入在任何代码路径都不可达(测试钉死)。
- 不引入 tmux/herdr 之外的驾驶舱依赖。

## 排除范围

- 阶段 2 headless client(独立合约)。
- 远程(--remote/ssh)驾驶舱。
- 崩溃自动重启与保活守护。

## 完成条件

场景: 锁被持有时拒绝启动(critical)
  测试: olp_life_launch_refuses_held_lock
  假设 stub 环境令 serve 锁显示为被持有
  当 执行 octos-global.sh launch
  那么 脚本非零退出、输出持有者信息、未调用任何后端启动命令

场景: 无任何后端时明确报错
  测试: olp_life_no_backend_errors
  假设 PATH 中既无 herdr 也无 tmux
  当 执行 octos-global.sh launch
  那么 脚本非零退出且错误信息指明缺失的后端

场景: herdr 优先于 tmux
  测试: olp_life_backend_prefers_herdr
  假设 PATH 中 herdr 与 tmux stub 同时存在
  当 执行 launch 与 inject
  那么 调用记录显示走 herdr 原语且未触碰 tmux

场景: approval 画面阻断注入(critical)
  测试: olp_life_inject_blocked_on_approval
  假设 read 原语返回含 Approval 卡特征的画面
  当 执行 inject "继续"
  那么 注入被拒绝、以专用退出码返回、后端未收到按键

场景: 审批键永不注入
  测试: olp_life_approval_keys_never_injected
  假设 画面处于任意状态
  当 执行 inject "y"
  那么 注入被拒绝(单键审批字符黑名单)

场景: stop 是唯一的计划内关停路径
  测试: olp_life_stop_graceful_shutdown
  假设 stub 后端中有一个运行中的 octos-global session
  当 执行 octos-global.sh stop
  那么 后端收到收尾按键序列后 session 被结束,且脚本全文无自动重启调用

场景: 后端等价性
  测试: olp_life_backend_equivalence
  假设 同一条 prompt 分别在仅 herdr 与仅 tmux 的 stub 环境注入
  当 比较两次后端收到的最终输入
  那么 文本与回车行为一致
