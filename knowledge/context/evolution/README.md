# 缺陷记录目录(本地约定)

本目录是 LEP-003 进化环的缺陷记录面,**本地约定,非 canon 新类型**:
knowledge base 不因这些文件而新增类型或改协议,它们只是本仓库用来
沉淀"症状卡 → 归因 → 修复"的载体。权威来源(协议、REQ、提案)不变。

## 状态机

每份 `FLAW-*.md` 携带 frontmatter `status`,取值与迁移:

```
open → consolidated → filed → accepted → specified → patched → verified → closed
```

- 任意状态可转 `rejected`(判定不成立或不再复现)。
- `rejected` 仅在带**新卡片**(进化黑板新 EVO 卡支持同一指纹)时才可转
  `reopened`,回到 `open`。
- 状态只前进不后退(除 rejected/reopened 分支);跳步允许(小缺陷可
  `filed → patched` 直达),但每步都应有对应的证据落点。

## 文件

- `operators.md` — 操作面约定(operator 直改,不由 agent 触碰)。
- `FLAW-template.md` — 新记录模板(frontmatter 全字段)。
- `memory.md` — 记忆表:每条 FLAW 一行的结果速览。
- `FLAW-001.md` / `FLAW-002.md` — 已归档记录。

## retro(阶段 1)

`scripts/olp-evo-retro.sh <repo>` 在采集之后运行:读取上次 retro 之后的
卡,按"去数字/路径后的归一化文本"分组成候选,输出一份带记录草稿的
简报(位于状态目录 `retro/<UTC>-<run>.md`)。草稿到记录的转换规则:
简报里的 yaml 草稿标 `draft: true`、`fingerprint: TODO`,**TODO 未
消除前不得保存为 FLAW-NNN.md**;持主审锁的外环消 TODO(fingerprint、
severity、repo)并归并锚点后,才落 `FLAW-NNN.md` 并更新 `memory.md`。
每次 retro 最多推进 3 条记录;判断(归层、锚定、立案)始终由人做。

## 指标(阶段 2)

`scripts/olp-evo-metrics.sh <repo> [--since EVO-NNNN] [--json]
[--baseline <json>]`:窗口化诊断——窗口 = 编号大于 since 的卡(缺省全部;
给了基线时 since 取基线的 `through_evo`),用共享 lib 重新分组。
**诊断非 KPI**:计数上升可能是检测变好,不是回归,输出固定带
`note: diagnostic only, not a KPI` 且不含 `regress` 字样;基线用法是
`--baseline <上一份 --json 输出>`,对每个 trigger 输出
`increase:`/`decrease:` 增减行(相等不输出),供人判读。只读,不写文件。

## kind 候选

采集面只收已定式的触发器;观察到的**新摩擦类别**先在此登记为 kind 候选,
等主审裁决是否进契约(不自行扩 harvest 触发面)。当前登记:

- `iteration_cap` —— 内环单 turn 迭代上限耗尽(超限静默收工,外观像
  "卡住";出处 2026-09-05 octos 活板 #48"48b 中断记录")。
- `patch_failed` —— 修复片打回后未再派发即关闭(goal 以 blocked 收尾,
  缺陷未消;同上出处)。

## 索引

`scripts/olp-evo-index.sh <repo>` 扫描本目录 `FLAW-*.md` 与
`docs/OUTER_LOOP_PROTOCOL.md` 的 `> 已记录:FLAW-NNN` 行,生成
`INDEX.md`(首行标注"生成,勿手改";末行 `retired_prose: N` 记被
PROTOCOL 散文取代的 FLAW 数)。内容相同不写,幂等且保 mtime。
