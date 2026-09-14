---
slug: pr628-glm-primary
outcome: completed
updated_unix: 1788922389
turn: 6
---

互审收尾轮 v2 完成：

- **`/private/tmp/octoloop-glm-k3-20260909/pr-628/.octos/cross-glm.md`**（唯一新写入文件，frontmatter 含 `outer_evidence=adjudicated`），含三节：A) k3 报告逐项裁决（v1 结论维持）、B) 外层反例裁决、C) 双方结论修订。
- **native result.md + `.result-owner`（peer）** 已更新。

三条反例的裁决核心：

| 反例 | 裁决 | 生产路径定位 | 定级 |
|---|---|---|---|
| 1. idle 有历史时 Warning reason 渲染不出 | **采纳** | turn-less Warning 与存量 turn-less 活动同组（分组键 `item.turn_id`），settled 组折叠只留 "Agent task completed" 组头（transcript_build.rs:3345-3349），reason 整组不渲染 | **major** |
| 2. active turn 时 Warning 被过滤 | **采纳** | `flow_activity_items` 只保留与本 turn 同 id 的 item（app.rs:2635-2641），`reject_autonomy_slash` 的 Warning 无 turn_id（对比 review_start 有 `.with_turn`） | **major** |
| 3. 重复拒绝第二次退化 | **采纳** | 与我方 F1 静态预测逐字吻合（adversarial.log left/right） | minor |

**结论修订**：我方初审「无 critical/major」撤回——初审验证了 model 层写入与结构正确，但把「写入 activity」隐含等同「transcript 可见」，未核查渲染管线。新增测试只断言 `activity.last()`，恰在失效层之下，所以全绿——这正解释了为何 PR 测试通过而交付在真实渲染路径不成立。最终：**major×2（渲染不可见）+ minor×2 + info×3 + 测试缺口，不建议按当前形态合入**。

外层动态证据全部引自 `adversarial.log`（非我运行），我的部分为静态渲染链复核，二者独立指向同一结论。未覆盖任何 independent-* 文件，仓库只读未动。
