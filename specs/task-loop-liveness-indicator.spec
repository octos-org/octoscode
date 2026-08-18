spec: task
name: "Loop 运行态可感知"
inherits: project
tags: [tui, autonomy, loop, ux]
estimate: 0.5d
---

## 意图

实测(2026-08-03):`/loop` 建立自主循环后,界面只有一行静态
`↻ Loops: 1 active [self-paced 请你完成这本书]`——不动、无倒计时、无到期时间,
回合跑起来时该行还会被挤掉,状态栏也只有静态的 "1 active loop(s)"。用户无法
判断"它到底在不在 loop 里"。本任务用**已有的真实数据**
(`next_run_at_ms` / `expires_at_ms` / `loop/fired` 事件)让 loop 活起来:
慢速旋转的 ↻ 图标、下次触发倒计时、已触发轮次、到期剩余;状态栏常驻紧凑
chip;loop 触发的回合在活动组头部带 ↻ 归因前缀。

## 已定决策

- **不虚构进度百分比**: `UiLoopRecord` 无总轮次字段,self-paced 模式本身
  没有分母。以"第 N 轮 / 下次 / 剩余"三项真实信息代替百分比。
- 旋转动画: 独立于既有 `spinner_frame`(160ms/帧)的**慢速**帧函数
  `loop_spinner_frame()`,约 500ms 一帧,仅在存在活跃 loop 时使用——
  自主指示行是常驻行,快转会造成视觉噪音。
- 轮次计数: 客户端按会话累计 `loop/fired` 事件
  (`AppState.loop_fire_counts: HashMap<(SessionKey, loop_id), u32>`);
  server 不提供该计数,客户端计数在重启后归零属可接受的近似。
- 归因: `loop/fired` 到达时记录"该会话的下一个回合由 loop 触发"
  (`AppState.loop_attributed_turns: HashSet<(SessionKey, TurnId)>`,在回合
  开始时落定),活动组头部渲染 `↻ ` 前缀。
- 状态栏 chip: 现有冗长静态文案改为 `↻ loop 2m14s`(倒计时);无 next_run
  时回落为 `↻ loop`;暂停态沿用既有暂停文案。
- 所有时间显示复用既有 `format_elapsed_secs`;倒计时为负或缺失时不显示该段。

## 边界

### Allowed Changes
- src/app.rs
- src/app/render.rs
- src/app/transcript_build.rs
- src/app/tests.rs
- src/model.rs
- src/store.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不虚构或推算不存在的进度百分比。
- 不改变 loop 的创建/暂停/恢复协议与命令语义。
- 不改变既有 `spinner_frame` 的节拍(其他 UI 依赖它)。
- reserve==render 不变量不得破坏。

## 排除范围

- server 侧为 loop 增加 `max_iterations`(有分母才能做真进度条)。
- `/goal` 计划步骤完成度的进度显示。

## 完成条件

场景: 自主指示行显示倒计时轮次与到期
  测试: loop_row_shows_countdown_iteration_and_expiry
  假设 一个活跃 loop,next_run 在 134 秒后、expires 在 3 小时后、已触发 7 次
  当 渲染自主指示行
  那么 该行包含第 7 轮的轮次文本
  并且 包含下次触发倒计时 2m 14s
  并且 包含到期剩余时间

场景: loop 图标慢速旋转
  测试: loop_spinner_advances_slower_than_the_turn_spinner
  假设 loop 专用的慢速旋转帧函数
  当 比较其换帧周期与既有 turn spinner
  那么 loop 帧周期至少为 turn spinner 的 3 倍

场景: 状态栏常驻紧凑 loop chip
  测试: status_bar_shows_compact_loop_chip_with_countdown
  假设 一个活跃 loop 且 next_run 在 134 秒后
  当 渲染状态栏
  那么 输出包含 loop 字样与 2m 14s 倒计时
  并且 不再包含旧的冗长提示 "/loop pause to stop"

场景: loop 触发的回合带归因前缀
  测试: loop_triggered_turn_group_carries_attribution_prefix
  假设 一个会话收到 loop/fired 后开始新回合
  当 渲染该回合的活动组头部
  那么 头部包含 ↻ 归因前缀

场景: 非 loop 回合无归因前缀
  测试: manual_turn_group_has_no_attribution_prefix
  假设 一个由用户手动发起的回合
  当 渲染该回合的活动组头部
  那么 头部不包含 ↻ 归因前缀
