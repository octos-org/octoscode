spec: task
name: "写入面校验主审权租约:外环会话结束或换人后,它留下的命令写不进来"
id: task-outer-duty-lease
status: implemented
tags: [olp, outer-duty, r7, fencing]
estimate: 1d
---

## Intent

外环被杀后,它执行到一半的命令(Codex 把命令放进别的进程组)会活下来,锁已经 VACANT,
它仍能写黑板、推 GitHub。operator 决定(2026-11-15,关掉 #744):不杀进程——那会连带杀掉
外环顺手拉起的服务,外环挂了不能影响内环——改为在写入口校验租约,让这些命令写不进来。
协议 R7 原文"硬 gate(写入面校验 lease)为后续条目"即本任务。

## Decisions

- `outer-duty hold` 取锁后生成 128 位随机租约号,原子写到 `<锁>.lease`(0600),再启动 agent;
  经环境变量交给 agent 及其执行的一切命令:`OLP_DUTY_LEASE`、`OLP_DUTY_PROJECT`(规范化的
  项目路径)、`OLP_DUTY_BIN`(这个 octoscode)、`OLP_DUTY_PANE`(外环所在的 herdr 窗格,
  不在 herdr 里为空)。
- `octoscode outer-duty lease --project P`:读环境里的 `OLP_DUTY_LEASE`,先读租约文件、再探锁、
  再读一次;锁 HELD 且两次读到的都等于它 → stdout `VALID`、退出码 0;锁 VACANT、已换人或读不
  出 → stdout `REFUSED`、原因在 stderr、退出码 3;环境里没有租约 → 退出码 2。
- 写入口:`olp board append`(在黑板的 flock 锁内、写入之前)、`olp board archive`(非
  `--dry-run`)、`olp repo promote`、`olp ask answer`。环境带 `OLP_DUTY_LEASE` 且写入者的
  `HERDR_PANE_ID` 等于 `OLP_DUTY_PANE` 时才校验;校验不过退出码 6,什么都不写。
- 不带租约的写入(内环、单元、人)照旧;写入者在别的 herdr 窗格(继承了外环环境的内环)
  视为不是外环,照旧。实测 herdr 的 API 命令不会自动拉起服务,服务由人或常驻任务启动;文档
  要求不要由外环启动它。
- `olp-board-append.sh` 保持自包含,检查写在脚本里;Python 工具共用 `olp-cli-lib.py` 的
  `duty_gate`。
- `docs/OLP_OUTER_BOOT.md` §3.5 的"硬 gate 为后续条目"改为现状,golden 哈希同步更新
  (operator 2026-11-15 批准)。

## Boundaries

### Allowed Changes
- src/outer_duty.rs、src/cmd/outer_duty.rs、src/cmd/mod.rs、src/cmd/olp.rs(帮助一行)
- scripts/olp-board-append.sh、scripts/olp-board-archive.py、scripts/olp-local-pr.py、
  scripts/olp-answer.py、scripts/olp-cli-lib.py
- tests/outer_duty_contract.rs、tests/olp_cli.rs、tests/olp_evo_retro.rs(BOOT golden)
- docs/OUTER_LOOP_PROTOCOL.md、docs/OLP_OUTER_BOOT.md(§3.5 一句)、docs/OLP_CLI.md、
  docs/OCTOLOOP_AGENTS.md、docs/OCTOLOOP_AGENTS.zh-CN.md、skills/octoloop/AGENTS.md(命令对照一行)
- specs/task-outer-duty-lease.spec.md

### Forbidden
- 不杀任何进程,不改死亡耦合。
- 不改锁命名、`check` 的输出、sidecar 字段。
- 不改不带租约时任何写入工具的行为。

## Out of Scope

- 外环自动重启(B)与重启后自动接班(C),各自另立任务。
- 普通的 `git push`(不经 `olp repo promote`)不在校验范围内。
- 新持有者取锁到写下租约之间的微秒级窗口。

## Completion Criteria

Scenario: 租约只对当前持锁者有效(critical)
  Tags: critical
  Test: duty_lease_is_valid_only_for_the_live_holder
  Given 一个持锁的 hold 及其写下的租约号
  When 依次用这个租约号、错误的租约号、不带租约调用 `outer-duty lease`,再结束会话、再开一个新的 hold
  Then 持锁时 VALID/退出码 0,错误租约 REFUSED/3,不带租约退出码 2;会话结束后 REFUSED/3;新 hold 的租约号与旧的不同,旧的 REFUSED/3,新的 VALID/0

Scenario: 外环会话结束后,带着它租约的写板与归档被拒,别的窗格与不带租约的写入照常(critical)
  Tags: critical
  Test: olp_board_append_refuses_a_dead_outer_lease
  Given 经 `olp duty hold` 启动的外环把收到的 `OLP_DUTY_*` 环境记下
  When 持锁时带这些环境写板;会话结束后再写板、再归档;再以别的 `HERDR_PANE_ID` 写板;最后不带租约写板
  Then 持锁时写入成功;会话结束后写板与归档都退出码 6 且黑板不变;别的窗格与不带租约的写入成功
