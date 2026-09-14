---
slug: pr629-master
outcome: completed
updated_unix: 1788923589
turn: 5
verified: partially-verified
protocol: olp/v2
goal_terminal: complete (goal_01 已于 1788923208 通过 completion verifier)
goal_terminal_history: blocked (1788921711, verifier 三次空判词 NotDone) → complete (1788923208)
resume_record: /goal resume 后，pr629-k3 独立只读核验 master result/活板 ACK/四份 peer 报告（六文件字节数+SHA256、HEAD=9bcf4099、六字段、ACK 条目），证据经其 native result turn-5 录入 goal ledger；随后 completion verifier 依据该 ledger 记录通过 goal_update(complete)。此前三次空判词与 blocked 阶段的历史保留于本文件与活板，未抹去。
---

# PR #629 GLM/k3 独立审查及交叉复核 — master 综合

- goal_id: **goal_01**（profile octosfix）
- 审查对象：PR #629 `fix(onboarding): refuse a provider save with no resolved profile`，HEAD `9bcf4099c2719cd8ee63090a1849a2c6f3766999`，基线 `0a174d95ddec2b123adb3498432e29eb13affb81`，diff = `locales/en.yml`(+1) `locales/zh.yml`(+1) `src/store.rs`(+102)，纯新增。
- 验证级别：**partially-verified（静态）**——全部结论来自只读源码审查 + 上游 pinned commit `d51601d` 只读核对；本轮未运行 cargo build/test/clippy（外层统一跑），未动态复现任何运行时行为，未采用 PR 作者 CI 结果。

## Peer 与车道

| peer slug | model lane / 模型 | 阶段产出 | native outcome |
|---|---|---|---|
| pr629-glm | review (zai-coding/glm-5.3) | 首轮传输失败（failed to send streaming request），**未产出**，已关闭 | errored |
| pr629-glm-primary | primary（继承 glm-5.3 主配置，外层纠偏后重派） | `.octos/independent-glm.md`（110 行）+ `.octos/cross-glm.md`（60 行） | completed ×2 |
| pr629-k3 | strong (moonshot/k3-256k) | `.octos/independent-k3.md`（105 行）+ `.octos/cross-k3.md`（66 行） | completed ×2 |

四份报告文件（均在 `/private/tmp/octoloop-glm-k3-20260909/pr-629/.octos/`）：
`independent-glm.md`（14080B, 10:29）、`independent-k3.md`（10534B, 09:54）、`cross-glm.md`（10420B, 10:31）、`cross-k3.md`（11834B, 10:34）。首审双盲（互不读对方报告），互审为真实交叉：GLM-primary 裁 k3 的 22 项，k3 裁 GLM 的七环根因+四触发序列+§4 正确性+四遗留观察。

## 两车道收敛结论

**Approve（partially-verified）。双方独立盲审一致判定：本 PR 新增回归 = 没有。**

- 根因声明逐环核实为真：`profile_id` serde 缺省不上线（model.rs:3566-3568 / 上游 transport 8736-8743）→ 未认证连接缺省默认 `_main`（上游 ui_protocol_transport.rs:12976-12992、types.rs:494）→ slug 校验 `[a-z0-9-]` 拒绝 `_`（上游 profiles.rs:2693-2707、1786）→ `-32603 failed to save profile`（transport 13090-13092）→ "Continue to Workspace" 被 saved-provider 门永久挡住（menu/providers.rs:2460-2477）。
- 三处 save gate（store.rs:4931 primary / 5016 research-lane / 5049 fallback）覆盖全仓仅有的三个 wizard save 构造点；gate 位于 pending 置位与 key gate 之前，拒绝不残留 spinner（测试断言 provider_pending.is_none()）。
- 测试证明力成立：fixture 显式清 `sessions[0].profile_id` 并断言 `current_profile_for_onboarding().is_none()`，防住五层解析链兜底误过；gate-off 侧由邻接既有测试覆盖。

## 互审争议与裁决（全部收敛，无驳回项）

