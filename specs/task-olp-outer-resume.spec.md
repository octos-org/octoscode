spec: task
name: "外环被自动重启后自动接班:接班提示与接班简报"
id: task-olp-outer-resume
status: implemented
tags: [olp, outer, r7]
estimate: 0.5d
---

## Intent

外环被监督任务重新拉起后(task-olp-outer-autorestart),要自己接上停机前的工作:处理停机
期间到的回执、回答积压的提问、重新挂哨、恢复循环。挂哨以挂上那一刻为基线,停机期间写进
黑板的行只能从收件箱补回;漏一步就会"看起来在跑,其实没在盯"。operator 决定(2026-11-15)
重启后自动接班。

## Decisions

- `octoscode olp outer resume --board B --project P [--repo R] [--signature S] [--json]`:只读的
  接班简报。内容:锁的状态;本会话是不是持锁者(环境里有 `OLP_DUTY_LEASE` 时由
  `outer-duty lease` 判定);最近 5 次自动重启的时间;`olp summary` 的结果(收件箱的卡、待
  回答的提问、负哨、单元);六项检查单(处理收件箱、回答提问、重新挂正哨、负哨、`/loop
  resume`、署名记接班)。持锁者退出码 0;否则照样输出、警告"只能批注,不要接班",退出码 3。
- `supervise` 重启成功后:`herdr agent wait <窗格> --until idle --until blocked`(最多
  `--ready` 秒,默认 90),再 `herdr agent prompt <窗格> <提示>`。默认提示要 `--board`,内容
  是让外环先跑上面的 resume;`--prompt` 换话,`--no-prompt` 不送。送不进去(agent 卡在确认
  界面:herdr 报 `agent_blocked`;或超时)时 stderr 提醒、黑板记一行"接班提示未送达",重启
  本身仍算成功(退出码 0)。`install` 原样传递这些参数。
- 提示算送到,要 herdr 看到 agent 开始办(`agent prompt … --wait --until working --until
  blocked`,10 秒)。没看到时:agent 空闲(`agent wait --until idle`,1 秒)才往窗格补一个
  回车,再等 10 秒,最多四次,每次间隔 3 秒;agent 不空闲(例如卡在确认界面)就不按。提示本身
  只打一次。来源:2026-11-26 在 hub 上实测,刚启动的 octoscode 丢了回车,提示留在输入框里,
  原先却报已送达。
- `--board` 是事件日志目录时:简报多一段"监督任务的留言"(日志里最近 5 条 note 事件);检查
  单的"重新挂正哨"换成"确认外环值守已注册(`olp keepalive list` 里的 `…-watch`,有就不要自己再启动)","署名记接班"换成"不用另记"。
- skill 外环模式加"被自动重启后(接班)"一段;`docs/OLP_KEEPALIVE.md` §4a 补充说明。

## Boundaries

### Allowed Changes
- scripts/olp-outer.py
- tests/olp_outer.rs
- docs/OLP_KEEPALIVE.md、docs/OLP_CLI.md
- skills/octoloop/AGENTS.md(外环模式一段、命令对照一行)
- specs/task-olp-outer-resume.spec.md

### Forbidden
- resume 不写任何文件。
- 不改 skill 受哈希保护的各节与 BOOT §0–§6。

## Out of Scope

- 替外环完成检查单(处理收件箱、写判词仍由外环自己做)。
- agent 卡在确认界面时代为确认(这是信任决策,归人)。

## Completion Criteria

Scenario: 重启后等 agent 就绪,把接班提示送进去(critical)
  Tags: critical
  Test: outer_supervise_hands_the_restarted_outer_the_resume_prompt
  Given 一个假 herdr 与空着的锁,给了 --board
  When 运行 supervise
  Then 先 agent wait 后 agent prompt,提示里有 `octoscode olp outer resume --board` 与黑板路径

Scenario: 提示没送到时黑板记一行,重启仍算成功
  Test: outer_supervise_notes_a_prompt_that_was_not_delivered
  Given agent 一直不就绪
  When 运行 supervise --ready 1
  Then 退出码 0,stderr 提醒没送到,黑板有"接班提示未送达",没有送提示

Scenario: 提示打进去了但 agent 没有开始办时,在它空闲时再按回车
  Test: outer_supervise_presses_enter_again_when_the_prompt_was_not_taken
  Given 假 herdr 收下提示但 agent 不开始办,直到窗格收到回车
  When 运行 supervise
  Then 提示只打一次,窗格收到一次回车,输出 prompted

Scenario: --no-prompt 不送任何话
  Test: outer_supervise_sends_nothing_with_no_prompt
  Given 一个假 herdr 与空着的锁
  When 运行 supervise --no-prompt
  Then 没有任何 agent 调用

Scenario: 持锁的外环得到收件箱与检查单,非持锁者被警告(critical)
  Tags: critical
  Test: outer_resume_briefs_the_holder_and_warns_anyone_else
  Given 黑板上 #7 已有内环 ACK(done)
  When 在 hold 里运行 resume --json;再在 hold 外运行 resume
  Then 前者退出码 0、holder 为真、锁 HELD、收件箱恰有 #7、检查单含 board watch;后者退出码 3,输出含"不要接班"与 #7
