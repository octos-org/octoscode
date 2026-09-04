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
