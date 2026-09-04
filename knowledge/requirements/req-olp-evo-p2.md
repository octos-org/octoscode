---
kind: requirement
id: REQ-OLP-EVO-P2
title: "进化环阶段 2:新 kind 采集、合成回放夹具与期望基线、窗口化指标"
status: accepted
liveness: auto
tags: [olp, evolution, harness, metrics, replay]
---

## Problem

阶段 0/1 让采集与 retro 机械化,但三处仍缺:①octos 侧新增的 `fallback_switch` 与 `malformed_exhausted` 事件(REQ-OLP-OBS 修订)采集哨与 retro 要认;②"进化"与"漂移"无法区分,因为没有可重复的基线:需要一套结构保持、由 allowlist 合成(不含任何真实自由文本)的回放夹具,把采集与 retro 的产出钉成期望值;③没有指标脚本,而指标一旦把累计计数上涨定义为回归,就会奖励少报——指标必须窗口化、只作诊断。

## Requirements

[REQ-OLP-EVO-P2-KINDS] 采集哨 MUST 把 events.jsonl 中 `kind` 为 `fallback_switch` 与 `malformed_exhausted` 的行分别识别为同名触发器,identity 与 symptom 规则与既有 events 触发器相同。

[REQ-OLP-EVO-P2-GROUP] retro 与指标脚本 MUST 把 `fallback_switch` 与 `malformed_exhausted` 视为 events 类:分组文本取 symptom JSON 的 `detail` 字段。

[REQ-OLP-EVO-P2-LAYER] retro 层提示表 MUST 增加 `fallback_switch` → `Execution`、`malformed_exhausted` → `Tooling`。

[REQ-OLP-EVO-P2-ANCHOR] `fallback_switch` 卡的锚点 MUST 为 `<session>|<from_provider>-><to_provider>`(from/to 取自 detail),使同一会话内不同车道切换各计一次。

[REQ-OLP-EVO-P2-LIB] 归一化、锚点与卡片解析 MUST 抽成单一 python 模块 `scripts/olp-evo-lib.py`,由 retro 与指标脚本共同调用,MUST NOT 各自复制实现。

[REQ-OLP-EVO-P2-REPLAY] `fixtures/evolution/replay/` MUST 由主审以 allowlist 合成:每一行只由固定假值(session、goal、slug、host、path、provider、ask id、reason 枚举句)与真实行形拼成,MUST NOT 含任何来自真实战役的自由文本;`expected.json` MUST 钉住采集后的卡片数、按 trigger 与来源的计数、候选数与每候选的 recurrence_hint,实现 commit MUST NOT 修改 `expected.json`。

[REQ-OLP-EVO-P2-REPLAY-TEST] 回放测试 MUST 对夹具副本运行采集哨与 retro 并与 `expected.json` 逐字段比对;脱敏测试 MUST 断言每一行匹配 allowlist 行形之一,且不含高风险模式(`/Users/`、非 `/home/u` 的 `/home/` 路径、`sk-`、`ghp_`、`AKIA`、`Authorization`、`Bearer`、邮箱、IPv4、`token[=:]`、`api[_-]?key`、非占位的 `instances/<hash>`)。

[REQ-OLP-EVO-P2-METRICS] `scripts/olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json]` MUST 只读进化黑板,对编号大于 `since`(缺省 0)的窗口用 `olp-evo-lib.py` 重新分组,输出 `through_evo`(窗口内最大卡编号)、卡片数、按 trigger 计数、按来源计数、窗口内 recurrence_hint ≥ 2 的候选数与列表;MUST NOT 读取或依赖 retro 简报,MUST NOT 写入任何文件。

[REQ-OLP-EVO-P2-METRICS-BASELINE] `--baseline <json>` MUST 以基线的 `through_evo` 作为本次窗口起点,对**所有** trigger 输出 `increase: <trigger> <base>-><now>` 或 `decrease:` 诊断行,MUST 打印固定说明行 `note: diagnostic only, not a KPI; rising counts may mean better detection`,MUST NOT 输出 `regress:` 字样,退出码恒为 0。

[REQ-OLP-EVO-P2-NOWRITE] 回放测试与指标脚本 MUST NOT 修改仓库内任何文件,也 MUST NOT 向真实 `~/.octos/outer/evo` 写入。

## Scenarios

Scenario: 新 kind 各落一卡并按 detail 分组
  Given events.jsonl 含 fallback_switch 与 malformed_exhausted 各一行
  When 运行采集哨与 retro
  Then 进化黑板恰新增两张卡,retro 两候选的 key 分别含 `router failover` 与 `feedback_limit`

Scenario: 同会话不同车道切换各计一次
  Given 两张 fallback_switch 卡,session 相同,detail 分别为 a -> b 与 b -> c
  When 运行 retro
  Then 该候选的 recurrence_hint 等于 2

Scenario: 回放夹具产出与期望一致
  Given fixtures/evolution/replay/ 的合成三源样本与 expected.json
  When 对夹具副本运行采集哨与 retro
  Then 卡片数、按 trigger 计数、按来源计数、候选数与各候选 recurrence_hint 与 expected.json 逐字段相等

Scenario: 指标窗口与基线诊断
  Given 基线 JSON 的 through_evo 为 5,当前进化黑板有 EVO-0001 到 EVO-0008,其中 6 到 8 含 goal_blocked×2、ack_blocked×1
  When 以 --baseline 运行指标脚本
  Then 输出 cards: 3、through_evo: 8、一行 `increase: goal_blocked`、固定 note 行
  And 输出不含 `regress:`,退出码等于 0,仓库与状态目录无文件被修改

## Dependencies

- REQ-OLP-EVO、REQ-OLP-EVO-RETRO(卡片、状态目录、retro 简报)
- REQ-OLP-OBS(events.jsonl kind 白名单,本阶段修订新增两 kind)

## Source Trace

- proposal:LEP-003 §10 阶段 2
- operator 2026-09-05 直令"按阶段完成这个目标"
- octos 侧发射点契约:octos 仓库 `specs/task-olp-obs-p2-producers.spec.md`(分支 feat/olp-obs-p2-producers)
- 契约 v2 经 codex/grok 对抗复审(2026-09-05):累计计数当回归会奖励少报;黑名单脱敏不可证明;since 窗口与最近简报混用;新 kind 需进 events 类分组;fallback 锚点需含车道
- 实测:阶段 0 影子采集 7 卡、阶段 1 干跑 4 候选,尚无可重复的期望基线

## Open Questions

- 是否把"无 ACK 停摆"与"伪 verified"两项跨源指标提前到阶段 2 末(需活板与 events 对账,暂留阶段 3)。

## Next

Single exit: compile this requirement into a task contract(`specs/task-req-olp-evo-p2.spec.md`)。
