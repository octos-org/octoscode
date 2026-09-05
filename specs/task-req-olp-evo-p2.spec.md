spec: task
name: "进化环阶段 2:新 kind 采集、共享 lib、合成回放夹具与期望基线、窗口化指标"
tags: [olp, evolution, harness, metrics, replay]
satisfies: [REQ-OLP-EVO-P2]
estimate: 1.5d
---

## 意图

补齐进化环的三块工具面:①采集哨与 retro 识别 octos 阶段 2 新增的 `fallback_switch` 与
`malformed_exhausted`(已在 43a/43-r1 落地),并把 fallback 锚点细化到车道;②归一化/锚点/解析
抽成共享 python 模块,retro 与指标共用;③一套由主审以 allowlist 合成、结构保持的回放夹具与
`expected.json`,把采集与 retro 的产出钉成可重复基线;④窗口化的指标脚本,只作诊断、不作 KPI、
不把累计计数上涨称为回归。不改运行时代码,不改协议,不写 ACK,不写真实状态目录。契约 v2 已并入
codex 与 grok 对抗复审。

## 已定决策

- 共享模块 `scripts/olp-evo-lib.py`(python3 标准库,无 CLI 入口):导出 `parse_cards(text)`(返回
  卡片列表:id、trigger、source、identity、envelope、symptom、raw)、`group_text(card)`(按 trigger
  取分组文本:活板类剥定式前缀;events 类含 `escalation`、`turn_error`、`goal_blocked`、
  `goal_budget_limited`、`fallback_switch`、`malformed_exhausted` 取 JSON `detail`,其中 `fallback_switch`
  的分组文本再把 `(\S+) -> (\S+)` 归一为 `<lane> -> <lane>` 并删去 `, \d+ms`(同会话不同车道的切换同组、
  靠锚点区分复发);MCP 类取
  `reason=` 之后)、`normalize(text)`(阶段 1 规则:lower → `<path>` → `<hex>` → `<num>`,数字规则
  `(?<![A-Za-z_\d])\d+(?![A-Za-z\d])` → 空白折叠 → 80 code point)、`anchor(card)`(rsplit 规则;
  `fallback_switch` 返回 `<session>|<from>-><to>`,**session 取 symptom JSON 的 `session` 字段全文**(可含
  `#`,如 `octos:local:tui#coding`),JSON 缺 session 时才退回 identity 解析——identity 形如 `events:<path>#<ts>#<kind>#<ref>#<sha>`,path/ts/kind/sha 均不含 `#`,故 **ref = 第 3 个 `#` 之后到最后一个 `#` 之前的全部文本**(可含 `#`,如 `tenant-a:local:tui#coding`),不得用 `rsplit("#")[-2]`;from/to 用正则
  `router failover: (\S+) -> (\S+)` 从 detail 取,取不到退回 session;锚点 `-` 或空退回 `EVO-NNNN`)、`layer(trigger)`(阶段 1 表 +
  两新行)、`group(cards)`(返回候选列表:key、trigger、layer、anchors、recurrence_hint、cards)。
  `scripts/olp-evo-retro.sh` 的内嵌 python 改为 `import` 该模块(通过 `sys.path.insert(0, 脚本目录)`),
  行为与阶段 1 契约一致;不得在 retro 与 metrics 各自复制实现。
- 回放夹具 `fixtures/evolution/replay/`(**主审入库**,内环不得改动其内容,实现 commit 不得改
  `expected.json`):`review-board.md`、`events.jsonl`、`mcp-board.md`、`expected.json`、`README.md`。
  夹具每一行只由 allowlist 假值拼成:session `octos:local:tui#coding`、goal `goal_01..goal_09`、
  slug `p1..p9`、host `host-a`、路径 `/repo/octos`、`/home/u/.octos/instances/0000000000000000`、
  provider `lane-a`/`lane-b`/`lane-c`、ask id `a1b2c3d4e5f6`、reason 枚举句(`inner stuck on step 3`、
  `waiting for outer decision`、`quota exhausted`)、ACK 正文枚举句;`question=`/`context=`/
  `tried=` 一律 `[redacted]`。`expected.json` 形状:`{"cards":N,"by_trigger":{…},"by_source":{"review":n,
  "events":n,"mcp":n},"candidates":N,"recurrence":{"<候选键>":<hint>,…}}`,由主审用本契约完成后的
  脚本计算并入库。
