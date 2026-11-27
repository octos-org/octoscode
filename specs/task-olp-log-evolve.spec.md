spec: task
name: "OctoLoop 按指标改进:症状进日志,阈值在日志里可调、可回放,自动试改带对冲与撤回"
id: task-olp-log-evolve
status: implemented
tags: [olp, store, jsonl, metrics, policy]
estimate: 2d
---

## Intent

指标(`task-olp-log-metrics`)只让人看见。2026-11-28 operator 定:把闭环做完——偏离自动成为带证据
的症状;随使用变化的阈值从代码里拿出来,记在日志里,换一个值之前能回放历史;最后允许按症状
自动试改,但一次只改一处、带对冲指标、变差自动撤回。只调阈值:日志、锁、闸门、主审权不是旋钮。

## Decisions

- 症状(`src/olp/symptom.rs`,`octoscode olp log symptoms <日志目录> [--window 7] [--min-cards 5]
  [--now 时刻] [--record] [--json]`):卡按派出时间分三段——最近一个窗口、它之前的一个窗口、更早
  的基线。一个指标在两个窗口都比基线差、基线与两个窗口合计各至少 `--min-cards` 张卡,才是
  症状;每条症状列出两个窗口里最能说明它的卡(至多 5 张)。只偏离一个窗口是噪声,不算。
  - 时长类(`total`、`to_claim`、`work`、`outer_wait`,取中位数):长出一半且多一分钟以上算差;
  - 每卡次数(`returns_per_card`、`alerts_per_card`、`blocked_per_card`):多 0.2 算差;
    `reversed_rate`:多 0.1 算差。
- `--record` 把症状各记一条 `symptom` 事件,键为指标加窗口起始日,同一窗口不重复。外环值守
  每天(UTC)自己算一次并记下;`symptom` 事件算作等外环处理的新事件,会唤醒外环窗格。
  症状只是发现,本身不改变任何东西。
- 旋钮(`src/olp/tune.rs`,`octoscode olp tune …`):有名字、默认值与上下界的阈值,值是日志里
  最后一条 `tuned` 事件,没有就是默认值;越界的值收回界内。改动因此可查(谁、为什么、从多少
  改来)、可撤,所有读日志的命令看到同一个值。压缩不移走 `tuned` 事件,并核对压缩前后旋钮的
  值相同。
  - `review_backlog_max`(默认 0 = 不限):等外环裁决的卡(收件箱分区)到这么多张,
    `card dispatch --log` 拒绝派新卡(退出码 1),`--allow-backlog` 放行。Markdown 板不受影响。
  - `follow_up_after`(默认 900 秒):单元认领后静默多久,值守提醒主持接着办。hub 在单元的
    `inbox` 回复里带上 `tuning.follow_up_after`;`olp remote watch` 没有显式给 `--follow-up` 时用它。
- `tune set <旋钮> <值> --reason …`(必须写原因,值在界内,同值不写)、`tune revert <旋钮>`
  (回到上一次改动之前的值,记下撤的是哪一条)、`tune list`。写入前查主审权租约。
- `tune replay <旋钮> <值>`:只读历史,回答"换成这个值会有哪些不同、那些卡后来怎样":
  积压上限——历史上每次派卡时的积压、会被拦下的派卡,以及被拦的卡与其余的卡后来等外环裁决
  的时长;跟进间隔——在手的静默段数、提醒次数的上限、超过阈值的静默总秒数。
- `tune auto [--apply]`:
  - 规则:`outer_wait` 的症状 → 下调 `review_backlog_max`(原为不限时取两个窗口里派卡时积压
    峰值减一,峰值不到 2 不动),对冲 `total`;`work` 的症状 → `follow_up_after` 减四分之一,
    对冲 `returns_per_card`。
  - 一次只有一处改动在试行(`tuned` 事件带 `trial`、所依据的症状与卡、对冲指标及改动前的值)。
    试行期间不试别的;改动之后派出的卡不到 `--min-cards` 张不判。够了:对冲指标比改动前差
    (同症状的判据)就写一条撤回(`verdict: reverted`,值回到原值),否则写一条保留
    (`verdict: kept`)。人对同一旋钮的改动也结束试行。
  - 不带 `--apply` 只说会做什么,不写。外环值守只在 `OLP_TUNE_AUTO=1` 时每天自动做一次;
    默认关。自动的 `tuned` 事件同样唤醒外环窗格。

## Boundaries

### Allowed Changes
- src/olp/symptom.rs、src/olp/tune.rs、src/olp/metrics.rs、src/olp/mod.rs、src/cmd/olp.rs
- src/olp/dispatch.rs、src/olp/serve.rs、src/olp/client.rs、src/olp/compact.rs、src/olp/outerwatch.rs
- tests/olp_log_evolve.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-log-evolve.spec.md

### Forbidden
- 不把日志、锁、闸门、主审权、受保护的技能段落做成旋钮。
- 自动改动不得同时试两处,不得没有对冲指标。
- 症状与回放不写日志(`--record` 除外)。

## Out of Scope

