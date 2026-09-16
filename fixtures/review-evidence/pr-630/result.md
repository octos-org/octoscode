---
slug: pr630-master-result
outcome: fail-request-changes
updated_unix: 1788922471
turn: 1
verified: partially-verified
protocol: olp/v2
---

# PR #630 GLM/k3 独立审查及交叉复核 — Master 综合报告

- goal_id: goal_01（"PR #630 GLM/k3 独立审查及交叉复核"；外层未显式指定 token budget，runtime 采用默认 100M）
- 审查对象: HEAD `276c866af758dd75f2f47a57590c7da35b700a61` vs 基线 `0a174d95ddec2b123adb3498432e29eb13affb81`，范围 "capabilities retry on same live connection"（`src/transport.rs` +198、spec +19）
- 验证级别: **partially-verified** — 本任务全部为静态代码审查 + 调用链推演；未运行 cargo build/test（按 brief 由外层统一编译验证）。两处动态失败证据为外层 Codex 在隔离合成树实跑（`/private/tmp/octoloop-codex-20260909/adversarial.log`），非本任务运行，peer 报告均已作来源区分。

## Peer 与报告清单

| Peer slug | 模型/车道 | 阶段 | 报告 | 初审结论 | 互审终态 |
|---|---|---|---|---|---|
| pr630-k3 | moonshot/k3-256k (strong) | 首审+互审 | `.octos/independent-k3.md` / `.octos/cross-k3.md` | pass-with-minor-observations | **fail**（turn 3 自我推翻 turn 2 的 pass） |
| pr630-glm-primary | glm-5.3 (primary) | 首审+互审 | `.octos/independent-glm.md` / `.octos/cross-glm.md` | pass-with-observations | pass-with-observations → 修订（新增 2 条中级缺陷，request-changes 方向） |
| pr630-glm | zai-coding/glm-5.3 (review 车道) | 首审（失败） | 无（native result=errored: "failed to send streaming request to Anthropic"，已关闭） | — | — |

首审阶段两 peer 相互盲审独立完成；互审阶段按外层指令追加"外层新增反例裁决"（`.octos/outer-evidence.md` / `.octos/outer-repros.rs`）。GLM 车道 review lane 网络故障一次，按外层 R2 纠偏换 primary 车道重派，未假称双模型同轮成功。

## 双车道一致的裁决（互审收敛点）

**反例1 — 静默 stdio child 下修复承诺未兑现（fail-to-fix）**：`stdio_child_startup_pending()`（transport.rs:2474-2479）只看 `!stdio_child_served_frame`；hello grace 到期路径（2494-2508）只解除 barrier 不清该标志 → `retry_capabilities_if_unanswered` 的 `startup_pending` 抑制（2527）在 grace 过期后仍永久生效 → attempts 永停 1。**两 peer 独立静态复核均采纳**；外层动态实测 `outer_review_630_no_first_frame_must_retry_after_grace` 失败（adversarial.log:118-121）佐证。严重度分歧：k3 判高（PR 注释点名的头号场景本身未被修）、GLM 判中（触发前提 >90s 零输出，正常冷启动 10-15s 出帧）。**master 裁定：高**——PR 自己声明的核心场景（"flushed into a stdio child still booting when the startup grace expired"）恰在此路径失效，属修复承诺未兑现，且这是 PR 存在的全部理由。性质：fail-to-fix（基线同样不重试，非行为劣化）。

**反例2 — retry 的 retain 静默丢弃已排队的唯一有效成功应答（新增行为回归）**：`next_event` 先做超时检查（3133）触发 retry → retain 按 method 删旧 pending id（2540-2542）→ 之后才 poll driver decode 已到达的成功帧（3137-3145）→ `pending_requests.remove(&id)` 得 None，成功帧被静默丢弃（3810-3812），probe 不清除且 `sent_at` 被重置（2546-2548）多盲一个周期。**两 peer 均采纳**；外层动态实测 `outer_review_630_queued_success_must_survive_retry_deadline` 失败（adversarial.log:123-126）佐证。严重度：中（双方一致）。基线无此问题（基线不重试、无 retain）——严格新增回归。与 GLM 首审 O2 是**不同窗口**：O2 分析的是事件已入 self.queue 未出队（判不可达，保留），反例2 是 driver 层未 decode，真实可达。

