spec: task
name: "多机部署:一步派卡、单元占用视图、回测卡不派给原作者"
id: task-olp-dispatch
status: implemented
tags: [olp, cluster, dispatch, unit]
estimate: 1d
---

## Intent

大外环给单元派一张卡原本是四步:查单元健康、查它是否正忙、写板、唤醒。每一步都靠外环记得
去做,漏一步卡就悬着;中途失败后重来又可能派出两张卡。新架构里一台单元同一时间只做一张
卡,回测(R1 冒烟)也作为任务派回单元,但不能派给做这张卡的那台。本任务把派卡做成一条可以
安全重试的命令,并给出单元的占用视图。

"派发是一步完成、可重试的动作"借鉴自 T3 Code 的 `delegate_task`(`clientRequestId` 幂等)。

## Decisions

- 命令 `scripts/olp-dispatch.py <单元号> --board <板> --title <标题> [选项]`,正文从 stdin。
  纯 Python 标准库,兼容 Python 3.9。在 hub 上由持锁的大外环执行。
- 步骤与顺序:① 板上已有本次派发的键 → 跳到唤醒;② `olp-unit.py check <单元号>` 必须退出
  0(`--skip-health` 跳过);③ `--verify-of M` 时 #M 必须在板上,且 #M 的 `unit:` 不等于目标
  单元;④ 目标单元不能还占着未了结的卡(`--allow-busy` 放行);⑤ 经
  `olp-board-append.sh --key <键>` 写板,条目为
  `### {N}. <标题>(<日期>,<署名>)` / `unit: <单元号>` / 可选 `verify-of: #M` / 空行 / 正文,
  编号由写板脚本在锁内分配,正文里的 `{N}` 一并替换;⑥ 给了 `--wake <窗格>` 时执行
  `herdr --machine <单元号> agent prompt <窗格> <一句话,含条目号>`(`OLP_HERDR` 可替换程序)。
- 幂等:键默认是 `dispatch:<单元号>:<sha256(单元号、标题、正文) 前 16 位>`,可用 `--key`
  指定。重跑同一条命令时,①命中,不再做健康、占用、回测检查(此时单元"正忙"的正是这张
  卡),不写第二张卡,只补做唤醒。
- 占用:`olp-inbox.py --units [--json] [--unit U]` 按单元列出还占着它的卡——派给它(条目里
  最后一条 `unit:` 行)且分区不是"已了结"的条目。`olp-dispatch.py` 通过加载同目录的
  `olp-inbox.py` 复用这段判定,不另抄一份。
- 被打回的卡:带 `unit:` 行的条目在打回之后归为"进行中"(原因:已打回,等单元返工后重新
  推送),仍算占着单元;不带 `unit:` 行的条目(单机)保持"已了结"。这与本地 PR 状态里
  `returned` 等单元的定义一致。
- 输出:文本模式最后一行是条目号,并给出挂哨标记 `本地复验通过(#N)`;`--json` 输出
  `number`、`unit`、`created`、`key`、`woken`、`watch_token`。
- 退出码:0 成功;1 被规则拒绝(不健康、正忙、回测派给了原作者、#M 不存在),未写板;2
  用法错误;4 写板失败;5 已写板但唤醒失败。
- 并发:占用检查与写板不是一个原子操作。派卡只由持主审权锁的大外环执行,同一时刻只有一个
  派发者,所以不加额外的锁。
- 不做:卡上的权限档声明与"只能收窄"的规则(需要 octos 侧配合,另议)。

## Boundaries

### Allowed Changes
- scripts/olp-dispatch.py
- scripts/olp-inbox.py(占用视图;带 `unit:` 的卡被打回后归为进行中)
- tests/olp_dispatch.rs
- docs/OLP_CLUSTER.md
- docs/OCTOLOOP_AGENTS.zh-CN.md(仅收件箱分区说明的一句)
- specs/task-olp-dispatch.spec.md
- specs/task-olp-board-inbox.spec.md(打回规则的一句)

### Forbidden
- 不改写板脚本、单元登记工具、推送钩子。
- 派卡命令不得绕过写板脚本直接写黑板。
- 不自动改派、不自动了结卡。

## Out of Scope

- 权限档只能收窄。
- 回测结果的自动汇总。
- 多个派发者并发。

## Completion Criteria

### Rule: dispatch — 一步派卡
Scenario: 一条命令查单元、写板并唤醒(critical)
  Tags: critical
  Test: dispatch_writes_a_numbered_card_and_wakes_the_unit
  Given 一台登记过且心跳正常的单元 m07,一块空板
  When 带 `--wake` 派一张正文含 `task/{N}` 的卡
  Then stdout 最后一行是 1;板上是带 `unit: m07` 的 #1,正文里的 `{N}` 已换成 1,并留有派发键;herdr 收到 `--machine m07 agent prompt <窗格> …#1…`;占用视图里 m07 为忙

Scenario: 唤醒失败后原样重跑只补做唤醒(critical)
  Tags: critical
  Test: dispatch_retry_after_a_failed_wake_does_not_write_a_second_card
  Given herdr 暂时失败
  When 派卡;herdr 恢复后原样再跑一次;再以 `--json` 跑一次
  Then 第一次退出码 5 且卡已写板;第二次成功、提示没有重复派卡、黑板不变、herdr 只被成功调用一次;JSON 里 `created` 为 false、编号仍是 1

### Rule: guards — 派卡前的检查
Scenario: 不健康或正忙的单元不派卡(critical)
  Tags: critical
  Test: dispatch_refuses_unhealthy_and_busy_units
  Given 单元 m07 起初未登记、随后已登记但无心跳、最后心跳正常
  When 在各状态下派卡;派出第一张后再派第二张;第一张被推送并打回后再派;第一张被采认后再派;以及 `--allow-busy` 与 `--skip-health`
  Then 未登记与无心跳时拒绝且黑板不变;第一张卡未了结(含被打回)期间第二张被拒并指出占着的卡;采认后可以派出 #2;两个放行开关各自生效

Scenario: 回测卡不派给做这张卡的单元
  Test: dispatch_verification_card_avoids_the_author_unit
  Given #1 由 m07 完成并已采认,m08 健康
  When 以 `--verify-of 1` 派给 m07;以 `--verify-of 9` 派给 m08;以 `--verify-of 1` 派给 m08
  Then 前两次以退出码 1 拒绝;第三次成功,条目带 `unit: m08` 与 `verify-of: #1`,占用视图里 m08 为忙、m07 为空闲

Scenario: 用法错误不写板
  Test: dispatch_rejects_bad_usage
  Given 非法单元号、空标题、空正文、非法键、非法日期、非数字的 `--verify-of`
  When 分别运行
  Then 都以退出码 2 结束,黑板不变;没有派给单元的条目时占用视图为空
