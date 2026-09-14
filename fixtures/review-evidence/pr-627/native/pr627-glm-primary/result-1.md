---
slug: pr627-glm-primary
outcome: completed
updated_unix: 1788923129
turn: 1
---

**报告文件**：`/private/tmp/octoloop-glm-k3-20260909/pr-627/.octos/independent-glm.md`

**审查范围**：`git diff 0a174d9..fbb59c3`（HEAD 已核验为 fbb59c32f5f830c5a63e409de6dbe123bafeb922），上游契约对照 `d51601df`（octos-core rev 与 Cargo.lock 一致）。验证级别：partially-verified，动态未验证（未跑 cargo）。

## 逐项发现（无 blocker / 无 major）

| # | 位置 | 触发 | 影响 | 严重度 |
|---|---|---|---|---|
| F1 | src/app.rs:4017 `goal_status_display`（本 PR 未改，但 archived 经本 PR 首次可达 TUI） | `/goal archive` 成功 → mirror status="archived" → 落入 `other => ("◆", …)` | 终态目标与 active 同字形 ◆，操作者无法一眼区分"在跑"与"已归档"；上游 #25 设计里 archived 是不可逆终态，渲染未配套 | minor |
| F2 | src/store.rs:9108-9122 | goal `budget_limited` 且 `tokens_used(1200) >= budget(1000)` → `/goal reopen` → 刷新返回同一记录 → TUI **仍派发** operator_transition，同时状态线已设 "Goal resumed but still over budget…" → 后端超预算守卫直接拒绝（"cannot reopen goal … exhausted its token budget"） | 状态线在 RPC 被拒前断言"已恢复"，与实际结果（被拒、状态未变）矛盾；模式沿袭 resume 遗留路径，本 PR 复制到新分支。后端守卫兜底，无数据损坏 | minor |
| F3 | locales/en.yml:610、zh.yml:1372（`cannot_verb_goal_state`，消费点 store.rs:2577、9063） | `/goal archive` 在目标已 archived 时 | 状态线显示 "Cannot archive goal in `archived` state **(model-owned)**" —— archived 是 operator-only 终态（上游 `set_goal` 明确拒绝该状态），"模型管控"措辞误导排障；reopen 自 complete 被拒同理 | minor |
| F4 | src/store.rs:12623-12633 | 归档通知 `status="archived"` 不在 toast 分派列表 | 落入兜底 "Goal status: archived"，无终态/不可逆语义提示，与 complete 有专属 toast 不对称 | nit |
| F5 | src/model.rs:5388-5394 | 阅读代码 | `pending_goal_transition` 字段注释仍只描述 pause/resume+set，未提 operator_transition；实现已泛化为 `PendingGoalFollowUp` 枚举，注释滞后 | nit |
| F6 | locales en:985 / zh:471（command.goal.desc）、docs/PEER_GOAL_ARCHITECTURE.md:104-112、docs/OCTOLOOP_GUIDE.md:160-165 | 查 /help、命令菜单、文档 | 动词 archive/reopen 无任何可发现性入口；OCTOLOOP_GUIDE 仍指引用户"serve 存活时走在线 RPC"却未提本 PR 的正解 `/goal archive` | nit |
| F7 | src/store.rs 测试模块（缺口） | — | 测试缺口：(1) 服务器未广播 operator 方法时的门控拒绝无测试；(2) readonly 阻断测试数组未含新命令（实现已加，transport.rs:2746）；(3) clear 丢弃 staged **operator** 过渡未测（只测了 Set）；(4) F2 超预算 reopen 分支无测试；(5) refresh 返回 complete 时 archive 的对偶 abort 测试缺失；(6) Params 无 wire 序列化测试。报告内附可直贴的测试片段 | nit |
| F8 | src/autonomy.rs:460-487 | `/goal archive --budget 5m` → 非 `--reason` 旗标 → 回落 `Set`，objective="archive --budget 5m" | 会按 set 语义替换当前目标；防劫持注释已论证该回落合理性，属设计取舍，但与 `--budget` 组合的静默替换值得在帮助文案点名 | nit |

**证据均已核对源码并贴入报告**；F2 同时贴了上游 `operator_transition_goal` 的拒绝守卫原文对照。

## 已核验无问题的关键面

方法名/能力门控（`coding.goal_runtime.v1`，旧服务器前端拒绝）、请求/结果 wire shape 与后端 `RawGoalOperatorTransitionParams` / 无 `ok` 字段结果逐字段对齐、`allows_from` 与后端状态机一一镜像（archive 自 complete 合法、reopen 仅 blocked/paused/budget_limited）、刷新先行 + 新鲜 goal_id 身份断言、readonly 阻断、reason 不入历史（args 非空不入）、断线后 staged 过渡滞留的最坏后果是无害中止（goal_id 断言 + session 比对双保险）、result 与 `SessionGoalUpdated` 通知双路径落地一致。

## 总体结论

**建议可合并（no blocker / no major）**。核心链路与上游 d51601df 契约逐项对齐，6+4 个新测试的断言目标与 PR 承诺真实对应（含非法路径：劫持防护、状态漂移中止、裸动词解析），向后兼容处理正确。3 个 minor（F1/F2/F3）与 F7 测试缺口适合作为跟进 PR，不阻塞。

盲审约束已遵守：未读取 `.octos/` 下任何其他报告或先前 Codex 报告；未修改源码、未 commit/push、未启动子进程、未跑 cargo。
