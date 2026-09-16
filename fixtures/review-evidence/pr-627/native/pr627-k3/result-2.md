---
slug: pr627-k3
outcome: completed
updated_unix: 1788923501
turn: 2
---

互审完成。

**产出**：`/private/tmp/octoloop-glm-k3-20260909/pr-627/.octos/cross-k3.md`

**逐项裁定摘要**

| GLM 编号 | 裁定 | 要点 |
|---|---|---|
| F1 archived chip 字形同 active（◆） | ✅ 采纳 | 亲验 `app.rs:4017` 兜底 `other → ◆`；GLM 最有价值的补充，k3 初审漏掉渲染层 |
| F2 超预算 reopen 仍派发+提前断言 | ✅ 采纳 | 与 k3 发现 1 独立重合；GLM 修复建议（TUI 同判后不派发）优于 k3 版本 |
| F3 operator 拒绝文案带 "(model-owned)" | ✅ 采纳 | 亲验 `en.yml:610/850` 两个 key 均带错位的责任方后缀 |
| F4 archived toast 落通用兜底 | ✅ 采纳 | `store.rs:12623-12633` 分派无 archived 臂 |
| F5 `pending_goal_transition` 注释滞后 | ✅ 采纳 | `model.rs:5389-5394` 仍只提 pause/resume/set |
| F6 可发现性（desc/docs 未提新动词） | ✅ 采纳 | `en.yml:985` desc 确无 archive/reopen |
| F7 测试缺口 7 项 | ✅ 采纳 6/7 | 第 5 项措辞修正：complete 是 archive 合法源状态，无 abort 场景可测，不应计缺口 |
| F8 `--budget` 混排静默变 Set | ✅ 采纳 | 设计取舍，与 k3 §1.1 边角记录一致 |
| F9 断线滞留 slot 延迟执行 | ✅ 采纳（minor） | **GLM 版本比 k3 发现 3 更完整**：hydration 队列（store.rs:9183-9190）重连后自动发 GoalGet，无需用户操作即触发消费；三环节源码链亲验全部成立 |

**上游契约六项（门控/wire shape×2/状态机镜像/goal_id 断言/通知双路径）**：与 k3 独立判断全部一致。补验了 GLM 未展开的一点——通知 generation 缺省 0 时 TUI `goal_event_generation_admits` 走 "always apply 不推进水位" 分支，双路径无乱序冲突。

**争议项最终立场**：
1. **断线滞留 slot 严重度从 k3 的 nit 上调至 GLM 的 minor**——hydration 自动消费路径显著拓宽了触发窗口，archive 又是终态不可逆动词。
2. F7 第 5 项不计缺口。
3. 修复优先级：F2 > F9（一行清 slot）> F1 > F3 > 其余 nit。

**k3 补充发现**：无新增重大问题。k3 独有的两条 nit（默认 reason 入 ledger 的 locale 漂移、`target_status` 死代码）维持原判，GLM 未列但不冲突。两车道并集即可作为合并前修复清单。
