spec: task
name: "OctoLoop 单元:主持 agent 的保活(herdr 重启、agent 退出之后拉起)"
id: task-olp-unit-host
status: implemented
tags: [olp, unit, herdr, keepalive]
estimate: 0.5d
---

## Intent

外环死了有监督任务拉起(`olp outer supervise`),单元一侧没有:herdr 重启、主持 agent 崩溃或
机器重启之后,主持的窗格里只剩一个 shell。单元值守(`olp remote watch`)只会向 hub 报告
"窗格接不了卡",不会把主持叫回来,在手的卡就停在那里。本任务补上单元一侧的保活:窗格里没有
程序在跑时,在原窗格重新启动主持,并送一句接班提示。

## Decisions

- 命令 `octoscode olp unit host --pane <窗格> | --label <工作区标签> [--cwd 目录]
  [--session <herdr 会话>] [--ready 90] [--max-restarts 3] [--window 1800]
  [--prompt 话 | --no-prompt] [--reset] -- <主持 agent 的启动命令…>`,在单元上跑,
  一般由常驻任务每分钟跑一次(`olp keepalive install unit-host --interval 60 -- …`)。
- 只看窗格前台有没有 shell 以外的程序(与 `olp outer supervise` 判断外环窗格的办法相同):
  有就什么都不做,不管它是不是主持、在做什么;没有才启动。不读卡,不判断任何事。
- `--pane` 与 `--label` 给且只给一个。`--pane` 用派卡时 `--wake` 的那个窗格;`--label` 每次
  按标签找工作区的第一个窗格,工作区不存在时新建。
- 启动后等窗格前台出现程序(最多 `--ready` 秒,上限 20 秒),再等 herdr 认出窗格里的 agent
  (刚启动的程序 herdr 还不认,`agent wait` 会立刻失败;2026-11-27 实测)并且空闲,送
  接班提示:先运行 `octoscode olp remote inbox <窗格>` 看待领取和在手的卡,在手的接着办。
  提示没送到只在 stderr 提醒:主持已在跑,单元值守的 `--follow-up` 会再请它接着办。
- 不登记 herdr 的恢复命令:保活任务一分钟内就会拉起,而且不依赖 herdr 的版本。agent 自带的
  herdr 集成若让 herdr 自己恢复了会话,保活看到窗格里有程序,不再重复启动。
- `--window` 秒内已重启 `--max-restarts` 次就暂停(退出码 4,不再启动),看过后带 `--reset`
  跑一次恢复。重启记录在 `~/.octos/outer/unithost/`(`OLP_UNIT_HOST_STATE` 可改)。
- 内环不由它恢复:主持接班后按规程重新组织。内环没了这件事由单元守护的告警报给 hub,
  外环值守会为它叫醒外环(见 task-olp-outer-watch)。

## Out of Scope

- 恢复内环的窗格和会话。
- 向 hub 报告"重启已暂停":待领取的卡由单元值守报告送达失败。

## Completion Criteria

Scenario: 窗格里没有程序时在原窗格重新启动主持并送接班提示;已在跑就不动(critical)
  Tags: critical
  Test: a_host_whose_program_is_gone_is_started_again_and_told_to_take_over
  Given 主持的窗格里只有 shell
  When 跑一次 `olp unit host --pane w3:p2 -- claude --continue 'two words'`,再跑一次
  Then 第一次在该窗格执行原样加引号的启动命令,送一句指向 `olp remote inbox w3:p2` 的接班提示,退出码 0;第二次报告在跑,不再启动、不再发话

Scenario: 窗格里有别的程序在跑时不启动、不发话
  Test: a_pane_with_a_program_in_it_is_left_alone
  Given 窗格前台有一个不是 shell 的程序
  When 跑一次保活
  Then 退出码 0,没有启动命令,没有接班提示

Scenario: 按标签找窗格,工作区不存在时新建;--no-prompt 不发话
  Test: the_host_pane_is_found_or_created_by_its_workspace_label
  Given herdr 里没有标签为 unit-host 的工作区
  When 以 `--label unit-host --no-prompt` 跑一次
  Then 新建工作区,在它的窗格里启动主持,不发接班提示

Scenario: 主持一起来就退出时,重启到上限后暂停;--reset 后恢复
  Test: restarts_pause_when_the_host_keeps_dying
  Given 启动命令执行后窗格里仍没有程序,`--max-restarts 2`
  When 连跑三次;排除故障后带 `--reset` 再跑一次
  Then 前两次退出码 1,第三次退出码 4 且不再启动;`--reset` 后正常拉起

Scenario: 参数不全时退出码 2,不碰 herdr
  Test: a_host_without_a_pane_or_a_command_is_a_usage_error
  Given 不给窗格、同时给窗格与标签、不给启动命令、窗格标识不合法
  When 分别运行
  Then 都是退出码 2,没有启动任何命令;启动命令里的 `-h` 属于主持,不当作本命令的帮助