- 规则与话术(选复验方、卡模板、接班提示)的进化:需要人批准的那一层,另做。
- Markdown 进化板(`olp evo harvest / metrics`)不读这些症状;两套并存。
- 人工介入量、产能、成本类指标的症状:等事件里有"谁做的"、单元状态变化与用量之后。
- 回放只能说"会有哪些不同、那些卡后来怎样",不能证明换了值结果一定更好。
- 单元值守读取下发的 `follow_up_after`:只测了 hub 的下发,值守一侧的读取没有单独的测试。
- 样本很少时自动调参不会动(两个窗口合计不足 `--min-cards` 张卡就没有症状);这是有意的。

## Completion Criteria

Scenario: 连续两个窗口都变差才算症状,带着说明它的卡,记进日志只记一次(critical)
  Tags: critical
  Test: a_symptom_names_its_cards_and_is_recorded_once
  Given 12 张卡:基线 6 张等外环裁决约两分钟,前一窗口 3 张约一小时,最近窗口 3 张约一个半小时
  When 取症状;把"现在"拨回一周再取;要求至少 7 张卡再取;带 --record 取两次
  Then 有 outer_wait 的症状,卡按等得最久的排;只有一个窗口变差时没有症状;卡不够时没有症状;只看不写;第一次记下、第二次不重复,日志里的症状事件带着那几张卡

Scenario: 旋钮的值是日志里最后一次改动,有界、要写原因、可撤回(critical)
  Tags: critical
  Test: a_knob_is_set_with_a_reason_within_bounds_and_reverted
  Given 一个空日志
  When 设 follow_up_after 为 300;同值再设;越界、不认识的旋钮、不写原因、撤一个没改过的旋钮、不认识的动作;撤回;再设为 240 后单元取收件箱
  Then 值与原因可查;同值不写;拒绝的退出码 1、用法错 2,都不写;撤回后回到 900 并记下撤的是哪一条;收件箱回复里 tuning.follow_up_after 是 240

Scenario: 等外环裁决的卡到了上限就不再派新卡(critical)
  Tags: critical
  Test: dispatch_stops_at_the_backlog_limit
  Given 一张已回执、等外环裁决的卡
  When 默认值下派卡;把 review_backlog_max 设为 1 后派卡;带 --allow-backlog 派卡;外环采认后再派;回放上限 1
  Then 默认不限;到上限时退出码 1、说明上限与放行办法、不写;放行可派;采认后可派;回放指出历史上会被拦下的是哪两张

Scenario: 自动调参一次只试一处,对冲指标没变差就保留(critical)
  Tags: critical
  Test: an_automatic_change_is_tried_alone_and_kept_when_its_hedge_holds
  Given 等外环裁决连续两个窗口变差、派卡时积压到过两张的历史
  When 不带 --apply 运行;带 --apply 运行;立刻再运行;之后派出六张不更慢的卡再运行
  Then 先只说会把 review_backlog_max 试改为 1,不写;写入后旋钮为 1 且标为试行,事件带症状与对冲指标;试行中不再试别的,卡不够不判;够了判为保留,不再是试行

Scenario: 对冲指标变差,自动改动自动撤回(critical)
  Tags: critical
  Test: an_automatic_change_is_undone_when_its_hedge_gets_worse
  Given 同样的历史,自动试改已写入
  When 之后派出的六张卡从派出到采认慢了数倍,再运行;另一份日志里人把同一旋钮改成 4
  Then 判为撤回,旋钮回到 0,撤回事件指向试改的那一条;人的改动结束试行,值为 4

Scenario: 压缩不丢旋钮的值
  Test: a_compaction_keeps_the_knobs
  Given 一条两个月前的 tuned 事件与三张已了结的旧卡
  When 压缩(保留 1 天、最近 0 张)
  Then 三张卡归档;单元收件箱回复里的 follow_up_after 仍是改过的值

Scenario: 症状要两个差的窗口和足够的卡
  Test: a_symptom_needs_two_bad_windows_and_enough_cards
  Given 基线十张卡,两个窗口各三张
  When 只有最近窗口慢;两个窗口都慢;要求八张卡
  Then 依次是没有症状;total 与 outer_wait 两条症状,卡按分量排;没有症状

Scenario: 时长长出一半算差,次数多出固定一步算差
  Test: a_rate_is_worse_by_a_fixed_step_and_a_duration_by_half
  Given 若干基线与当前值
  When 判断是否变差,并算三张卡的指标
  Then 时长要长出一半且多一分钟;每卡次数多 0.2、改判率多 0.1;时长取中位数,次数取平均

Scenario: 旋钮取最后一次改动,越界收回界内
  Test: a_knob_is_its_last_change_within_its_bounds
  Given 没有改动、有改动、有越界的改动
  When 读旋钮
  Then 默认值;最后一次的值;收到下界;不认识的旋钮为 0

Scenario: 回放数出另一个值会改变什么
  Test: a_replay_counts_what_another_value_would_have_changed
  Given 三次派卡(派出时积压为 0、1、2)与一段 40 分钟的在手静默
  When 回放积压上限 2 与 1、跟进间隔 600 与 3600
  Then 被拦下的分别是第三张、后两张;静默一段,提醒次数与超过阈值的秒数按值变化
