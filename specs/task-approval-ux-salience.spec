spec: task
name: "授权到达可感知与卡片可读性"
inherits: project
tags: [tui, approval, ux, subagent]
estimate: 0.5d
---

## 意图

实测(2026-08-02 16:55–17:00,turn 019fc1ae):Spawn 出的子代理连跑 30+ 轮
bash 五分钟,主视图只有一行静止的 "Orchestrating… Spawn";随后授权请求以
无边框灰字形态无声出现——用户"半天没反应,突然弹出个授权,授权 UI 很不好"。
本任务解决三件事:授权/提问**到达时可感知**(终端响铃);授权卡**可读**
(边框 + 风险徽章,不再混入流水);子代理**等待可解释**(行内显示已运行
时长)。并用回归测试钉死"活动会话的可见授权必须把状态栏翻成 Waiting"。

## 已定决策

- 响铃走既有 pending-flag 模式:store 不做 I/O,新增
  `AppState.pending_decision_bell`,由事件循环在 `flush_pending_clipboard`
  同位置排空并写一次 `\x07`(BEL)。终端关铃则无害降级。
- 授权卡重绘 `push_inline_approval_card`:危险色顶/底边框 + 左栏 `│`,
  标题行含 `⚠` 与工具名,风险等级以危险色徽章内联;正文与动作键沿用既有
  `approval_modal_lines` / `approval_action_labels`,不改语义。
- 子代理耗时用客户端 first-seen 时钟:`AppState.task_first_seen`
  (task id → Instant,沿用 `PeerMeta.created` 先例),任务首次以
  pending/running 出现时记录;chip 行渲染 `标题 · 耗时`,靠活动回合的
  动画重绘节拍自然刷新。
- 不改 server 协议与授权载荷;不做桌面级通知(先响铃,不够再升级)。

## 边界

### Allowed Changes
- src/model.rs
- src/store.rs
- src/event_loop.rs
- src/app.rs
- src/app/transcript_build.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- store 层不得直接写终端(响铃必须经事件循环排空)。
- 不改变授权动作键位与审批语义(y/s/n/d 原样)。
- scrollback flush 的完整性不受影响。
- 不新增 crate 依赖。

## 排除范围

- OSC 9 / 桌面通知。
- 子代理全量输出进主转录(Tab peek 与 /ps 仍是钻取入口)。
- 活动折叠展示(独立合约)。

## 完成条件

场景: 授权到达武装响铃且只响一次
  测试: decision_arrival_arms_the_terminal_bell
  假设 一个可见授权到达活动会话
  当 store 应用该到达事件
  那么 pending_decision_bell 为真
  并且 排空后其他无关事件不再重新武装

场景: 授权卡渲染为带边框与风险徽章的卡片
  测试: approval_card_renders_bordered_with_risk_chip
  假设 一个 risk=high 的可见授权
  当 渲染转录
  那么 输出包含顶部与底部边框字符
  并且 风险等级以显式徽章文本出现
  并且 标题 "Approval Requested" 保留

场景: 运行中子代理行显示已运行时长
  测试: running_subagent_row_shows_elapsed_time
  假设 一个归属活动回合的运行中任务,其 first-seen 时钟为 272 秒前
  当 渲染 live tail
  那么 子代理行包含耗时文本(约 4m32s)

场景: 活动会话的可见授权把状态栏翻成 Waiting
  测试: spawn_originated_approval_flips_state_to_waiting
  假设 活动会话存在可见授权且回合处于进行中
  当 渲染状态栏
  那么 状态标签为 Waiting 而非 Working
