---
kind: requirement
id: REQ-OLP-EVO-P2
title: "进化环阶段 2:新 kind 采集、回放夹具、指标脚本"
status: accepted
liveness: auto
tags: [olp, evolution, harness, metrics, replay]
---

## Problem

阶段 0/1 让采集与 retro 机械化,但三处仍缺:①octos 侧新增的 `fallback_switch` 与 `malformed_exhausted` 事件(REQ-OLP-OBS 修订)采集哨还不认;②"进化"与"漂移"无法区分,因为没有可重复的基线:需要一套来自真实战役、已脱敏的回放夹具,把采集与 retro 的产出钉成期望值;③没有指标脚本,§1 的成功指标(无 ACK 停摆、伪 verified、改判、同指纹复发)只能人肉数。

## Requirements

[REQ-OLP-EVO-P2-KINDS] 采集哨 MUST 把 events.jsonl 中 `kind` 为 `fallback_switch` 的行识别为触发器 `fallback_switch`,`kind` 为 `malformed_exhausted` 的行识别为 `malformed_exhausted`,identity 与 symptom 规则与既有 events 触发器相同。

[REQ-OLP-EVO-P2-LAYER] retro 层提示表 MUST 增加 `fallback_switch` → `Execution`、`malformed_exhausted` → `Tooling`。

[REQ-OLP-EVO-P2-REPLAY] `fixtures/evolution/replay/` MUST 含一套来自真实 octos 战役、已脱敏(无凭据、无用户正文、路径改为占位)的三源样本(活板片段、events.jsonl 片段、MCP 审计板片段)与一份期望文件 `expected.json`,期望文件 MUST 钉住采集后的卡片数、按 trigger 的计数、retro 的候选数与每候选的 recurrence_hint。

[REQ-OLP-EVO-P2-REPLAY-TEST] 回放测试 MUST 对夹具运行采集哨与 retro 脚本并与 `expected.json` 逐字段比对,任一偏差即失败。

[REQ-OLP-EVO-P2-METRICS] `scripts/olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json]` MUST 只读进化黑板与 retro 状态,输出:卡片总数、按 trigger 计数、按来源计数、retro 候选中 recurrence_hint ≥ 2 的候选数、按 goal/条目锚点去重后的复发候选列表;MUST NOT 写入任何文件。

[REQ-OLP-EVO-P2-METRICS-BASELINE] 指标脚本 MUST 支持 `--baseline <json>`:与上一份指标 JSON 比对,对 `override`、`r2_record`、`goal_blocked`、`goal_budget_limited`、`malformed_exhausted` 五类计数上升的情况在 stdout 标注 `regress:`,退出码仍为 0。

[REQ-OLP-EVO-P2-NOWRITE] 回放测试与指标脚本 MUST NOT 修改仓库内任何文件,也 MUST NOT 向真实 `~/.octos/outer/evo` 写入。

## Scenarios

Scenario: 新 kind 各落一卡
  Given events.jsonl 含 fallback_switch 与 malformed_exhausted 各一行
  When 运行 olp-evo-harvest.sh
  Then 进化黑板恰新增两张卡,trigger 分别为 fallback_switch 与 malformed_exhausted

Scenario: 回放夹具产出与期望一致
  Given fixtures/evolution/replay/ 的三源样本与 expected.json
  When 对夹具运行采集哨与 retro
  Then 卡片数、按 trigger 计数、候选数与各候选 recurrence_hint 与 expected.json 逐字段相等

Scenario: 指标脚本输出与基线比对
  Given 一份进化黑板与一份基线指标 JSON,其中 goal_blocked 计数比基线多 1
  When 以 --baseline 运行 olp-evo-metrics.sh
  Then stdout 含 `regress: goal_blocked`
  And 退出码等于 0
  And 仓库与状态目录无任何文件被修改

## Dependencies

- REQ-OLP-EVO、REQ-OLP-EVO-RETRO(卡片、状态目录、retro 简报)
- REQ-OLP-OBS(events.jsonl kind 白名单,本阶段修订新增两 kind)

## Source Trace

- proposal:LEP-003 §10 阶段 2
- operator 2026-09-05 直令"按阶段完成这个目标"
- octos 侧发射点契约:octos 仓库 `specs/task-olp-obs-p2-producers.spec.md`(分支 feat/olp-obs-p2-producers,27d9799f)
- 实测:阶段 0 影子采集 7 卡、阶段 1 干跑 4 候选,尚无可重复的期望基线

## Open Questions

- 回放夹具的脱敏规则是否需要 canon 化(阶段 3 与散文沉淀退役一并考虑)。

## Next

Single exit: compile this requirement into a task contract(`specs/task-req-olp-evo-p2.spec.md`)。
