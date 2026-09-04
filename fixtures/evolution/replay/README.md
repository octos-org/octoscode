# 进化环回放夹具(阶段 2,REQ-OLP-EVO-P2-REPLAY)

由主审(外环)以 **allowlist 合成**:每一行只由固定假值与真实行形拼成,不含任何来自真实战役的自由文本。

| 假值 | 取值 |
|---|---|
| session | `octos:local:tui#coding` |
| goal / slug | `goal_01..goal_09` / `p1..p9` |
| host / 路径 | `host-a` / `/repo/octos`、`/home/u/.octos/instances/0000000000000000` |
| provider | `lane-a` / `lane-b` / `lane-c` |
| ask id | `a1b2c3d4e5f6…` |
| reason 枚举句 | `inner stuck on step 3`、`waiting for outer decision`、`quota exhausted` |
| `question=`/`context=`/`tried=` | 一律 `[redacted]` |

文件:`review-board.md`(活板)、`events.jsonl`(实例事件流)、`mcp-board.md`(MCP 问外环板)、
`expected.json`(采集 + `retro --dry-run` 后的期望:cards、by_trigger、by_source、candidates、每候选 recurrence_hint)。

规则:实现 commit 不得改动本目录;`expected.json` 由主审用契约完成后的脚本计算并入库。
回放测试见 `tests/olp_evo_replay.rs`。
