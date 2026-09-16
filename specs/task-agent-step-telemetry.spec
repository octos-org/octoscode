spec: task
name: "长回合显示真实 step/token/time"
inherits: project
tags: [tui, progress, ux, long-running]
estimate: 0.25d
---

## 意图

Octos 的 Codex-style 交互回合不再使用任意的固定迭代上限。客户端必须让用户
看见真实的 LLM step、累计活跃 token 和经过时间，同时避免这些高频遥测把
Activity 历史刷满。

## 已定决策

- 使用加法式 `progress/updated{kind:"agent_progress"}`；未知 kind 的旧客户端
  可安全忽略，新客户端沿用通用 progress reducer。
- 文本形态由服务端给出：`Step N · T tokens · XmYs · R reflections`。
- step 与 reflection checkpoint 都显示在状态栏，但不计为 Activity 行。
- 文件抖动、工具失败、重试等可操作事件仍按既有规则记录，不被本规则吞掉。

## 边界

### Allowed Changes
- src/store.rs
- specs/task-agent-step-telemetry.spec

### Forbidden
- 不新增协议版本或依赖。
- 不根据 token 推算费用。
- 不隐藏工具、文件变更或错误事件。

## 完成条件

场景: step/token/time 遥测只占状态栏
  测试: agent_step_token_time_telemetry_stays_in_status_line
  假设 一个活跃回合收到 step 状态以及 reflection checkpoint 状态
  当 reducer 应用这两个 `progress/updated` 事件
  那么状态栏显示服务端提供的 step、token、时间与 reflection 次数
  并且 Activity 列表不新增高频遥测行
  并且回合保持 running
