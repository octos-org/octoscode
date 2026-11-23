spec: task
name: "OctoLoop 事件日志:一步派卡(健康、复验方、占用、不碰)"
id: task-olp-dispatch-log
status: implemented
tags: [olp, store, jsonl, dispatch, rust]
estimate: 1d
---

## Intent

`olp card dispatch` 在 Markdown 板上是脚本 `olp-dispatch.py`:查健康、查占用、写板、往窗格
打一段话。事件日志上只有 `card open`,没有任何检查。本任务把派卡在事件日志上用 Rust 做全,
并加入两条新规则:卡上的复验方必须是这台单元跑得了的(2026-11-23 定:operator 给范围,外环
在范围内挑);卡上可以写明不许碰什么(借鉴 Antigravity Swarm 的 brief)。

## Decisions

- 入口:`octoscode olp card dispatch <单元> --log <目录> --title <标题> […]`,正文从 stdin。
  给的是 `--board`(Markdown 板)时仍走 `olp-dispatch.py`,那条通道不变。
- 顺序:键是否已存在 → 健康 → 复验方 → 回测不派给原作者 → 占用 → 写入。读日志的检查和写入
  在日志锁内完成,两次并发派卡不会都看到"空闲"。
- 幂等键 `--key`;不给时为 `dispatch:<单元>:<摘要>`,摘要取自单元号、标题、正文、分工与
  不碰清单。键已存在且是派给同一台单元的卡:不再检查、不再写,返回原卡号(`created: false`)。
- 健康:读 `olp unit` 的登记与心跳(`unit::health`),只有"正常"才派;未登记也拒绝。
  `--skip-health` 跳过健康与复验方两项。
- 复验方:`--review` 里的每个名字,在单元最近一次心跳里——
  - 是它上报的复验 CLI(`reviewers`):必须已安装,且不是"未登录"(登录状态未知的放行);
  - 不是上报的 CLI,但 herdr 里有这种内环在跑(`agents`):放行;
  - 都不是:拒绝,说明"不在心跳里"。
  `--allow-reviewers` 放行(operator 直接点名时用)。工具不内置任何复验方名单。
- 回测卡 `--verify-of M`:#M 必须存在,且不是派给目标单元的;写进 `data.verify_of`,渲染为
  `verify-of: #M`。
- 占用:目标单元还有未了结的卡时拒绝并列出卡号;`--allow-busy` 放行。
- 不碰清单 `--avoid <文本>`(可重复,每条单行、最长 400 字节):写进 `data.avoid`;渲染为
  `avoid: …` 行;单元的收件箱(`remote inbox`)每张卡带 `avoid`,`how` 里加一句"avoid 列出的
  东西不要碰,非碰不可就回执 blocked 说明"。没有清单的卡不带这个字段。
- 送达:`--wake <窗格>` 把窗格记进卡里(同 `card open --pane`),卡进单元的收件箱,由单元值守
  唤醒;hub 不往窗格打字。
- 退出码:0 成功;1 被规则拒绝,未写入;2 用法;4 日志读写失败;6 主审权已失效。

## Boundaries

### Allowed Changes
- src/olp/dispatch.rs、src/olp/cards.rs、src/olp/render.rs、src/olp/client.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_dispatch_log.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-dispatch-log.spec.md

### Forbidden
- 不改 `scripts/olp-dispatch.py`:Markdown 板的派卡照旧。

## Out of Scope

- operator 的"原则"(什么样的卡用几个复验方)存放在哪里:待定,见设计文档。
- 单元上执行"不碰":这里只把清单送到;是否越界由复验方与外环判断。

## Completion Criteria

Scenario: 健康且空闲的单元接到卡,同一个键重跑不重复派卡(critical)
  Tags: critical
  Test: a_healthy_free_unit_gets_the_card_once
  Given 一台登记过、心跳正常的单元
  When 带实现方、复验方、窗格派一张卡;原样再跑一遍;再用各种不合法的参数派卡
  Then 第一次写入一条 dispatched 事件(带单元、窗格、复验方),输出卡号与键;第二次返回同一卡号、created 为 false、不因"正忙"被拒、日志仍只有一条;不合法的参数退出码 2 且不写入

Scenario: 不健康、未登记、正忙的单元不接卡(critical)
  Tags: critical
  Test: an_unhealthy_or_busy_unit_is_refused
  Given 一台巡检未过的单元
  When 向它和一台未登记的单元派卡;加 --skip-health 连派两张;再加 --allow-busy
  Then 前两次退出码 1、说明原因、不写入;跳过健康后第一张写入,第二张因正忙被拒并列出占着的卡号;放行后写入

Scenario: 卡上的复验方必须是单元心跳里可用的(critical)
  Tags: critical
  Test: reviewers_the_unit_cannot_run_are_refused
  Given 一台单元,心跳报 claude 未登录、codex 已登录、gemini 未安装,herdr 里有 octoscode 与 kimi
  When 分别以 claude、gemini、qwen、codex+claude 作复验方派卡;再以 codex+kimi 派;再以 claude 加 --allow-reviewers 派
  Then 前四次退出码 1,各自说明未登录、未安装、不在心跳里,并提示 --allow-reviewers,不写入;已登录的 CLI 与在跑的内环通过;放行后照派

Scenario: 回测卡不派给原作者
  Test: a_verification_card_does_not_go_back_to_the_author
  Given 两台正常的单元,#1 派给了 m07
  When 把 #1 的回测卡派给 m07;把不存在的 #9 的回测卡派给 m08;把 #1 的回测卡派给 m08
  Then 前两次退出码 1 并说明;第三次写入,渲染出的板上有 verify-of: #1

Scenario: 卡上写明不许碰什么,单元在收件箱里看得到
  Test: what_a_card_must_not_touch_reaches_the_unit
  Given 一台正常的单元
  When 带两条 --avoid 派一张卡,单元查收件箱;再派一张不带清单的卡
  Then 事件里有 avoid 清单,渲染出的板上逐条有 avoid: 行,收件箱里该卡带 avoid、how 里提到它;不带清单的卡没有这个字段,how 也不提

Scenario: 汇总也读事件日志
  Test: the_summary_reads_the_event_log
  Given 一张派出的卡,单元上报了送达失败
  When 对日志目录执行 olp summary(JSON 与文本)
  Then 卡、分工、各区计数、单元状态都在;告警里有这张卡的送达失败;文本里该卡在收件箱
