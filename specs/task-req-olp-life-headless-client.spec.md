spec: task
name: "Global 运行时阶段 2:octoscode --headless client"
tags: [olp, lifecycle, headless, octoscode]
depends: [task-req-olp-ctrl-steer, task-req-olp-obs-cli, task-req-olp-life-cockpit-script]
satisfies: [REQ-OLP-HEADLESS]
estimate: 2d
---

## 意图

阶段 1 的驾驶舱脚本仍以 TUI 为载体(键盘模拟、画面解析)。本任务落地
LEP-002 阶段 2:octoscode 增加 `--headless` client 模式——承担全部
client 协议职责但不渲染不读键盘,指令入口只走 steer(octos 侧)、观测
只走 OBS 的 --json/事件流。落地后 tmux/herdr 桥退役为调试手段。
operator 已拍板:`--headless` 标志复用现有协议栈;与 TUI 抢锁互斥。

## 已定决策

- CLI 新增 `--headless` 标志:复用既有 transport/store 全栈——
  capabilities 握手、session/open、`peer/staged` 消费并后台打开 peer
  会话、事件泵、durable 通知处理——但不初始化终端(无 raw mode、无
  splash、无渲染循环、无键盘读取)。
- headless 进程的生命周期:前台常驻,SIGTERM/SIGINT 走既有优雅关停;
  退出码非零当且仅当 backend 不可恢复(与 TUI 的 reconnect 语义一致)。
- 指令入口:headless 模式不读 stdin;唯一指令路径是 octos 侧 steer
  (依赖 task-req-olp-ctrl-steer)。审批在 headless 下永远 park 并走
  escalation 通知(依赖同合约),无任何自动应答。
- 与 TUI 互斥:沿用 serve 排他锁语义,不做共享后端(operator 拍板;
  WS 多客户端另行提案)。
- 日志:headless 把原 TUI 状态栏级信息写 stderr(结构化前缀),供
  驾驶舱脚本或 systemd 收集。

## 边界

### Allowed Changes
- src/main.rs
- src/cli.rs
- src/event_loop.rs
- src/transport.rs
- src/lib.rs
- tests/**
- specs/**

### Forbidden
- 不改 TUI 模式的任何行为(--headless 缺省关闭,零回归)。
- headless 不得应答 approval/question(park + escalation 是唯一路径)。
- 不绕过 serve 排他锁。
- 不新增网络端点。

## 排除范围

- 共享后端(TUI 与 headless 同时连一个 serve)。
- headless 的守护化(systemd unit 等部署形态)。
- steer/observability 本体(各自合约)。

## 完成条件

场景: headless 打开 peer 会话(critical)
  测试: olp_headless_opens_staged_peer
  假设 octoscode --headless 连接 mock backend 且收到 peer/staged 通知
  当 事件泵处理该通知
  那么 peer 会话被打开并开始事件流,无任何终端渲染调用

场景: headless 不读键盘不初始化终端
  测试: olp_headless_never_touches_terminal
  假设 以 --headless 启动且 stdin 为已关闭的管道
  当 运行一个完整事件循环周期
  那么 无 raw-mode/终端探测调用且进程不因 stdin 关闭退出

场景: 审批在 headless 下 park 并升级
  测试: olp_headless_approval_parks_and_escalates
  假设 headless 会话收到一个 approval 请求
  当 事件泵处理它
  那么 approval 保持未决且 escalation 记录产生,无自动应答

场景: TUI 与 headless 互斥
  测试: olp_headless_mutex_with_tui
  假设 一个 headless 实例持有 serve 锁
  当 同 cwd 启动 TUI 实例
  那么 后者收到锁拒绝(复用既有 data-dir-locked 提示)

场景: 优雅关停
  测试: olp_headless_sigterm_graceful
  假设 headless 实例正常运行
  当 收到 SIGTERM
  那么 backend 子进程被正常终止且退出码为 0
