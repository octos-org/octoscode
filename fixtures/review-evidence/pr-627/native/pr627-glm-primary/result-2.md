---
slug: pr627-glm-primary
outcome: completed
updated_unix: 1788923216
turn: 2
---

报告已收口完成。

## 报告路径
`/private/tmp/octoloop-glm-k3-20260909/pr-627/.octos/independent-glm.md`（约 210 行，盲审全程未读取 `.octos/` 下其他报告）

## 发现摘要

**无 blocker / 无 major**。4 minor + 4 nit + 1 个测试缺口清单：

| # | 严重度 | 位置 | 问题 |
|---|---|---|---|
| F1 | minor | src/app.rs:4017 | `goal_status_display("archived")` 落入兜底分支显示 active 同款 ◆ 字形——本 PR 首次让 archived 对 TUI 可达，终态无专属渲染（新增可达状态+未配套） |
| F2 | minor | src/store.rs:9108-9122 | 超预算 reopen：TUI 仍派发请求且状态线提前断言 "Goal resumed…"，后端守卫实际会直接拒绝；resume 遗留模式被复制到新分支 |
| F3 | minor | locales/en.yml:610、zh.yml:1372 | `/goal archive` 在已 archived 时报 "(model-owned)" —— archived 是 operator-only 终态，措辞误导（上游 `set_goal` 明确拒绝该状态） |
| F9 | minor（部分**待验证**） | src/store.rs:8821、9037；transport.rs:765 | 断线后滞留的 staged operator 过渡会在重连 hydration 的 GoalGet 到达时**延迟执行**（无确认/超时机制）。静态已证实「GoalGet 到达即无条件 consume」+「断线取消不清 pending」；缺动态断线→重连→hydration 端到端复现来证实时序。双层守卫（session 比对+goal_id 断言）保证不会误伤其他目标 |
| F4 | nit | src/store.rs:12623-12633 | archived 过渡 toast 落通用兜底，无终态语义提示 |
| F5 | nit | src/model.rs:5388-5394 | `pending_goal_transition` 字段注释仍是旧 pause/resume 语义，实现已泛化 |
| F6 | nit | locales en:985/zh:471 + docs | 命令描述与文档均未提 archive/reopen，可发现性缺口 |
| F7 | nit | 测试缺口 | 6 项：能力门控拒绝、readonly 阻断回归锚点、clear 丢弃 operator pending、超预算 reopen 分支、archive-from-complete 对偶 abort、Params wire 序列化（报告内附可直贴测试片段） |
| F8 | nit | src/autonomy.rs:460-487 | `/goal archive --budget 5m` 静默回落为 Set 替换目标——防劫持设计的刻意取舍，建议文档点名 |

## 总体结论

**Approve（可合并）**。核心链路——解析防劫持 → 能力门控（旧服务器不广播即前端拒绝）→ 刷新先行 → 新鲜 goal_id 身份断言 → 后端状态机镜像（`allows_from` 与上游 `operator_transition_goal` 一一对应）→ 无 `ok` 字段结果解码 → mirror/actor 落地——与上游锁定提交 d51601df 契约逐项对齐；6+4 个新测试断言真实覆盖 PR 承诺（含非法路径）。全部 minor/nit 均不阻塞，适合跟进 PR 处理。

## 验证级别

**Partially-verified**：静态代码审查 + 上游权威源码只读对照；**动态未验证**（未运行 cargo 编译/测试，按约束留给外层统一跑）。唯一标注「待验证」的是 F9 的端到端时序。

已就绪，等待与 k3 车道的双向互审分派。