- 回放测试 `tests/olp_evo_replay.rs`:把夹具复制到临时目录,`OLP_EVO_STATE` 指向临时状态根,依次
  运行 `olp-evo-harvest.sh` 与 `olp-evo-retro.sh --dry-run`,解析进化黑板与简报,逐字段与
  `expected.json` 比对;脱敏测试对夹具每一行断言匹配 allowlist 行形之一(正则集合写在测试内),并断言
  不含高风险模式:`/Users/`、`/home/(?!u/)`、`sk-`、`ghp_`、`AKIA`、`Authorization`、`Bearer`、
  邮箱 `\S+@\S+\.\S+`、IPv4、`token[=:]`、`api[_-]?key`、`instances/(?!0{16})[0-9a-f]{16}`。
- 指标脚本 `scripts/olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json] [--baseline <json>]`:
  只读 `<repo>/.octos/EVOLUTION.md`(可由 `OLP_EVO_BOARD` 覆盖),不读 retro 状态,不写任何文件;
  窗口 = 编号 > since 的卡(缺省 0;给了 `--baseline` 时 since 取基线的 `through_evo`);用 lib
  重新分组。文本输出固定行:`note: diagnostic only, not a KPI; rising counts may mean better detection`、
  `through_evo: EVO-NNNN`(窗口内最大编号,无卡时 `EVO-0000`)、`cards: N`、`by_trigger: k=v …`(按
  名排序)、`by_source: review=n events=n mcp=n`、`recurring_candidates: N`、每个复发候选一行
  `- <trigger> hint=<n> anchors=<a,b,…>`;`--baseline` 时对所有 trigger 输出 `increase:`/`decrease:`
  行(`<trigger> <base>-><now>`,相等不输出);`--json` 输出同内容对象(键 `note`、`through_evo`、
  `cards`、`by_trigger`、`by_source`、`recurring_candidates`、`recurring`、`deltas`)。退出码恒 0。
- README(`knowledge/context/evolution/README.md`)加"指标"一段:窗口语义、诊断非 KPI、基线用法。
- 零写入包含脚本目录:所有调用 python 的入口(retro、metrics)用 `python3 -B` 或在 import 前设
  `sys.dont_write_bytecode = True`,不得在 `scripts/` 下产生 `__pycache__`;零写入测试对**脚本目录副本**与
  仓库夹具目录、状态目录三处前后比对文件集合与 sha256。
- 契约 v3.1(2026-09-05,codex 第二轮复审):identity 回退按位置解析 ref 段。契约 v3(2026-09-05,codex 复审后):以上三条为修订;`expected.json` 由主审在实现落地后重算入库。
- 测试:`tests/olp_evo_metrics.rs`;新 kind 锚点测试加在 `tests/olp_evo_retro.rs`;不新增 Cargo
  依赖;脚本只依赖 bash、coreutils、python3。

<!-- lint-ack: decision-coverage — 用法/输出格式等决策由多个场景共同行使,不单列场景 -->

## 边界

