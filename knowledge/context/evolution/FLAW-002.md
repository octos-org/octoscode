---
kind: context
id: FLAW-002
title: "goal_create 把 archived 当未完成拒绝,准入判定只认 complete"
repo: octos-org/octos
layers: [Lifecycle, Verification]
status: filed
severity: S2
recurrence: 1
fingerprint: lifecycle/goal-create-admission-rejects-archived
issue: https://github.com/octos-org/octos/issues/2237
cards: []
filed: 2026-09-04
---

## 症状

会话上一个 goal 处于 `archived` 时,`goal_create` 报
`cannot create a new goal because this session has an unfinished goal (status \`archived\`)`;
`goal_update` 也拒绝("stale verdict")。master 建不了 goal,只能裸驱 peer,收工时
无法 `/goal stop`;TUI 头部把 archived 的 goal 当当前 goal 显示。

## 责任步

master 上岗第③步 `goal_create`:supervisor 权威层显示 goal_01–04 全部非 active,
会话侧 goal 卡态仍为 archived,准入判定按卡态拒绝。

## 根因

`crates/octos-cli/src/autonomy/agent_orchestrator.rs` L8992(`model_create_goal`):
`if existing.status != "complete" { return Err(...) }`。而 `archived` 在同一文件里被当作
终态:L2250 把 `archive` 转移映射为 `"archived"`,L2318/L2336 视其为终态先验,
L14333 映射为 `GroupStatus::Completed`。判定写的是"是否 complete",语义应是
"是否未完成"。

## 锚点

- `crates/octos-cli/src/autonomy/agent_orchestrator.rs` L8975–L8998(准入)
- `crates/octos-cli/src/autonomy/agent_orchestrator.rs` L2250、L2318、L2336、L14333(archived 终态语义)

## 复发史

- 2026-09-04,octos 活板 #45,实例 `f182cc95436c22f7`,会话 `octos:local:tui#coding`。
- 近亲:活板 #43(离线 archive 被 live cache 反盖),已在线化。

## 保护门

- `active` / `blocked` / `paused` 仍拒绝创建(不变)。
- 替换已完成 goal 必须铸新 goal id(同函数 Fix B)对 `archived` 同样适用。

## 异议

(第二外环署名批注,只追加)
