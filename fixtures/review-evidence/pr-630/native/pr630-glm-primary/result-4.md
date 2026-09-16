---
slug: pr630-glm-primary
outcome: completed
updated_unix: 1788922427
turn: 4
---

补充定点任务完成：

**cross-glm.md 新增“5. 外层新增反例裁决”小节**（turn 3，追加不覆盖；independent 文件未动；result.md 已更新）。

四项裁决摘要（动态证据均引自外层 adversarial.log 并注明非我方运行，静态复核为我方独立读码）：

| 反例 | 裁决 | 核心依据 |
|---|---|---|
| 1. 静默 child 永久抑制重试 | **采纳（新增回归·中，fail-to-fix）** | `stdio_child_startup_pending`（2474-2479）只看首帧；hello grace 超时路径（2494-2508）不清 `stdio_child_served_frame` → 2527 永久 return，attempts 停 1。PR 注释（2519-2520）自称要修的“startup grace 过期”场景的静默变体未修复。正常出帧 child 不触发 |
| 2. 排队成功被 retain 丢弃 | **采纳（新增回归·中）** | `next_event` 3133 先超时检查后 3137-3145 才轮询；retain（2540-2542）删 pending id 后，成功帧 `remove` 落空（3809-3812）静默丢弃。窗口需消费停摆 ≥5s，窄但真实；**这是 PR 新代码引入，基线无此问题** |
| 3. 测试证明力高估 | **采纳，修订双方结论** | 测试 1 break 于成功时刻（13961-13964），不证明“成功后停止”（无限重发下 marker 截面通常仍为 2）；无 pid/launch 断言。我方“杀死无限重发并间接排除重连”强度撤回；k3“恰好 2 次隐含证明停止”同样不成立。测试 2 仅证明耗尽短路单点，0→4 链不完整（3→4 无测试） |
| 4. send_text 仅入队非 wire | **采纳为表述纠正（无功能缺陷）** | `StdioTransportDriver::send_text`（1334-1340）= `try_send` 入有界 mpsc；实际写入异步（1189-1196），失败走 disconnect 事件。我方初审复述 PR 注释“actually written”系把注释当执行证据，已纠正 |

**对我方初审的修订**（已列修订表）：
- 撤回“未发现新增回归级缺陷”——反例 2 为真实新增回归；反例 1 为修复不完整（非劣化于基线）
- 测试 1 证明力降级为“同连接第二次重发 + 应答入 store”的窄版本
- O2 结论保留（分析对象是 queue 层窗口）但补充遗漏了 driver 层窗口
- “wire 挂钩”表述纠正为“入 driver 队列即挂钩”

修订后综合评级：核心场景修复有效，两条中级新发现，建议 request-changes 方向修订后合并；是否阻塞归外层裁量。
