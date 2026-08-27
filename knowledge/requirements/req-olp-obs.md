---
kind: requirement
id: REQ-OLP-OBS
title: "外环观测面:只读状态 CLI、结构化事件流、稳定寻址"
status: accepted
liveness: auto
tags: [olp, observability, octos]
---

## Problem

外环模型今天靠"翻 per-instance 目录 + 自算跨版本不稳定的 session hash +
grep 人类日志"观测内环,三件套全部脆弱(实测踩坑:日志按进程启动日期
滚动、DefaultHasher 上游自认不稳定、ugrep 缓冲吞事件)。外环需要机器
可读、契约化的观测入口。

## Requirements

[REQ-OLP-OBS-JSON] `octos` MUST 提供只读子命令 `goal status --json`、
`peer list --json`、`ledger tail <goal_id> --json`,直读数据目录,
在 serve 进程不存活时同样可用。

[REQ-OLP-OBS-EVENTS] serve MUST 向 `<data_dir>/events.jsonl` 追加结构化
事件行(字段:ts、kind、goal_id?、slug?、session?、model_lane?、detail),
kind 至少覆盖 peer_staged、finding_recorded、escalation、goal_transition、
steer_consumed、turn_error。

[REQ-OLP-OBS-ADDR] `octos` MUST 提供 `inbox path --session <key>` 查询
命令,返回该 session 的 inbox 文件路径;外部消费者 MUST NOT 需要自行
实现哈希算法。

[REQ-OLP-OBS-LANE] peer_staged 事件 MUST 携带该 peer 解析后的 model
lane(未指定时为 primary),使外环可审计成本分布。

## Scenarios

Scenario: serve 停止时仍可读 goal 状态
  Given 一个含已完成 goal_02 账本的数据目录且 serve 未运行
  When 执行 octos goal status --goal goal_02 --json
  Then 输出合法 JSON 且 status 字段为 "complete"

Scenario: peer 交付产生结构化事件
  Given 一个 goal-scoped peer 完成一个 turn
  When finding 写入账本
  Then events.jsonl 追加一行 kind=finding_recorded 且含 goal_id 与 slug

Scenario: 外部进程查询 inbox 路径
  Given 会话 key octos:local:tui#coding
  When 执行 octos inbox path --session octos:local:tui#coding
  Then 输出路径与 serve 实际读写的 notes 文件一致

## Dependencies

None.

## Source Trace

- proposal:LEP-001(§3 A1/A2/A3/D2;operator 2026-08-22 拍板 A3 取
  "只加查询命令,零迁移")
- 实测:外环监控两次失效(日志滚动、ugrep 缓冲),见
  docs/OUTER_LOOP_PROTOCOL.md 已知局限。

## Open Questions

None.

## Next

Single exit: compile this requirement into a task contract with
`agent-spec requirements draft-specs`.
