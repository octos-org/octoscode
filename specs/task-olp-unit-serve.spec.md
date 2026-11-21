spec: task
name: "OctoLoop 事件日志:单元经 ssh 强制命令的写入与可见范围(hub 一侧)"
id: task-olp-unit-serve
status: implemented
tags: [olp, store, jsonl, unit, isolation]
estimate: 1d
---

## Intent

单元不直接碰 hub 上的事件日志,它经 ssh 在 hub 上执行一条被固定的命令。本任务实现这条命令的
hub 一侧:单元能做什么、能看到什么,由 hub 决定。两个目的:进度由单元上报(2026-11-21 operator
定:外环不再逐台追问);单元之间任务隔离(现在的镜像是整块板,所有单元的卡互相可见)。

## Decisions

- 命令 `octoscode olp remote serve-log --unit <单元号> --log <日志目录>`(`src/olp/serve.rs`),
  写在 hub 的 authorized_keys 里作强制命令。单元号由这一行决定,请求里说什么都不算。请求来自
  `SSH_ORIGINAL_COMMAND`(按 shell 词法拆开,不执行):
  - `ack <N> done|wontdo|blocked`,说明从 stdin(单行、非空、≤64 KiB)→ `ack` 事件,署名
    `内环(<单元号>)`。
  - `progress <N> claimed`、`progress <N> review <轮次>`、`progress <N> rework <轮次>`(轮次
    1–999),说明从 stdin(可空)→ `progress` 事件,署名 `单元(<单元号>)`,带幂等键
    `<单元>:progress:<N>:<节点>:<轮次>`:同一节点同一轮只记一次。
  - `events [<序号>]`→ 以 JSON Lines 输出该单元可见、序号大于给定值的事件。
- 写入规则(锁内判断):卡必须存在、未作废、**当前派给这台单元**(最近一次派卡或改派的单元);
  否则拒收并说明派给了谁。
- 可见范围(`cards::visible_to`):当前派给该单元的卡的全部事件,加上该单元自己、不带卡号的事件
  (它的提问)。别的单元的卡、没有单元的卡一概不出现。
- 其余请求(含旧通道的 `append`、`read`)一律拒收。退出码:0 完成;2 强制命令行本身写错;
  3 拒收;4 日志读写失败。
- 进度事件是信息,不改变卡的分区;渲染成 `> 单元(m07)·进度(#N): <节点>[ r<轮次>][ 说明]`。
  哪些节点叫醒外环、超时判断,是后续任务。

## Boundaries

### Allowed Changes
- src/olp/serve.rs、src/olp/cards.rs、src/olp/render.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_unit_serve.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-unit-serve.spec.md

### Forbidden
- 不改 `scripts/olp-board-remote.py` 与现有的 `remote serve`(Markdown 黑板的通道照旧)。

## Out of Scope

- 单元一侧的客户端命令(`remote ack` / `progress` 走新通道、离线排队)、增量镜像落盘。
- 派卡收件箱的领取、推送登记、提问转发。
- 多项目:一条强制命令对应一个项目的日志目录;单元为多个项目干活时每个项目一把密钥或一条
  带项目参数的命令,随项目隔离任务确定。

## Completion Criteria

Scenario: 单元只能对派给自己的卡回执(critical)
  Tags: critical
  Test: a_unit_acks_only_its_own_cards
  Given 三张卡:#1 派给 m07,#2 派给 m08,#3 没有单元
  When m07 对 #1、#2、#3、不存在的 #9 回执,在请求里冒充 m08,以及对已作废的 #1 回执
  Then 只有对 #1 的回执写入(署名 内环(m07));其余退出码 3 并说明派给了谁或没有这张卡,日志不变

Scenario: 其他请求一律拒收,不写入
  Test: anything_else_is_refused
  Given 同样的三张卡
  When 发来空请求、旧通道的 append/read、缺参数、带分号的注入、多行说明、不认识的节点、别人的卡的进度、引号不闭合的请求
  Then 都是退出码 3、无输出、日志不变;强制命令行里的单元号不合法是退出码 2

Scenario: 进度节点每个只记一次,不改变卡的分区
  Test: progress_nodes_are_recorded_once
  Given #1 派给 m07
  When 重复上报 claimed、review 1、rework 1(带说明)、review 2
  Then 日志里四条进度事件;卡仍在进行中;渲染视图里有对应的进度行

Scenario: 单元只看得到自己的卡(critical)
  Tags: critical
  Test: a_unit_sees_only_its_own_cards
  Given m07 与 m08 各有一张卡并各有后续事件
  When 两台单元分别请求 events,m08 再带序号请求,没有卡的 m09 也请求
  Then 各自只得到自己那张卡的事件;带序号时只得到之后的;m09 得到空
