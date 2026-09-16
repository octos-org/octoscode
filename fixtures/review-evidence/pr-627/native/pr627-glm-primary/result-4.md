---
slug: pr627-glm-primary
outcome: completed
updated_unix: 1788923589
turn: 4
---

**Cross 报告路径**：`/private/tmp/octoloop-glm-k3-20260909/pr-627/.octos/cross-glm.md`（初审文件保持原样；未写其他报告文件）

## 逐项裁定摘要

**k3 四项发现**：发现 1（over-budget reopen 文案）**采纳**（=我 F2）；发现 2（默认 reason 走 i18n 进 ledger）**采纳**（我初审遗漏的有效补充）；发现 3（staged transition 永驻）**采纳并升格 blocker**；发现 4（target_status 死代码）**采纳**。k3 七项「无问题」主张全部确认，但其 §1.4 断线“核对通过”的总结论被反例推翻。

**k3 核心承诺五项**：fresh goal_id / per-action 状态域 / 反劫持 / 无 ok 解码 / 旧服务器拒绝 —— **全部独立复核成立**（但 fresh-id 承诺的防护范围有限，见反例 A）。

**GLM 补充发现**：G-1 archived 字形（minor）、G-2 model-owned 文案（minor）、G-3/G-4 nit 集合与测试缺口。

**specs 缺口**：`specs/` 67 个文件无任何 operator_transition/archive/reopen 场景；project.spec 要求“每个验收场景绑定显式测试 selector 并落 spec”——本 PR 新增 10 个测试但零 spec 承载。列为 process 级缺口（minor/nit，交外层裁量）。

## 反例 A/B 最终立场：均采纳（blocker 级）

我未执行任何动态验证（动态证据归外层 adversarial.log），但**逐环静态核实了全部可达性**：

- **反例 A（采纳）**：错误路径五专属 arm 无 goal 清理；Set 成功结果不清 pending（8823-8838）；GoalGet 到达无条件 consume（8821）；`Archive.allows_from("active")==true`；**上游 goal_id 断言不拦截**——客户端发的是新鲜 new-goal id，正是后端 live goal。我初审 F9“三重守卫不会误伤其他目标”的结论对 A **不成立，已更正**。用户从未要求归档的新目标被送入不可逆终态。
- **反例 B（采纳，同根因）**：`request_cancelled` 走 generic error 不清；`BackendConnectionEpoch` 仅 bump epoch；`reconcile_after_backend_relaunch` 清了 orchestration/hydrate/agents 等 7 类状态唯独不清 pending（逐行核对）；重连 hydration 必发 GoalGet → 重放旧 Archive。
- **根因假说（采纳）**：`PendingGoalTransition` 仅存 session/profile/follow_up，无 request id/epoch/原 goal_id；错误、取消、重连、成功 Set 四类事件均不清。A/B 是同一缺陷两个入口。
- **归因区分**：机制先于本 PR（基线 pause/resume 已有），但本 PR 把它引入**不可逆**动词 + archive 状态域含“用户此后新建的任何目标”，后果不可降级（上游 reopen 拒绝 archived），裁为 **PR 级 blocker**。上游 d51601d 后端无任何可缓解行为，纯 TUI 缺陷。

## 修订后合入建议

**Request changes**（由 approve 下调）：最小修复 = 错误/取消即弃 + relaunch 即弃 + 成功 Set 即弃（三处各一行级改动，语义完备），更彻底方案是 pending 携带 request id/epoch/原 goal_id 三比对；并把外层两条反例收编为回归测试。minor 清单（reopen 文案 fail-fast、archived 字形、model-owned 文案、reason locale 无关化）建议合并时顺手；nit 与 specs 同步跟进。

验证级别：partially-verified（静态 + 上游对照；动态未验证，归外层）。
