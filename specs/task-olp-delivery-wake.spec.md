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
  `{"unit": …, "items": [{number, title, pane, impl, review, body}]}`。只有本单元的卡;
  `body` 是卡的全文。
- 单元一侧 `remote inbox [<窗格>]`:把上面的 JSON 打出来,并给每张卡加上 `next`——接下来的
  命令 `claim`(`remote progress <N> claimed`)和 `ack`(`remote ack <N> done|wontdo|blocked <说明>`);
  有卡时另有一句 `how` 说明顺序(领取、按 `body` 办理、回执)。被唤醒的 agent 只凭这一次输出
  就能办完,不用猜命令(2026-11-22 真实环境验证:没有这些时,agent 读完收件箱就停下来问人)。
  hub 较旧、不返回 `body` 时,`next.read` 指向 `remote mirror`。
- `olp remote --help` 先列出本程序自己的动作(ack、mirror、progress、inbox、watch、serve-log),
  再给出仍由脚本处理的动作的帮助。
- 单元值守 `remote watch <窗格> [--once] [--interval 15] [--renudge 900] [--retry 120] [--idle-wait 5]`:每轮
  向 hub 问该窗格的收件箱;有还没提醒过(或距上次提醒已过 `--renudge` 秒)的卡时,先
  `herdr agent wait <窗格> --until idle --until done --timeout <毫秒>`,agent 空闲才
  `herdr agent prompt <窗格> "有新派卡,请执行:octoscode olp remote inbox <窗格>,再按输出里的 next 领取、办理、回执" --wait --until working --until blocked --timeout 10000`。
  - 短句固定,不含卡号、标题、正文;值守不读卡的内容。
  - 一轮只发一句,不论有几张卡在等。
  - agent 在忙就不发,下一轮再看。
  - 发话用 herdr 自带的"提交并等待":提交后看到 agent 进入 working 或 blocked(它自己要审批)
    才算提醒过。herdr 报 `agent_prompt_stalled`/`timeout`(话打进去了,agent 没有开始——herdr
    在对话框挡着输入时也可能报空闲,2026-11-22 在 octoscode 与 kimi 两种 agent 上都遇到)记为
    未送达,`--retry` 秒后再发一次。连续两次都没接住,说明窗格不在收输入,再发只会让短句在
    输入框里越堆越多(同一次验证里看到),于是不再按 `--retry` 追发,改按 `--renudge` 的间隔。
    被接住一次后计数清零。herdr 报 `agent_blocked` 时它没有打字,按"接不了卡"处理。
  - 已提醒、还没领取的卡在等的时候,每轮也用 `herdr agent get` 看一眼:窗格此后卡在审批、
    对话框,或 agent 没了,同样算接不了卡。
  - 等空闲没等到时,用 `herdr agent get <窗格>` 问 agent 的状态:`working` 是在忙,下一轮再看;
    其余(`blocked`——被对话框或审批挡住,`unknown`——窗格里没有 agent 或 herdr 没在跑)是
    接不了卡,**不往里打字**。
  - **送达失败要让 hub 知道**:窗格持续 `--retry` 秒接不了卡(上面两种情况,或发话没人接),
    值守对每张在等的卡上报一次 `progress <N> undelivered`(说明里带窗格与原因)。hub 的收件箱
    把这样的卡从"进行中"提到"收件箱",理由是"送达失败:单元报告窗格没有接住提醒,请看一下
    该窗格";卡被领取(`claimed`)后恢复为"已派出,等内环 ACK"。一张卡要么被领取,要么 hub
    知道它卡住了,不会无声地等着。
  - 状态文件现在是 `{nudged, misses, stuck, reported}`;旧格式读不出时从空开始。
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
- 自动化测试用的是记录调用的假 herdr;真实 herdr 与真实 agent 窗格上的验证是手工做的
  (2026-11-22,hagency herdr 0.9.5,octoscode agent),不在测试套件里。

## Completion Criteria

Scenario: 派出的卡在单元的收件箱里等到被领取(critical)
  Tags: critical
  Test: a_dispatched_card_waits_in_the_units_inbox_until_claimed
  Given 四张卡:两张给 m07 的不同窗格,一张给 m08,一张给 m07 但没有窗格
  When 各单元查收件箱(带与不带窗格),m07 领取一张两次,外环作废另一张
  Then m07 只看到自己带窗格的两张、按窗格可筛选,m08 只看到自己的,没有卡的单元为空;每张卡带全文 body 与 next(领取、回执命令),有卡时带 how,空收件箱不带;领取后该卡离开收件箱,作废的也离开;没有单元的 --pane 与非法窗格被拒

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

Scenario: 窗格接不了卡时不打字,并让 hub 知道(critical)
  Tags: critical
  Test: a_pane_that_cannot_take_a_card_is_reported_to_the_hub
  Given 一张派给窗格的卡,herdr 报该窗格的 agent 为 blocked
  When 跑一轮;--retry 0 再跑两轮;解除阻塞后再跑一轮,单元领取
  Then 始终没有往被挡住的窗格打字;第一轮只说明接不了;过了重试时间向 hub 上报一次 undelivered,hub 收件箱把该卡列入"收件箱"并写明送达失败,日志渲染出的 Markdown 读出来一样;解除后被唤醒,领取后提示消失

Scenario: `olp remote --help` 列出本程序自己的动作
  Test: remote_help_lists_the_actions_this_binary_has
  Given 打包了 remote 的脚本的 octoscode
  When 执行 olp remote --help
  Then 输出先列出 ack、mirror、progress、inbox、watch、serve-log,后面仍有脚本处理的动作(如 flush)的帮助,退出码 0

Scenario: 发了话而 agent 没有开始工作的,不算提醒过
  Test: a_nudge_the_agent_did_not_pick_up_is_tried_again
  Given 一张待领取的卡,herdr 报 agent 空闲、收下了话,但 agent 没有开始工作
  When 跑一轮;紧接着再跑一轮;--retry 0 再跑两轮;对话框消失后 --renudge 0 跑一轮,再 --retry 0 跑一轮
  Then 第一轮发话并说明没有开始工作;未到重试时间不再发;到了再发一次并提示找人看窗格;连续两次未接住后不再按重试时间追发;到 --renudge 时被接住的那次算数,此后不再发
