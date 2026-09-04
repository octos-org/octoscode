# 回放夹具(replay)

来源:2026-09-04 octos 活板 #45 战役(`.octos/OUTER_LOOP_REVIEW.md` 节选)、实例 events.jsonl 节选、
`~/.octos/outer/OUTER_LOOP_MCP.md` 节选,由主审脱敏后入库(路径改 `/home/u`、`/repo/<仓库>`、
实例 hash 改 0…;`question=`/`context=`/`tried=` 正文改 `[redacted]`;无凭据字样)。

历史里不存在的触发器种类(ACK(blocked)/ACK(wontdo)、escalation、turn_error、fallback_switch、
malformed_exhausted、goal_budget_limited)以**标注 `[synthetic]`** 的行按真实形状补齐,其余为真实行。

`expected.json` 由主审用 43-r1(1ee05ea)后的 `olp-evo-harvest.sh` + `olp-evo-retro.sh --dry-run`
对本夹具计算得出(15 卡、13 候选);回放测试逐字段比对,任何偏差都是采集/retro 行为的变化。
