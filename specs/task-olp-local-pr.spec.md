spec: task
name: "多机部署:本地 PR 的状态汇总与推 GitHub 前的采认闸门"
id: task-olp-local-pr
status: implemented
tags: [olp, cluster, git, inbox]
estimate: 0.5d
---

## Intent

单元把 `task/<N>` 推到 hub 的本地仓库,一次推送就是一个本地 PR。现在没有任何地方回答:
哪些本地 PR 在等大外环审核、哪些被退回、哪些已采认但还没推 GitHub、哪些已经在 GitHub 上
等人合并。"大外环通过后才推 GitHub"也只靠自觉。本任务提供状态汇总(给全景状态台的本地
PR 看板用)和推 GitHub 前的采认闸门。

## Decisions

- 工具 `scripts/olp-local-pr.py`(纯 Python 标准库,兼容 Python 3.9),子命令 `status` 与
  `promote`,都要 `--repo <hub.git> --board <黑板>`,可选 `--remote`(默认 `github`)。
  工具不联网;`promote` 之外不写任何东西。
- 状态不另存:由 hub 仓库的引用和黑板上的三种行首定式算出——
  `> 单元(<U>)·本地复验通过(#N): task/N @ <提交号> …`、`> 外环(<署名>)·采认(#N)`、
  `> 外环(<署名>)·打回(#N)`。只认行首定式(先剥 `> `、`**`、空白),代码围栏内不算。
  这三种定式分别由"本地 git 仓库"与"黑板收件箱"两个工作包定义;本工具只依赖行的格式,
  不依赖它们的代码,所以可以独立合并。
- 判定:对每个 `refs/heads/task/<N>`,取板上 #N 的**最后一行**——推送登记或没有记录 →
  `pending`;打回 → `returned`;采认 → 若 `refs/remotes/<remote>/task/<N>` 等于分支头则
  `submitted`,否则 `accepted`。采认之后又有推送登记 → 回到 `pending`。
- 每项输出 `waiting_on`(`outer`/`unit`/`human`)与 `partition`:`pending`、`accepted`、
  `submitted` 是 `inbox`,`returned` 是 `working`,与黑板收件箱的分区一致。
- `promote <N>`:仅当状态为 `accepted` 时执行 `git push <remote> task/N`,随后把
  `refs/remotes/<remote>/task/<N>` 记为分支头(裸仓库推送后不一定更新跟踪引用)。
  `pending`、`returned`、分支不存在 → 退出码 1,不推送;`submitted` → 提示无需再推,退出码
  0;`--dry-run` 只检查。不强推;开 PR(标题、正文)仍由大外环做。
- 警告:分支头不以最近一次推送登记的提交号开头;已采认但跟踪引用是旧提交;板上没有该
  分支的任何记录。文本模式走 stderr,JSON 在 `warnings` 里。
- "已合并"需要查 GitHub,不在本工具范围内。
- 退出码:0 成功;1 `promote` 被闸门拒绝或推送失败;2 用法错误、仓库或黑板不存在。

## Boundaries

### Allowed Changes
- scripts/olp-local-pr.py
- tests/olp_local_pr.rs
- docs/OLP_LOCAL_PR.md
- specs/task-olp-local-pr.spec.md

### Forbidden
- 不改任何现有脚本与协议文档。
- 不调用 GitHub API,不依赖 `gh`。
- `promote` 不得强推,不得推 `task/<N>` 以外的引用。

## Out of Scope

- 开 GitHub PR、判断 PR 是否已合并。
- 把本地 PR 的状态并进 `olp-inbox.py` 的输出(两个工具的分区字段一致,后续合并)。
- 全景状态台的页面。

## Completion Criteria

### Rule: status — 状态判定
Scenario: 每个任务分支的状态由黑板上最后一行决定(critical)
  Tags: critical
  Test: local_pr_status_follows_the_latest_board_line
  Given 四个任务分支:#1 只有推送登记,#2 被打回,#3 已采认,#4 已采认且 GitHub 跟踪引用等于分支头
  When 查看状态;随后 #2、#3 各再推送一次
  Then 状态依次为 pending、returned、accepted、submitted,等待方依次为大外环、单元、大外环、人;再次推送后 #2、#3 回到 pending;`main` 不算本地 PR

Scenario: 对不上的地方作为警告报告
  Test: local_pr_reports_mismatches_as_warnings
  Given #1 的分支被绕过钩子改动,#7 在板上没有任何记录,板上有代码围栏里和正文中间的采认字样
  When 查看状态
  Then 警告指出 #1 的分支头与登记不符、#7 没有记录;围栏内和正文里的字样不算结论,两者都是 pending

### Rule: gate — 推 GitHub 的闸门
Scenario: 推 GitHub 的闸门只放行已采认的分支头(critical)
  Tags: critical
  Test: local_pr_promote_requires_acceptance_of_the_current_head
  Given #1 已推送未审核,#2 被打回,没有 #9
  When 依次对三者 promote;给 #1 写采认后先 dry-run 再 promote,并重复 promote;单元再次推送 #1 后 promote;写第二次采认后 promote
  Then 未采认、被打回、不存在的都以退出码 1 拒绝且 GitHub 上没有对应分支;dry-run 不推;采认后推上去的是分支头且状态变为 submitted;重复 promote 是空操作;采认后的新推送使闸门重新关闭,GitHub 上保持先前采认的提交,并在第二次采认后报告"GitHub 上是旧提交";再 promote 后 GitHub 上是新提交

### Rule: usage — 输入检查
Scenario: 用法错误
  Test: local_pr_rejects_bad_usage
  Given 不存在的仓库、不存在的黑板、非法远程名、缺参数、非数字任务号
  When 分别运行
  Then 都以非 0 退出(用法错误为 2)
