---
kind: requirement
id: REQ-OLP-HEADLESS
title: "Global 运行时阶段 2:octoscode --headless client"
status: accepted
liveness: auto
tags: [olp, lifecycle, headless, octoscode]
---

## Problem

阶段 1 的驾驶舱脚本(REQ-OLP-LIFE)仍以 TUI 为载体:键盘模拟与画面
解析是桥,不是终态。session client-coupled 契约要求有一个 client 才能
打开 peer 会话——需要一个不渲染、不读键盘、纯协议职责的 client 形态。

## Requirements

[REQ-OLP-HEADLESS-MODE] octoscode MUST 提供 `--headless` 标志:复用既有
transport/store 协议栈,承担全部 client 职责(capabilities 握手、
session/open、消费 peer/staged 打开 peer 会话、事件泵),不初始化终端、
不渲染、不读键盘。

[REQ-OLP-HEADLESS-STEER-ONLY] headless 模式的指令入口 MUST 仅为 octos
侧 steer(REQ-OLP-CTRL);approval/question MUST 一律 park 并走
escalation,MUST NOT 存在任何自动应答路径。

[REQ-OLP-HEADLESS-MUTEX] headless 与 TUI MUST 沿用 serve 排他锁互斥;
MUST NOT 绕过锁或引入共享后端(另行提案)。

[REQ-OLP-HEADLESS-SHUTDOWN] SIGTERM/SIGINT MUST 走优雅关停(子进程
正常终止,退出码 0);backend 不可恢复时 MUST 以非零退出。

## Scenarios

Scenario: headless 打开 staged peer 会话
  Given octoscode --headless 连接 backend 且收到 peer/staged
  When 事件泵处理该通知
  Then peer 会话被打开,全程无终端渲染调用

Scenario: 审批只 park 不应答
  Given headless 会话收到 approval 请求
  When 事件泵处理它
  Then approval 保持未决且产生 escalation 记录

Scenario: 与 TUI 抢锁互斥
  Given headless 实例持有 serve 锁
  When 同 cwd 启动 TUI
  Then TUI 收到既有的 data-dir-locked 拒绝

## Dependencies

- REQ-OLP-CTRL(指令入口)、REQ-OLP-OBS(观测面)、REQ-OLP-LIFE(阶段 1
  先行,桥退役条件)。

## Source Trace

- proposal:LEP-002(operator 2026-08-22 拍板:--headless 标志、抢锁
  互斥先行)
- 从 REQ-OLP-LIFE 拆出:corpus 规则 requirement-multiple-specs(一个
  ready work unit 一份合约)。

## Open Questions

None.

## Next

Single exit: compile this requirement into a task contract with
`agent-spec requirements draft-specs`.
