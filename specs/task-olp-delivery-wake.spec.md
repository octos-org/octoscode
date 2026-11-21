spec: task
name: "OctoLoop 事件日志:派卡收件箱与单元值守的固定短句唤醒"
id: task-olp-delivery-wake
status: implemented
tags: [olp, store, jsonl, unit, wake]
estimate: 1d
---

## Intent

Markdown 通道的派卡把整段指令直接打进单元的窗格(`herdr agent prompt`):消息丢了就丢了,
内容进了输入框。2026-11-21 定稿的做法是"收件箱 + 唤醒":卡写进日志就是送达,收件箱是准确
来源;唤醒只是一句不带内容的固定短句,由单元上一个不是 agent 的值守进程在 agent 空闲时发。
agent 不轮询。

## Decisions

- 送达即日志:`card open --unit U --pane <窗格>` 把窗格记在派卡事件里(`data.pane`)。`--pane`
  必须与 `--unit` 同给;窗格标识为字母或数字开头,字母、数字、`_ . : -`,最长 128。
- 单元的收件箱(`cards::pending_for`):派给该单元、带窗格、未作废、还没有 `claimed` 进度节点
  的卡,按卡号排列。**领取就是上报 `claimed`**(`remote progress <N> claimed`,幂等),不另设
  领取动作。
- hub 一侧 `remote serve-log` 增加请求 `inbox [<窗格>]`,输出
  `{"unit": …, "items": [{number, title, pane, impl, review}]}`。只有本单元的卡。
- 单元一侧 `remote inbox [<窗格>]`:把上面的 JSON 打出来。
- 单元值守 `remote watch <窗格> [--once] [--interval 15] [--renudge 900] [--idle-wait 5]`:每轮
  向 hub 问该窗格的收件箱;有还没提醒过(或距上次提醒已过 `--renudge` 秒)的卡时,先
  `herdr agent wait <窗格> --until idle --until done --timeout <毫秒>`,agent 空闲才
  `herdr agent prompt <窗格> "有新派卡,请执行:octoscode olp remote inbox <窗格>"`。
  - 短句固定,不含卡号、标题、正文;值守不读卡的内容。
  - 一轮只发一句,不论有几张卡在等。
  - agent 在忙就不发,下一轮再看。
  - 已提醒的卡记在状态文件(`OLP_WATCH_STATE`,默认
    `~/.octos/outer/watch/<单元>-<窗格>.json`),重启不重复提醒;卡被领取或作废后从中去掉。
  - hub 连不上时报一句、下一轮再试。`OLP_HERDR` 指定 herdr。
- 需要 `store = log`;没有时这两条命令退出码 2 并说明。

## Boundaries

### Allowed Changes
- src/olp/card.rs、src/olp/cards.rs、src/olp/serve.rs、src/olp/client.rs、src/cmd/olp.rs
- tests/olp_delivery.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-delivery-wake.spec.md

### Forbidden
- 不改 `scripts/olp-dispatch.py`:Markdown 通道的派卡与唤醒照旧。

## Out of Scope

- 派卡前的单元健康与占用检查(`card dispatch` 迁移时接上)。
- 把值守注册成常驻任务(`olp keepalive`)、往上的方向(外环事件收件箱与外环值守)。
- 在真实的 herdr 与真实的 agent 窗格上验证:本任务的测试用的是记录调用的假 herdr。

## Completion Criteria

Scenario: 派出的卡在单元的收件箱里等到被领取(critical)
  Tags: critical
  Test: a_dispatched_card_waits_in_the_units_inbox_until_claimed
  Given 四张卡:两张给 m07 的不同窗格,一张给 m08,一张给 m07 但没有窗格
  When 各单元查收件箱(带与不带窗格),m07 领取一张两次,外环作废另一张
  Then m07 只看到自己带窗格的两张、按窗格可筛选,m08 只看到自己的,没有卡的单元为空;领取后该卡离开收件箱,作废的也离开;没有单元的 --pane 与非法窗格被拒

Scenario: 值守在 agent 空闲时发一句固定短句,不带卡的内容,不重复提醒(critical)
  Tags: critical
  Test: the_watcher_wakes_an_idle_pane_with_one_fixed_sentence
  Given 一个记录调用的 herdr,agent 空闲
  When 无卡时跑一轮;派两张卡后连跑三轮;再派一张;--renudge 0 再跑;全部领取后再跑
  Then 无卡时不调用 herdr;两张卡只发一句固定短句(先等空闲再发),其中没有卡的任何内容;已提醒的不再提醒;新卡再发一句;到了重提醒时间再发;领取后不再发

Scenario: agent 正忙时不打扰,空闲后再提醒
  Test: a_busy_agent_is_left_alone_until_it_is_idle
  Given 有一张卡在等,agent 正忙
  When 值守跑两轮,agent 转为空闲后再跑一轮;另一个窗格的值守也跑一轮
  Then 忙时不发短句,空闲后发一句;别的窗格的值守不发;缺窗格或参数不对退出码 2
