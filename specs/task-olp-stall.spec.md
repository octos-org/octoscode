spec: task
name: "OctoLoop 事件日志:整体静默才报警,以及领不走的卡"
id: task-olp-stall
status: implemented
tags: [olp, store, jsonl, hub, timeout]
estimate: 1d
---

## Intent

hub 侧的"超时"原先的设计是按单张卡计时。借鉴 Antigravity Swarm 的 monitor(2026-11-23):一张
卡做得久不是问题,只要有人在做——长时间构建是正常的;真正要报的是**所有人都在等**:有卡没
了结、占着卡的单元上没有内环在工作、日志也没有新事件。这时没有人会自己动起来。报的时候要
说清是哪张卡、等什么、从哪条事件起、谁能推动,并且同一段沉默不反复报。

另补上单元值守的一个缺口:agent 每次被提醒都开始工作、卡却一直没领取(出错、额度用完),
原先只会按 `--renudge` 一直提醒,hub 不知道。

## Decisions

- 命令 `octoscode olp board stall <日志目录> [--quiet 秒] [--stale-after 秒] [--state 文件] [--json]`,
  在 hub 上按定时器跑。只读日志;自己的状态记在 `--state`(默认日志目录下 `stall-state.json`)。
- 静默的判定,三条同时成立:
  1. 有派给单元、处于"收件箱"或"进行中"的卡(搁置的卡是有意在等,不算);
  2. 这些卡所在的单元里,没有一台在最近一次未失联的心跳中报告有内环处于 `working`;
  3. 日志最后一条事件距今不少于 `--quiet` 秒(默认 600)。
- 看不到的单元(未登记、心跳失联)不当作"在工作",并在输出里单独列出。
- 静默时每张未了结的卡一条:卡号、单元、原因、该卡最后一条事件的时间与序号、谁能推动。
  - 卡在外环的"收件箱"里(待复验、wontdo、blocked、送达失败):等外环,原因带收件箱给的理由;
  - 未领取:等单元,"派出后没有领取";
  - 已领取无回执:等单元,"领取后没有回执"。
- 不重复:同一张卡,最后一条事件没变时,报过之后要再过 3 次运行才再报,最多报 3 次;事件
  变了从头算。卡了结后从状态里去掉。被压下不报的张数在 `held_back` 里。
- 退出码:0(是否静默看输出);2 用法;4 日志读不了。
- 单元值守(`remote watch`):状态文件增加 `picked`(每张未领取的卡,agent 开始工作的提醒次数)。
  某张卡已有 2 次这样的提醒、到了下一次该提醒的时间仍未领取,向 hub 上报一次
  `progress <N> undelivered`,说明为"agent 已 N 次开始工作,卡仍未领取(可能出错或额度用完)"。
  之后照常按 `--renudge` 提醒;领取后 hub 收件箱里的"送达失败"消失。

## Boundaries

### Allowed Changes
- src/olp/stall.rs、src/olp/client.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_stall.rs、tests/olp_delivery.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-stall.spec.md、specs/task-olp-delivery-wake.spec.md

### Forbidden
- 不往任何窗格打字,不改派卡。

## Out of Scope

- 把静默提醒送到外环的窗格(外环事件收件箱与外环值守的任务)。
- 外环自己是否在工作:本命令只看单元;等外环的卡照样列出,由读它的一方判断。
- 把本命令注册成常驻定时任务(`olp keepalive`)。

## Completion Criteria

Scenario: 有人在工作、或日志刚有动静,都不算静默(critical)
  Tags: critical
  Test: working_units_and_a_fresh_log_are_not_a_stall
  Given 两台单元,一台的内环在工作,一台空闲
  When 没有卡时查;各派一张卡后查;都空闲后不带 --quiet 0 再查
  Then 没有卡不算静默;有一台在工作不算静默并列出它,没有任何提醒;都空闲但日志刚写过,不算静默并说明还没到时间

Scenario: 整体静默时逐张卡说明原因、起点和谁能推动(critical)
  Tags: critical
  Test: a_stall_names_each_card_its_cause_and_who_can_move_it
  Given 三张卡:一张没领取,一张领了没回执,一张已 ACK(done) 等外环复验;没有内环在工作
  When 以 --quiet 0 查;再给一台未登记的单元派一张卡后查
  Then 判为静默,三张卡各一条:前两张等单元(没有领取 / 领取后没有回执),第三张等外环(待复验),都带单元、起始时间与事件序号;未登记的单元列在看不到的单元里,不当作在工作

Scenario: 同一段沉默不连着报,变了才再报,最多三次(critical)
  Tags: critical
  Test: the_same_silence_is_not_reported_over_and_over
  Given 两张没人动的卡
  When 连查三次;其中一张卡有了新事件后再查;之后继续反复查
  Then 第一次两张都报;第二、三次不报并在 held_back 里计数;第四次两张都报(一张因事件变了重新计数,一张因隔了三次);没有变化的那张总共只报三次;缺参数或目录不存在退出码 2

Scenario: agent 每次都开始工作、卡却一直没领取的,也让 hub 知道
  Test: a_card_the_agent_keeps_starting_on_but_never_claims_is_reported
  Given 一张派给窗格的卡,agent 空闲、每次提醒都开始工作,但不领取
  When 以 --renudge 0 提醒两次;不到时间再跑一轮;到时间后再跑两轮;最后单元领取
  Then 前两次只是提醒;不到时间不上报;到时间后向 hub 上报一次 undelivered,说明里写明开始工作的次数,hub 收件箱显示送达失败;领取后消失