| 条目 | GLM-primary 裁 k3 | k3 裁 GLM | master 建议 |
|---|---|---|---|
| k3 #3 `_main` 默认 | 采纳（附条件：仅未认证连接；Some(connection) 提前 return） | —（自身初审即有此条件） | doc comment 可补一句认证分支说明，非阻断 |
| k3 #16 test/fetch 不 gate 契约无测试钉住 | 采纳（自认初审漏列） | — | 建议补一条回归测试钉住豁免契约，info 级 |
| **O-1** `_main` 经 keyless test 成功回灌解析链第 4 层 → gate 被 `Some("_main")` 绕过 | 维持「待验证」+ 新增削弱证据（store.rs:11301 有 `profile_llm_list_has_provider_state` 前置，16283-16285） | **采纳并升级为「静态证实、动态未复现」**：补齐服务器侧最后一环（d51601d transport 13362→17381→13623→10814，result.profile_id 恒写 `_main`） | 遗留边角（旧代码同在，test 从不 gate），非本 PR 回归；建议后续在 gate 中校验 `profile_id != "_main"` 或服务器侧拒绝 `_main` upsert，另补动态复现 |
| **O-2** 认证连接 `_main` 默认不可达，doc comment 未提 | 仍成立 | 采纳 | 文档精度问题，info |
| **T-1** research-lane gate 无专属测试 | 双方同判（5016 当前不可达为 false，纵深防御） | 同 | 可选一行测试，非阻塞 |
| **T-2** 菜单行入口未测 | 仍成立（Info；composer 与 menu 汇聚于 4365-4368，其下已覆盖） | 采纳 | info |
| k3 独有：5001 前置拒绝不调 `refresh_active_menu_if_open` | 采纳（读码证实，pre-existing） | 自查仍成立 | 遗留 UI 细节，可顺手修，非本 PR 义务 |
| k3 独有：双文案不一致 / 服务器侧缺口 | 采纳 | 自查仍成立 | cosmetic / 上游范围 |
| k3 #17 测试只断 status 不断 last_message | 采纳（trivial） | 自查仍成立 | trivial |

## 测试缺口汇总（verification 级别均为 partially-verified）

1. research-lane gate 无直接测试（不可达防御路径，info）。
2. test/fetch_models 豁免契约无回归测试（info）。
3. 菜单行入口未测（info）。
4. 未断言 last_message（trivial）。
5. 动态项全部未验证：cargo 编译/测试、-32603 实际复现、TUI 渲染、O-1 回灌链运行时行为——以外层 CI 为准。

## 外层复验反例（外部证据，非本轮内环所跑）

外层 Codex 已归档四份报告并真实运行了反例 `outer_review_629_runtime_main_list_must_not_enable_invalid_save`（**内环未运行 cargo/运行时，以下为外部复验结果转录，不冒称本内环执行**）：

- 无 profile 时，`ProfileLlmList` 回包携带 `{profile_id: "_main", primary: null, fallbacks: []}` → list 事件路径 seed `onboarding.profile_id = Some("_main")`（`apply_profile_llm_list_event` → `maybe_seed_onboarding_profile_id`，store.rs:11164 附近）→ select/key/save 后**实际 dispatch 了 `Some("_main")` 的 upsert**。
- 该路径是 **list 事件回灌**，不同于 k3 在 cross-k3 中静态证实的 test-mutation 回灌（O-1）：它**不受** `profile_llm_list_has_provider_state`（store.rs:16283-16285）非空门限制——O-1 中我方引以为"削弱证据"的前置条件对 list 路径不适用，回灌可达性比我方静态评估更高。
- 定性维持：**遗留路径**（list seed 与 `_main` 均先于本 PR 存在），None 防护未引入新回归；但 "gate 只看 is_some() 放行 `Some("_main")`" 的绕过已由外层动态坐实，建议合入前处理（gate 增加 `_main` 黑名单或服务器侧拒绝 `_main` upsert）。
- **合入前缺口（外层要求）**：补仓库要求的 specs 场景覆盖该 unresolved→list-seed→save 行为。

## 过程记录（ fidelity ）

- 原 pr629-glm（review 车道）native result `outcome: errored`，未产出报告；按外层纠偏改派 pr629-glm-primary（继承 primary glm-5.3），首审/互审均 completed。
- pr629-k3 互审首轮因 GLM 报告缺失如实报 blocked（未伪造裁决），解除后完成 cross-k3.md。
- 全程只读：无 commit/push/rebase、无 cargo、无源码修改；本文件与四份 peer 报告为仅有的写出物。
