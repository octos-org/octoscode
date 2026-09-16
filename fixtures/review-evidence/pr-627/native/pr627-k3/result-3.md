---
slug: pr627-k3
outcome: completed
updated_unix: 1788923969
turn: 3
---

互审 v2 完成。

**产出**：`/private/tmp/octoloop-glm-k3-20260909/pr-627/.octos/cross-k3.md`（初审文件未动）

**GLM 逐项裁定**：F1-F9 全部**采纳**（F7 仅第 5 项措辞修正——complete 是 archive 合法源状态，无 abort 场景可测，不计缺口）。F1（archived chip 同 active 字形）是 GLM 最有价值的补充，k3 初审漏掉了渲染层。上游契约六项与 k3 独立判断全部一致，含我补验的通知 generation=0 不推水位细节。

**外层反例裁决（核心结论）**

- **根因假说：成立。** 穷举核实：`pending_goal_transition` 写入点仅 1 处（dispatch）、消费点仅 GoalGet arm（不区分触发源）、丢弃点仅 GoalClear。**错误/取消/重连/成功 Set 均不清**；consume 用 fresh goal_id + staged 意图——后端身份断言防 stale id，防不了"旧意图 + 新有效 id"。
- **反例 A：采纳，定级 major。** 事件序列全部由普通操作组成（archive refresh 失败 → `/goal new task` 的 Set 不清 slot → bare `/goal` 触发 consume → `Archive(new-goal)`）。用户从未意图归档的新 goal 被不可逆归档，后端断言在此场景无效。pause/resume 的同类重放是遗留薄弱机制（可逆、同 goal），但**本 PR 把不可逆终态动词装进无生命周期管理的槽位，升级部分属本 PR 责任**。
- **反例 B：采纳，与 A 同根因同修复。** 动态证实了 GLM F9 的静态推断；`reconcile_after_backend_relaunch` 清十余类状态唯独漏 slot，亲验属实。k3 正式将初审 nit#3 上调至 minor（并入 major 根因项）。
- **诚实声明**：反例动态执行为外层在独立合成树完成（adversarial.log 两条 FAILED panic 证据），k3 仅独立核实源码可达性，未冒称执行。

**修订后合入建议**：核心链路正确性结论不变，但从"可合并"修订为 **"1 个 major 修复后可合并"**——最小修复三处清理（relaunch / GoalSet arm / refresh-error），结构化修复为槽位绑定原 goal_id；外层两条 repro 应收为回归锚点。

**specs 缺口**：AGENTS.md 契约要求"行为改动同步更新 specs/"，本 PR 未触碰 specs/，goal 动词族 spec 缺失是先存存量缺口——定为 minor 纪律项，建议 ACK 中回应，不阻塞。

**k3 补充发现**：无新增重大问题；两条独有 nit（ledger reason locale 漂移、`target_status` 死代码）维持原判。
