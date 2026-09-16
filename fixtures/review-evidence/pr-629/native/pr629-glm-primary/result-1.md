---
slug: pr629-glm-primary
outcome: completed
updated_unix: 1788920900
turn: 1
---

第一阶段独立盲审完成，两份输出均已写入：

**报告**：`.octos/independent-glm.md`（122 行）＋ native `result.md`（briefs 目录下，615 bytes）

## 核心结论：**批准（Approve）—— 无本 PR 新增回归**

**验证基础**
- HEAD 核对 ✅（`9bcf4099` / 父 `0a174d95`）；diff 仅 +104 行（store.rs +102，en/zh locale 各 +1）
- 未读任何其他审查报告（盲审约束遵守）
- 上游权威源码经 `git show d51601df:<path>` 只读核对

**根因链逐环核实为真**（doc comment 的每条声明都对得上上游锁定提交）：
1. `profile_id` None → serde 缺字段 ✅
2. 服务器 `raw_scoped_llm_profile_id` 默认 `MAIN_PROFILE_ID`（`"_main"`，`octos-core/types.rs:494`）✅
3. `validate_slug_shape` 仅收 `[a-z0-9-]`，`_` 被拒 ✅
4. `save_with_merge` → `internal_error` → `-32603` ✅
5. "Continue to Workspace" 由 saved-provider 门禁锁死 ✅
6. `sub_providers/upsert` 同链路 ✅
7. **test/fetch_models 豁免理由成立**：两 handler 只读 `get` + 内存 `default_profile` 兜底，不触 `save_with_merge`，永不撞 slug 校验 ✅

**覆盖完整性**：全仓 `ProfileLlmUpsert` 仅 2 个构造点、wizard 的 `SubProvidersUpsert` 仅 1 个（inline `/research add` 早有自己的 None 拒绝）——三个 save 出口全部 gate，无遗漏。gate 位于 pending 置位之前，顺带消除旧失败路径的 spinner 卡死次生伤害。

**遗留观察（非阻断，非本 PR 回归）**：
- **O-1 (Low)**：`_main` 可能经 test 结果 `to_list_result()` 回灌 `profile_llm_state`，再经 `current_profile_for_onboarding` 第 4 层把 `_main` 当“已解析”绕过 gate——但需 keyless family test 成功的苛刻前提，且旧行为本就如此（代码推理级，未动态复现）
- **O-2 (Info)**：认证连接（`X-Profile-Id`）上 `_main` 默认实际不可达，旧 bug 在远程认证场景表现为静默写错 profile 而非 -32603；doc comment 未提此分支，属轻微不完整
- **T-1/T-2 (Info)**：research-lane gate 与菜单行入口无专属测试（前者在当前拓扑下不可达为 false，风险≈0）

**验证级别：partially-verified**（静态调用链 + 上游只读核对；按硬性限制未运行 cargo，无动态/UI 实测）。
