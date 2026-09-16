---
slug: pr628-glm-primary
outcome: completed
updated_unix: 1788922425
turn: 7
---

核实结果：监督方看到的 10:46 版本与当前盘面不同——**当前 `cross-glm.md`（10:52 落盘，83 行）已包含全部要求的内容**，本轮只需追加一节存在性注记，无需重做裁决：

**已确认在盘内容（非本轮新增）**：
- frontmatter 第 5 行已有 `outer_evidence=adjudicated` ✅
- B 节「外层新增反例裁决」（第 30-69 行）已逐条裁决：
  - **反例 1 → 采纳，升 major**：idle 有历史时 Warning reason 被 settled-group 折叠吞掉（turn-less Warning 与存量 turn-less 活动同组，`push_agent_task_group` 的 `collapse_settled` 分支只留 "Agent task completed" 组头，transcript_build.rs:3345-3349）——与 adversarial.log 渲染输出精确吻合
  - **反例 2 → 采纳，升 major**：active turn 时无 turn_id 的 Warning 被 `flow_activity_items` 过滤（app.rs:2635-2641 只保留与本 turn 同 id 的 item；对比 `review_start_command` 有 `.with_turn`，`reject_autonomy_slash` 没有）
  - **反例 3 → 采纳**：与我的 F1 静态预测逐字吻合（log 中 left="command is unavailable" / right=具体 reason）
- C 节已撤回双方「无 major/可合入」结论，修订为 **major×2 + minor×2 + info×3 + 测试缺口**
- 证据来源全程区分：外层动态引自 adversarial.log（明示非我运行），我的部分为静态渲染链复核

**本轮追加**：E 节增补注记（v3）——仅说明时间线与结论指针，不重复裁决内容。

**native result.md + `.result-owner`（peer）** 已更新为一句话结论版本。最终裁决不变：**PR 核心承诺（rejection 可见于 transcript）在两条生产渲染路径不成立，不建议按当前形态合入**。这是最后一次增量，结束。
