---
kind: requirement
id: REQ-OLP-EXEC
title: "执行硬化:peer 工具链、默认隔离、机制化验证"
status: accepted
liveness: auto
tags: [olp, peer, verification, octos]
---

## Problem

实测三类执行可信度缺陷:peer 的 shell 缺 cargo 只能交付"未验证"结果;
多写者共用工作区仅靠 AGENTS.md 纪律兜底;内环两次以 lib-only 测试的
绿色失实声称"已验证"(CI 实际编译失败)。验证必须从纪律降为机制。

## Requirements

[REQ-OLP-EXEC-PATH] peer 与 master 的工具 shell MUST 继承 operator 的
PATH,或使用 profile 显式配置的 `tool_path`;两者皆缺时 MUST 在 result
中声明工具链不可用。

[REQ-OLP-EXEC-ISOLATE] `peer_handoff` 的 worktree 缺省 MUST 可由 profile
配置;开启后 peer 结束(closed 且成果已被 gather)时 runtime MUST 自动
清理其 `wt/` 克隆(operator 2026-08-22 拍板:默认开 + 完成即清)。

[REQ-OLP-EXEC-VERIFY] profile MUST 支持 `verify_command` 配置;goal-scoped
peer 交付时 runtime MUST 执行该命令并把结果(pass|fail|skipped 及原因)
写入 result.md frontmatter 的 `verified:` 字段与 goal 账本。

[REQ-OLP-EXEC-VERIFY-SRC] `verify_command` MUST 仅来自 operator 手写的
profile 配置;模型工具、黑板、steer 通道 MUST NOT 能写入或修改该字段。

## Scenarios

Scenario: peer 工具链继承
  Given operator 的 PATH 含 cargo 且 profile 未配 tool_path
  When peer 执行 bash 工具运行 cargo --version
  Then 命令成功且输出版本号

Scenario: 交付触发机制化验证并落账
  Given profile 配置 verify_command 为 cargo test --all-targets
  When 一个 goal-scoped peer 完成交付
  Then result.md frontmatter 含 verified: pass 或 fail,且账本记录同值

Scenario: 验证失败的交付被如实标记
  Given verify_command 会因编译错误退出非零
  When peer 声称完成并交付
  Then verified: fail 且外环可从事件流看到 turn_error 或 fail 记录

Scenario: worktree 完成即清
  Given profile 开启默认 worktree
  When peer closed 且成果已 gather
  Then peers/<slug>/wt/ 被删除而 result.md 与账本保留

## Dependencies

- REQ-OLP-OBS(verified 结果经 events.jsonl/账本对外环可见)

## Source Trace

- proposal:LEP-001(§3 C1/C2/C3;operator 2026-08-22 拍板 C2 取
  "默认开 + 完成即清")
- 实测:peer 报告"工具链确实有问题";两次 lib-only 失实验证
  (specs 黑板第 3/6 条)。

## Open Questions

None.

## Next

Single exit: compile this requirement into a task contract with
`agent-spec requirements draft-specs`.