### Allowed Changes
- scripts/olp-evo-lib.py
- scripts/olp-evo-harvest.sh
- scripts/olp-evo-retro.sh
- scripts/olp-evo-metrics.sh
- tests/olp_evo_harvest.rs
- tests/olp_evo_retro.rs
- tests/olp_evo_replay.rs
- tests/olp_evo_metrics.rs
- fixtures/evolution/**
- knowledge/context/evolution/README.md

### Forbidden
- 不改 `src/**` 任何运行时代码。
- 不改 `AGENTS.md`、`.octos/loop.md`、`.claude/skills/**`、`docs/**`、`tests/olp_contract.rs`。
- 不改阶段 0/1 已钉的触发器判定、简报格式与阶段 1 测试断言(只允许新增)。
- 实现 commit 不得修改 `fixtures/evolution/replay/expected.json` 与其它回放夹具文件。
- 不向审查活板写入任何内容;不向真实 `~/.octos/outer/evo` 写入;指标脚本不创建任何文件。
- 不新增 Cargo 依赖,不新增 MCP 工具。
- 指标输出不得含 `regress` 字样。

## 排除范围

- octos 侧发射点(octos `specs/task-olp-obs-p2-producers.spec.md`)。
- 采集挂外环 watch 节拍、规格直出契约、散文沉淀退役(阶段 3)。
- "无 ACK 停摆"与"伪 verified"两项跨源指标(阶段 3)。

## 完成条件

场景: 新 kind 各落一卡(critical)
  标签: critical
  测试: olp_evo_harvest_new_kinds_fallback_switch_and_malformed_exhausted
  假设 events.jsonl 含 kind 为 fallback_switch 与 malformed_exhausted 各一行,其它来源无新触发行
  当 运行 olp-evo-harvest.sh
  那么 进化黑板恰新增两张卡
  并且 两张卡的 trigger 行分别为 fallback_switch 与 malformed_exhausted

场景: 未知 kind 不落卡
  测试: olp_evo_harvest_unknown_kind_does_not_trigger
  假设 events.jsonl 含一行 kind 为 peer_staged 与一行 kind 为 steer_consumed
  当 运行 olp-evo-harvest.sh
  那么 进化黑板中以 ### EVO- 开头的行数等于 0

场景: retro 层表覆盖新 kind
  测试: olp_evo_retro_layer_for_new_kinds
  假设 进化黑板含 fallback_switch 与 malformed_exhausted 各一张卡
  当 运行 olp-evo-retro.sh
  那么 简报中两候选行分别含 layer=Execution 与 layer=Tooling

场景: 同会话不同车道切换各计一次(critical)
  标签: critical
  测试: olp_evo_retro_fallback_anchor_includes_lanes
  假设 进化黑板含两张 fallback_switch 卡,symptom JSON 的 session 均为 octos:local:tui#coding,detail 分别为 router failover: lane-a -> lane-b (quota exhausted, 1200ms) 与 router failover: lane-b -> lane-c (quota exhausted, 900ms)
  当 运行 olp-evo-retro.sh --dry-run
  那么 简报含 candidates: 1
  并且 该候选行含 recurrence_hint=2 且 anchors 行含 octos:local:tui#coding|lane-a->lane-b 与 octos:local:tui#coding|lane-b->lane-c

场景: 不同会话同后缀不合并
  测试: olp_evo_retro_fallback_anchor_uses_full_session
  假设 两张 fallback_switch 卡 detail 相同,symptom JSON 的 session 分别为 tenant-a:local:tui#coding 与 tenant-b:local:tui#coding
  当 运行 olp-evo-retro.sh --dry-run
  那么 简报含 candidates: 1 且该候选行含 recurrence_hint=2

场景: identity 回退也保留完整会话
  测试: olp_evo_retro_fallback_anchor_identity_fallback_keeps_full_session
  假设 两张 fallback_switch 卡 detail 相同,symptom JSON 均无 session 字段,identity 的 ref 段分别为 tenant-a:local:tui#coding 与 tenant-b:local:tui#coding
  当 运行 olp-evo-retro.sh --dry-run
  那么 简报含 candidates: 1 且该候选行含 recurrence_hint=2 且 anchors 行含 tenant-a:local:tui#coding|

场景: 缺 session 的 fallback 卡退回 EVO 编号
  测试: olp_evo_retro_fallback_anchor_falls_back_to_evo_id
  假设 一张 fallback_switch 卡的 symptom JSON 无 session 字段且 identity 的会话段为 -
  当 运行 olp-evo-retro.sh --dry-run
  那么 该候选的 anchors 行含 EVO-0001

场景: retro 与指标共用 lib 且结果一致
  测试: olp_evo_lib_shared_by_retro_and_metrics
  假设 一份含 5 张卡的进化黑板
  当 分别运行 olp-evo-retro.sh --dry-run 与 olp-evo-metrics.sh --json
  那么 两者的候选数相等且每候选的 recurrence_hint 相等
  并且 olp-evo-retro.sh 与 olp-evo-metrics.sh 均含 import 语句引用 olp_evo_lib

场景: 回放夹具产出与期望一致(critical)
  标签: critical
  测试: olp_evo_replay_matches_expected
  假设 fixtures/evolution/replay/ 的合成三源样本与 expected.json
  当 对夹具副本运行 olp-evo-harvest.sh 与 olp-evo-retro.sh --dry-run
  那么 卡片数、by_trigger、by_source、候选数与各候选 recurrence_hint 与 expected.json 逐字段相等

场景: 回放夹具符合 allowlist 且无高风险模式
  测试: olp_evo_replay_fixture_matches_allowlist
  假设 仓库检出
  当 逐行扫描 fixtures/evolution/replay/ 的三源文件
  那么 每一行匹配 allowlist 行形之一
  并且 不含 /Users/、非 /home/u 的 /home/ 路径、sk-、ghp_、AKIA、Authorization、Bearer、邮箱、IPv4、token=、api_key、非占位 instances hash

场景: 指标文本输出
  测试: olp_evo_metrics_text_output
  假设 一份含 5 张卡(ack_blocked×2、goal_blocked×2 且 goal 不同、turn_error×1)的进化黑板
  当 运行 olp-evo-metrics.sh
  那么 stdout 含 note: diagnostic only、through_evo: EVO-0005、cards: 5、by_trigger: ack_blocked=2 goal_blocked=2 turn_error=1、recurring_candidates: 1
  并且 含一行以 - goal_blocked hint=2 开头

场景: 指标 JSON 与 since 窗口
  测试: olp_evo_metrics_json_and_since_window
  假设 同上进化黑板,卡编号 EVO-0001 到 EVO-0005,goal_blocked 两卡为 EVO-0004 与 EVO-0005
  当 以 --json --since EVO-0003 运行 olp-evo-metrics.sh
  那么 stdout 为合法 JSON 且 cards 等于 2、recurring_candidates 等于 1、through_evo 等于 EVO-0005

场景: 基线窗口诊断不含 regress
  测试: olp_evo_metrics_baseline_window_diagnostics
  假设 基线 JSON 的 through_evo 为 EVO-0005 且 by_trigger 中 goal_blocked 为 1,当前进化黑板 EVO-0006 到 EVO-0008 含 goal_blocked×2、ack_blocked×1
  当 以 --baseline 运行 olp-evo-metrics.sh
  那么 stdout 含 cards: 3、through_evo: EVO-0008、increase: goal_blocked 1->2、note: diagnostic only
  并且 stdout 不含 regress
  并且 退出码等于 0

场景: 指标脚本零写入(含脚本目录)
  测试: olp_evo_metrics_writes_nothing
  假设 scripts/ 复制到无 __pycache__ 的临时目录,任意进化黑板与状态目录
  当 用该副本运行 olp-evo-metrics.sh 前后分别记录脚本副本目录、仓库夹具目录与状态目录全部文件的 sha256
  那么 三处记录逐文件相等且文件集合相同

场景: retro 不产生字节码缓存
  测试: olp_evo_retro_writes_no_pycache
  假设 scripts/ 复制到无 __pycache__ 的临时目录
  当 用该副本运行 olp-evo-retro.sh --dry-run
  那么 副本目录下不存在 __pycache__

场景: 无卡时指标仍可输出
  测试: olp_evo_metrics_empty_board
  假设 进化黑板不存在
  当 运行 olp-evo-metrics.sh
  那么 退出码等于 0
  并且 stdout 含 cards: 0 与 through_evo: EVO-0000
