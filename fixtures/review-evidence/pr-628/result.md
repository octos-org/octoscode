---
slug: pr628-master-result
outcome: request-changes
updated_unix: 1788922900
turn: 7
verified: partially-verified
protocol: olp/v2
---

# PR #628 Master 综合结果：GLM/k3 双车道独立审查 + 交叉复核

- goal_id: goal_01（native goal，本文件为最终综合）
- 审查对象：commit 0c223656926f80509c6d5592ad092230c0e65b10（基线 fbb59c32f5f830c5a63e409de6dbe123bafeb922，范围 autonomy slash rejection visibility）
- 变更：src/store.rs +105/-4（status_before 快照 + reject_autonomy_slash + 三条拒绝路径接出 + 1 新测试）；locales/en.yml、zh.yml /goal 帮助行补 archive/reopen
- 合入判定：**Request changes**（major×2 未修不合入）

## 车道与报告清单

| 车道 | peer slug | model_lane | 首审 | 互审 | native outcome |
|---|---|---|---|---|---|
| GLM | pr628-glm-primary | primary(glm-5.3) | .octos/independent-glm.md | .octos/cross-glm.md（outer_evidence=adjudicated） | completed |
| k3 | pr628-k3 | strong(moonshot/k3-256k) | .octos/independent-k3.md | .octos/cross-k3.md（outer_evidence=adjudicated） | completed |

（原 pr628-glm review 车道 native outcome=errored，已关闭并由 primary 车道替代，见活板 #2。）

外层新增证据：.octos/outer-evidence.md + .octos/outer-repros.rs（外层在隔离合成树实际运行 cargo test，三例 outer_review_628* 全部失败；日志 /private/tmp/octoloop-codex-20260909/adversarial.log）。外层已独立读 transcript_build.rs:974 与 3369 核实 idle 场景直接原因是 settled-group 折叠。

## 证据来源分层（每条结论均标注）

- **[静态]** peer 在本 clone HEAD 只读源码核查（static-read / static+tests-read）
- **[外层动态]** 外层 Codex 在隔离合成树（#628 head + #629/#630）实际 cargo test 失败输出（adversarial.log），非内环 peer 运行
- **[交叉]** 两 peer 互审独立复核收敛

## 终版发现（互审收敛 + 外层反例修订后）

| # | 发现 | 严重度 | 来源 | 位置 |
|---|---|---|---|---|
| R1 | idle + 会话已有历史活动时，拒绝 Warning 的 reason 正文被 settled-group 折叠吞掉，只剩 "• Agent task completed" 类 header——PR 核心承诺（transcript 可见）在主路径不成立 | **major** | [外层动态] 反例1 + [交叉] 双方静态核查采纳（push_activity_section_with_finalization→push_agent_task_group collapse_settled=true 提前 return） | transcript_build.rs:974/2962-2980/3369 |
| R2 | active turn 期间输入非法 slash：Warning 无 turn_id 被 flow_activity_items 过滤，连 header 都不出 | **major** | [外层动态] 反例2 + [交叉] 双方静态核查采纳（app.rs:2636-2639；store.rs:1017-1022 不打 turn_id，对比 store.rs:6996 .with_turn） | app.rs:2635-2639 |
| R3 | 同一非法命令第二次 Enter：reason 因 status==status_before 字符串相等退化 "command is unavailable" | minor | [外层动态] 反例3 与 GLM-F1 静态预测逐字吻合；k3 互审自我修正「fallback 不可达」论断 | store.rs:1011-1016 |
| R4 | fallback 通用文案缺命令上下文 | minor | [静态] k3 首审提出、GLM 互审采纳内容并驳回其可达性论证 | store.rs:1012 vs 6588-6602 |
| R5 | Ok(None) 出口经唯一调用方不可达；注释 "bare /turn" 表述不准 | info | [静态] 双方独立一致 | store.rs:974-977；autonomy.rs:261-264 |
| R6 | parse 错误硬编码英文被抄入 zh transcript（遗留 l10n 延伸） | info | [静态] 双方一致 | autonomy.rs:210-252 |
| R7 | stop 别名 complete/done 未列帮助文案（遗留） | info | [静态] 双方一致 | en.yml:985；autonomy.rs:456 |
| R8 | 测试缺口：只断言 model 层 activity.last()，恰在渲染失效层之下——未断言总数/detail/Err 路径/accepted 无 Warning/重复拒绝/debug_render_text 渲染 | minor | [静态] 双方独立收敛；[外层动态] 反证其全绿原因 | store.rs:40696-40740 |

无 critical。正确性核实通过的部分（保持）：三条拒绝出口各恰一次 Warning、无双发；快照点不被 composer 清理污染；locale 帮助行与 parse_goal 语法一致 [静态，双方独立]。

## 关键交叉过程（裁决与修正记录）

1. 两份盲审初判均为「无 major、可合入」——互审阶段外层反例 + 双方独立静态复核渲染链后一致**撤回**。
2. k3 首审「fallback 不可达」被 GLM-F1 重复拒绝序列推翻，k3 在 cross-k3.md 自我修正。
3. GLM cross 纠正 k3 首审 peer 分支行号错误（505-510 实为保留 draft）。
4. 共同盲区承认：双方均把「push_local_activity 写入 model」当作「transcript 可见」充分条件；过滤/折叠机制本身为 PR 前遗留（show_unknown/show_unavailable 同样不可见），但「承诺未兑现」归本 PR。

## 修复要求（Request changes 依据）

(a) R2：Warning 打当前 turn_id；(b) R1：豁免 settled 折叠或改走 Report 独立渲染通道（flow_report_items/push_report_section turn-independent 常显）；(c) R3/R4：以「dispatcher 是否写过」标志位替代字符串相等，fallback 用 command_unavailable_reason 补命令名；(d) R8：收编外层三条 repro，渲染断言走 debug_render_text。

## 验证级别

static partially-verified：内环全程只读静态（未运行 cargo，按 brief 由外层统一跑 --all-targets+clippy+fmt）；全部动态证据引自外层 adversarial.log，未冒称自跑。R1/R2/R3 有外层动态复现支撑；R4-R8 为纯静态推演。

## 结论

Request changes：major×2（渲染不可见）+ minor×2 + info×3 + 测试缺口 1 组。修复 (a)(b) 前本 PR 实际交付为「用户在两个主路径场景看不见的 activity 行」，不应合入。
