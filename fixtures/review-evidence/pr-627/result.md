---
slug: pr627-master
outcome: completed
updated_unix: 1788924400
turn: 1
verified: partially-verified
protocol: olp/v2
---

# PR #627 双车道评审综合结果（master 汇总 v2，含外层裁决）

## 0. 任务与 goal 元数据

- goal_id：goal_01（外层未显式指定 token budget；runtime 采用默认 100M）
- 仓库：/private/tmp/octoloop-glm-k3-20260909/pr-627（只读 clone）
- HEAD：fbb59c32f5f830c5a63e409de6dbe123bafeb922（已核验）；基线 0a174d95ddec2b123adb3498432e29eb13affb81
- peer 车道：pr627-glm-primary（primary glm-5.3；初名 pr627-glm review 车道 native errored 后按活板 #2 重派）；pr627-k3（strong/k3-256k）
- 终态核验（本轮收齐）：pr627-glm-primary native outcome=completed（turn 4, 1788923589）；pr627-k3 native outcome=completed（turn 3, 1788923969）；pr627-glm（旧 review 车道）outcome=errored 已关闭不计入。双模型身份成立，无降级冒称。

## 1. 四份 peer 报告（存在性 + SHA256 取证）

| 文件 | 车道 | 字节 | SHA256（前16） |
|---|---|---|---|
| .octos/independent-glm.md | GLM 首审 | 20908 | 640fc49fff425b46 |
| .octos/independent-k3.md | k3 首审 | 16051 | 5bad206aba0e2660 |
| .octos/cross-glm.md | GLM 互审（含外层反例裁决） | 13358 | 30e53763f4e0780f |
| .octos/cross-k3.md | k3 互审（含外层反例裁决） | 14854 | 3dc533e4e65baaf8 |

四份报告独立落盘、互不覆盖；两份初审在互审阶段冻结原文未改；动态证据归属外层（adversarial.log）的标注在两份 cross 中均保留。

## 2. 外层终裁（本轮 master 采纳的裁决基线）

外层已读两份含反例裁决的 cross，采纳反例 A/B 与 Request changes。master 按外层裁决明确三点：

1. **定级分歧的统一处理**：GLM 裁反例 A 为 blocker、k3 裁 major，二者为**定级分歧而非事实分歧**（事件序列、根因、可达性两车道逐环核实一致）。外层统一为 **P1 / 合入前必须修复**，master 记录为最终定级，不再沿用 blocker/major 之辩。
2. **不采纳"旧 pause/resume 仅可能作用于同一 goal"的缓解说法**：旧机制同样缺目标关联（同一无生命周期槽位、同一无条件消费点），并不天然局限同一 goal；准确表述是——**本 PR 把不可逆终态动词（archive，上游 reopen 不可自 archived）装进了本就缺关联的槽位，新增的是不可逆影响**，风险升级部分归本 PR。
3. **specs 缺口不因存量先例豁免**：AGENTS.md 明文要求行为改动同步更新 specs/，本 PR 未触碰 specs/ 且 goal 动词族无 spec——属**合入前必须补**（本 PR 或并入的规范要求），不因"整个 goal 族均无 spec"的旧先例降级为可选跟进。

## 3. 评审结论（两车道独立 → 互审收敛 → 外层终裁）

### 3.1 核心承诺：两车道独立核实全部成立（互审后维持）

1. fresh goal_id 身份断言（dispatch 先 GetSessionGoal 取新鲜 id 再发 operator_transition，cached/fresh 分叉有专测）
2. per-action 状态域与上游 operator_transition_goal 守卫逐条镜像（archive 允许一切非 archived 含 complete；reopen 仅 blocked|paused|budget_limited；over-budget reopen 后端硬拒绝）
3. /goal archive <free text> 反劫持解析（词边界 + MissingReason，objective 语义保留，有专测）
4. operator result 无 ok 字段的独立解码设计（拒绝经 JSON-RPC error 帧）
5. 旧服务器能力前置拒绝（require_mutating_appui_method + 上游 d51601df 方法广播表）
6. 通知双路径（result 与 SessionGoalUpdated 都落 mirror；k3 补验 generation=0 不推水位）

### 3.2 唯一实质争议与收敛

staged transition（PendingGoalTransition）断线/relaunch 后永驻 slot 的严重度：k3 初审 nit → GLM 首审 minor（发现 hydration 自动消费路径 store.rs:9183-9190）→ k3 互审正式上调一致 → 外层动态反例 A/B 证实后统一 P1。

### 3.3 外层动态反例（两车道逐环静态核实，动态执行归外层）

- **反例 A**：archive refresh 失败（goal_unavailable）→ /goal new task 的 Set 不清 slot → 普通 GoalGet 无条件消费旧 Archive 意图 → 从未意图归档的新目标被不可逆归档。根因：pending_goal_transition 写点 1/消费 1/丢弃仅 GoalClear，错误、取消、重连、成功 Set 均不清；fresh-id 断言防 stale id 防不了"旧意图 + 新有效 id"。
- **反例 B**：archive refresh 被取消（request_cancelled）→ BackendConnectionEpoch/Relaunched 不清 slot → 重连 hydration 队列自动 GoalGet → 旧 Archive 幽灵生效。reconcile_after_backend_relaunch 清十余类状态唯独漏该 slot。

### 3.4 修复清单（合入门槛）

1. **P1（合入前必须）**：pending_goal_transition 生命周期修复——三处清理（reconcile_after_backend_relaunch 清 slot、AutonomyResult::GoalSet arm 丢弃同 session pending、refresh 错误/取消路径丢弃），结构化方案为槽位绑定发起 request id/connection epoch/原 goal_id；外层两条 repro 收为回归测试。
2. **P1（合入前必须）**：补 specs/ ——goal operator 动词 spec，绑定 goal_archive_dispatches_operator_transition_with_fresh_goal_id 等真实测试名（AGENTS.md 契约，见 §2.3）。
3. 应修 minor：over-budget reopen 的 TUI fail-fast（TUI 已有全部字段可同判不发请求，优于换文案）；archived chip 字形与 active 区分（app.rs:4017 兜底 other → ◆）；operator 路径拒绝文案去 "(model-owned)" 错位。
4. nit：默认 reason 入 durable ledger 的 locale 漂移（locale 无关机器字符串）、archived toast 兜底、pending 字段注释滞后、命令描述/docs 可发现性、target_status 死代码、--budget 混排静默回落 Set（设计取舍，文档点名即可）。
5. 测试缺口（互审收敛后）：能力门控拒绝、readonly 阻断回归锚点、clear 丢弃 operator pending、over-budget reopen 分支、wire 序列化、--budget 混排钉住（F7 第 5 项经 k3 修正不计缺口）。

### 3.5 总体合入建议

**Request changes**（初审两车道均 approve；外层动态反例 + 两车道互审逐环静态核实后修订）。核心链路（解析/门控/wire/状态机镜像/断言/双路径）实现正确、测试到位；阻断项为意图槽位生命周期（反例 A/B，P1）与 specs 补同步（P1），修复后可合并。

## 4. 验证级别声明

- 本任务全程静态审查（两车道独立 + 互审 + master 汇总），未运行 cargo 编译/测试（按约束留给外层统一 --all-targets + clippy + fmt）。
- 动态证据仅两条外层反例（adversarial.log），由外层在独立合成树执行；两车道与 master 均未冒称执行。
- 上游对照锁定 d51601df（与 Cargo.lock octos-core rev 一致）。
- **整体验证级别：partially-verified。**
