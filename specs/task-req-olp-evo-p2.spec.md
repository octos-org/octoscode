spec: task
name: "进化环阶段 2:新 kind 采集、回放夹具与期望基线、指标脚本"
tags: [olp, evolution, harness, metrics, replay]
satisfies: [REQ-OLP-EVO-P2]
estimate: 1d
---

## 意图

补齐进化环的三块工具面:①采集哨识别 octos 阶段 2 新增的 `fallback_switch` 与
`malformed_exhausted` 事件;②一套来自真实 octos 战役、已脱敏的回放夹具与期望文件,把采集与
retro 的产出钉成可重复基线,让"进化"与"漂移"可区分;③指标脚本,把 §1 成功指标从人肉数变成一条
命令并支持与基线比对。不改运行时代码,不改协议,不写 ACK,不写任何真实状态目录。

## 已定决策

- 采集哨:`scripts/olp-evo-harvest.sh` events 分支的 kind 判定增加 `fallback_switch` →
  trigger `fallback_switch`、`malformed_exhausted` → trigger `malformed_exhausted`;identity、
  symptom、锚点规则与既有 events 触发器相同;retro 层提示表增加 `fallback_switch` →
  `Execution`、`malformed_exhausted` → `Tooling`。
- 回放夹具 `fixtures/evolution/replay/`:`review-board.md`(octos 活板 #45 段落节选,含
  ACK(blocked)/ACK(wontdo)/带署名改判与 R2 记档各至少一行)、`events.jsonl`(octos 实例
  events 节选,含 goal_transition blocked/budget_limited、escalation、turn_error、
  fallback_switch、malformed_exhausted 各至少一行,顶栏 goal_id 形状)、`mcp-board.md`
  (`~/.octos/outer/OUTER_LOOP_MCP.md` 节选,含 blocked/timeout 与非触发的 ask/answer/refusal)、
  `expected.json`。脱敏规则:绝对路径改为 `/repo/…` 或 `/home/u/…`,主机名与 hash 保留形状但
  改为固定假值,`question=` 之后正文替换为 `[redacted]`,不含任何 key/token 字样;夹具由主审从
  阶段 0/1 的影子采集材料制作并入库(见板上 43a),内环不得自造"真实"数据。
- `expected.json` 形状:`{"cards":N,"by_trigger":{"ack_blocked":n,…},"by_source":{"review":n,
  "events":n,"mcp":n},"candidates":N,"recurrence":{"<候选键>":<hint>,…}}`;回放测试把夹具复制到
  临时目录、以临时状态根运行 `olp-evo-harvest.sh` 与 `olp-evo-retro.sh`,解析进化黑板与简报,
  逐字段与 `expected.json` 比对。
- `scripts/olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json] [--baseline <json>]`:
  只读 `<repo>/.octos/EVOLUTION.md`(可由 `OLP_EVO_BOARD` 覆盖)与项目状态目录的
  `retro.json`/`retro/*.md`(可由 `OLP_EVO_STATE` 覆盖;缺失视为无 retro);解析用 python3 标准库。
  文本输出为固定行:`cards: N`、`by_trigger: k=v k=v …`(按 trigger 名排序)、
  `by_source: review=n events=n mcp=n`、`recurring_candidates: N`(最近一份简报中
  recurrence_hint ≥ 2 的候选数)、每个复发候选一行 `- <trigger> hint=<n> anchors=<a,b,…>`;
  `--json` 输出同内容的 JSON 对象(键 `cards`、`by_trigger`、`by_source`、`recurring_candidates`、
  `recurring`)。`--since EVO-NNNN` 只统计编号大于该值的卡。
- `--baseline <json>`:读取上一份 `--json` 输出,对 `override`、`r2_record`、`goal_blocked`、
  `goal_budget_limited`、`malformed_exhausted` 五类,当前计数 > 基线计数时输出
  `regress: <trigger> <base>-><now>`,其它类不比;退出码恒为 0;文本与 JSON 模式均输出。