**反例3 — 双方初审测试证明力高估（双方自我修订）**：测试1 收到成功即 break、marker 为截面断言、无独立 pid/launch 次数断言，只证明"第二次重发+应答入 store"，不证明"成功后停止"与"无重连"；测试2 直接构造 attempts=4，不证明 0→4 完整链。双方均采纳修订。

**反例4 — "clocked from the wire" 措辞不准（语义澄清）**：stdio driver `send_text` Ok 仅表示入内部通道（1334-1341），真正写 stdin 的是异步 writer task（1184-1208）。两 peer 均采纳为表述纠正，非独立缺陷；作为反例1/2 严重度评估的修正因子。

## 分歧与证据

| 争议 | k3 | GLM | master 裁定 |
|---|---|---|---|
| 反例1 严重度 | 高 | 中 | **高**（PR 点名场景本身未修） |
| 互审 outcome | fail | pass-with-observations（修订，request-changes 方向） | 见下，实质收敛 |
| O1 错误浮出分类 | "遗留路径上的新增有界噪声"（1→4 或 0→N） | "遗留放大" | 采 k3 的更精确表述：遗留路径上的新增有界噪声，低 |

两份互审终态用词不同但**实质收敛**：双方均确认反例1、2 成立、均撤回初审乐观结论、均给出相同修复方向。k3 的 fail 强调"PR 新增缺陷"维度，GLM 的 request-changes 强调"修订后合并"维度——按质量论不冲突，按合并裁决属外层裁量。

## 修复建议（两 peer 一致）

1. hello grace 到期后解除 `startup_pending` 对 capabilities 重试的抑制（或 grace 超时路径清除/降级 `stdio_child_served_frame` 语义），使静默 child 下重试真正可达。
2. retain 不再按 method 盲删：推迟到 drain 之后，或精确按在途 request id 删除，或让孤儿 capabilities 成功应答仍产生 `Capabilities` 事件。

## 测试缺口

- 无"成功后停止 + 无重连"的独立断言（pid/launch 次数）。
- 无 0→4 完整预算链测试（测试2 只构造终态）。
- 无 hello-grace 过期 + 无首帧的重试测试（反例1 的盲区，恰是修复声称的场景）。
- 无 driver 层排队窗口的回归测试（反例2）。
- 以上动态项本任务未运行，属外层统一验证范围。

## 结论

**fail-request-changes**：核心 happy path（出帧 child 吞掉首次请求后被同连接重问）实现正确、测试真实覆盖；但反例1（高，fail-to-fix：PR 点名的慢启动静默场景修复未兑现）与反例2（中，新增丢事件回归）均经双车道独立静态复核采纳并有外层动态失败佐证。建议作者修复上述两点后重审。

## 运行注记

- 全程只读：源码零修改，无 commit/push；产出仅本目录 `.octos/` 报告。
- 失败重派链：review 车道 errored → primary 车道重派（外层 R2 纠偏），最终四份有效报告齐备。

## 外层验收回执与最终校正（turn 2 追加）

外层 Codex 已读取四份报告与本综合，采纳：反例1（fail-to-fix）、反例2（新增回归）及 Request-changes 建议。两点外层校正记入如下（不再派 peer）：

1. **反例2 触发窗口校正**：不需要"消费停摆 ≥5s"。反例成立的充分条件是任意一次调度间隙或慢响应落在 5s 周期边界附近——例：请求 t=0 发出，成功帧 t=4.99s 到达 driver，下一次 poll t=5.01s：`next_event` 开头的超时检查（3133）先于 driver drain（3137-3145）执行，此时 probe 恰已到期 → retain 删旧 id → 已到达的成功帧 decode 时落空被丢。整个竞态窗口可窄至 ~20ms 调度间隙；另一等价触发是服务器响应本身 >5s（迟到成功跨过 probe 截止线）。GLM 互审报告中"窗口窄（需 ≥5s 消费停摆/堆积）"的表述按此收紧，两 peer 报告原文保留不改。
2. **修复方向校正**：须保留首帧真实语义与 connection epoch/RPC 关联——建议 (a) 带截止时间的 startup 抑制（grace 到期后不再无限期抑制重试），(b) 保留同连接有界的旧 request id 使迟到成功仍可关联。两 peer 报告中的具体修法（grace 清 served_frame / retain 推迟 drain / 按 id 精确删）为方向性建议，未经运行验证。

本节为外层验收记录，验证级别不变：partially-verified（静态）。