- 零写入:回放测试与指标脚本不创建、不修改仓库内文件,状态根只用临时目录;指标脚本不创建任何文件。
- 测试:`tests/olp_evo_replay.rs`(回放)与 `tests/olp_evo_metrics.rs`(指标),Rust 集成测试,
  `std::process::Command` 调脚本,夹具复制到 `std::env::temp_dir()` 唯一子目录;采集哨新 kind 测试
  加在 `tests/olp_evo_harvest.rs`;retro 层表测试加在 `tests/olp_evo_retro.rs`。不新增 Cargo
  依赖;脚本只依赖 bash、coreutils、python3。

<!-- lint-ack: decision-coverage — 用法/输出格式等决策由多个场景共同行使,不单列场景 -->

## 边界

### Allowed Changes
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
- 不改阶段 0/1 已钉的触发器判定与简报格式(只新增 kind 与层表行)。
- 不向审查活板写入任何内容;不向真实 `~/.octos/outer/evo` 写入。
- 不新增 Cargo 依赖,不新增 MCP 工具。
- 回放夹具不得含真实凭据、用户正文、真实主机路径。

## 排除范围

- octos 侧发射点(octos `specs/task-olp-obs-p2-producers.spec.md`)。
- 采集挂外环 watch 节拍、规格直出契约、散文沉淀退役(阶段 3)。
- 指标脚本的"无 ACK 停摆"与"伪 verified"两项(需要活板与 events 跨源对账,阶段 3)。

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

场景: 回放夹具产出与期望一致(critical)
  标签: critical
  测试: olp_evo_replay_matches_expected
  假设 fixtures/evolution/replay/ 的三源样本与 expected.json
  当 对夹具副本运行 olp-evo-harvest.sh 与 olp-evo-retro.sh
  那么 卡片数、by_trigger、by_source、候选数与各候选 recurrence_hint 与 expected.json 逐字段相等

场景: 回放夹具已脱敏
  测试: olp_evo_replay_fixture_is_scrubbed
  假设 仓库检出
  当 扫描 fixtures/evolution/replay/ 全部文件
  那么 不含 /home/alexzhang、question=（后接非 [redacted] 正文）、api_key、token= 等字样

场景: 指标文本输出
  测试: olp_evo_metrics_text_output
  假设 一份含 5 张卡(ack_blocked×2、goal_blocked×2、turn_error×1)的进化黑板与一份含 goal_blocked hint=2 候选的 retro 简报
  当 运行 olp-evo-metrics.sh
  那么 stdout 含 cards: 5、by_trigger: ack_blocked=2 goal_blocked=2 turn_error=1、recurring_candidates: 1
  并且 含一行以 - goal_blocked hint=2 开头

场景: 指标 JSON 与 since
  测试: olp_evo_metrics_json_and_since
  假设 同上进化黑板,卡编号 EVO-0001 到 EVO-0005
  当 以 --json --since EVO-0003 运行 olp-evo-metrics.sh
  那么 stdout 为合法 JSON 且 cards 等于 2

场景: 基线比对标注回归
  测试: olp_evo_metrics_baseline_flags_regress
  假设 基线 JSON 中 goal_blocked 为 1,当前进化黑板中 goal_blocked 为 2,ack_blocked 由 3 降为 2
  当 以 --baseline 运行 olp-evo-metrics.sh
  那么 stdout 含 regress: goal_blocked 1->2
  并且 stdout 不含 regress: ack_blocked
  并且 退出码等于 0

场景: 指标脚本零写入
  测试: olp_evo_metrics_writes_nothing
  假设 任意进化黑板与状态目录
  当 运行 olp-evo-metrics.sh 前后分别记录仓库与状态目录全部文件的 sha256
  那么 两次记录逐文件相等且文件集合相同

场景: 缺少 retro 状态时仍可输出
  测试: olp_evo_metrics_without_retro_state
  假设 进化黑板存在但状态目录不存在
  当 运行 olp-evo-metrics.sh
  那么 退出码等于 0
  并且 stdout 含 recurring_candidates: 0
