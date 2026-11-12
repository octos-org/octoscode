spec: task
name: "多机部署:hub 本地 git 仓库与推送规则(本地 PR 的入口)"
id: task-olp-local-git-hub
status: implemented
tags: [olp, cluster, git, hub]
estimate: 1d
---

## Intent

多机部署的工作流是三道关:单元内复验 → 大外环审核 → 人审核。单元不直接推 GitHub,只向 hub
上的本地仓库推 `task/<N>`,一次推送就是一个本地 PR。这道门如果只靠约定,一条错推就会污染
集成分支,"本地复验没问题才可以推"也无从保证。本任务提供 hub 仓库的初始化、服务端钩子、
ssh 强制命令和单元侧的推送命令,并把三道关的规则写进协议附录。

## Decisions

- hub 侧工具 `scripts/olp-git-hub.py`(纯 Python 标准库,兼容 Python 3.9),子命令 `init`、
  `serve`、`hook`;单元侧 `scripts/olp-task-push.sh`。
- `init <裸仓库> --board <黑板>`:建裸仓库(已存在则只刷新),写 `olp.board`、
  `olp.scripts`、`receive.advertisePushOptions=true`,安装 `pre-receive`/`post-receive`
  两个钩子(单行 shim,调用本脚本的 `hook`)。目标已存在但不是裸仓库、或黑板不存在时退出码
  2 且不创建任何东西。
- 单元身份:来自环境变量 `OLP_UNIT`,由 ssh 强制命令 `serve --unit <U> --repo <R>` 设置。
  `serve` 只放行 `git-receive-pack`/`git-upload-pack`(含 `git receive-pack` 写法)且路径
  解析后必须等于 `<R>`,其余命令退出码 3。客户端自带的 `OLP_UNIT` 被强制命令覆盖。
- 不带 `OLP_UNIT` 的推送视为 hub 本机上大外环的操作,不受限制(集成分支、种子、标签由它
  维护),也不登记到黑板。因此单元账号必须走强制命令——写入文档。
- 推送规则(`pre-receive`,任一 ref 违规则整次推送被拒):
  1. ref 必须是 `refs/heads/task/<数字>`;不能删除。
  2. 黑板上必须有条目 `#N`,且条目内(代码围栏外)最后一行 `unit: <U>` 等于推送方的单元
     号。条目没有 `unit:` 行视为未指定单元,拒收。
  3. 必须有 push option `olp-verified=<sha>:<署名>`,`sha` 等于该 ref 的新提交号,署名匹配
     `[A-Za-z0-9][A-Za-z0-9._@-]{0,63}`。
  4. 非快进推送不额外限制:分支只有被派到的单元能推,返工与 rebase 需要覆盖。
- 本地复验记录选用 push option 而不是提交尾注:尾注要求复验后改写提交,复验过的就不再是推
  上来的那个提交了。
- `post-receive`:对每个被接收的任务分支,经 `olp-board-append.sh`(flock 互斥)在黑板末尾
  追加 `> 单元(<U>)·本地复验通过(#N): task/N @ <sha 前 12 位> 复验 <署名>`。行首
  `> 单元(` 与外环的签名批注 `> 外环(` 不同,不触发进化环采集。追加失败不回滚推送,在推送方
  输出里告警。调用写板脚本前清掉钩子环境里的 `GIT_*` 变量。
- `olp-task-push.sh <N> --verified-by <署名> [--remote hub] [--force]`:取本地 `task/<N>`
  的分支头,带上对应的 push option 推送。任务号非数字、缺署名、署名非法、分支或远程不存在
  时退出码 2,不发起推送。
- 派卡字段 `unit: <单元号>` 在本任务里定义并由钩子读取;占用判断等更多用法属于后续工作包。
- 协议:`docs/OUTER_LOOP_PROTOCOL.md` 新增附录 C(三道关、权限划分、两块板与 `#N-rM`
  返工编号、推送规则、本地复验记录的效力),保持 `olp/v2`。上岗卡里"只 commit 不 push"的
  措辞本任务不改(另一个未合并分支正在改同一段),附录 C 给出多机下的读法。
- 局限(写入协议与 PR):本地复验记录不能证明复验独立;大外环不得因此省略 R2。

## Boundaries

### Allowed Changes
- scripts/olp-git-hub.py
- scripts/olp-task-push.sh
- tests/olp_git_hub.rs
- docs/OUTER_LOOP_PROTOCOL.md(仅文末新增附录 C)
- docs/OLP_LOCAL_GIT.md
- specs/task-olp-local-git-hub.spec.md

### Forbidden
- 不改 R1–R7 的条文与协议版本号。
- 不改 docs/OLP_OUTER_BOOT.md、docs/OCTOLOOP_AGENTS*.md、AGENTS.md。
- 不改现有脚本。
- 钩子不得修改被推送的内容。

## Out of Scope

- 本地 PR 的状态汇总与推 GitHub 前的采认检查(下一个工作包)。
- 单元占用判断与回测派发。
- 单元本机板的工具支持(`#N-rM` 目前是书写约定)。
- 核对单元是否在登记文件里(登记工具在另一个未合并的分支)。

## Completion Criteria

### Rule: accept — 合规的推送
Scenario: 派给本单元且带复验记录的推送被接收并登记到黑板(critical)
  Tags: critical
  Test: hub_accepts_verified_push_from_assigned_unit_and_logs_board_line
  Given 黑板上 #1 带 `unit: m07`,单元 m07 的工作仓库里有分支 task/1
  When 以 `olp-task-push.sh 1 --verified-by m07-ver` 推送
  Then hub 仓库的 task/1 等于本地分支头,黑板末尾恰好多出一行本地复验通过的登记

Scenario: 返工后同一单元可以强推自己的任务分支
  Test: hub_allows_rework_force_push_by_owner
  Given task/1 已推送,随后本地改写了提交
  When 不带 `--force` 再推;再带 `--force` 推
  Then 第一次失败且 hub 上的分支不变;第二次成功,黑板上有两条登记,分别对应两个提交

Scenario: hub 本机不带单元号的推送不受限制
  Test: hub_local_pushes_without_unit_are_unrestricted
  Given 没有设置单元号
  When 推送 main、一个标签和 integration 分支
  Then 全部被接收,黑板不变

### Rule: reject — 违规的推送
Scenario: 违反推送规则的推送被拒收(critical)
  Tags: critical
  Test: hub_rejects_pushes_that_break_the_rules
  Given 单元 m07,黑板上 #1 派给 m07、#2 派给 m08、#3 只在代码围栏里出现 unit 行、#4 先派 m07 后改派 m08、没有 #9
  When 分别推送:集成分支、其他分支、非数字任务分支、标签、#9、#2、#3、#4、不带复验记录的 #1、记录指向别的提交的 #1、署名非法的 #1;一次推送里混入一个违规分支;删除已推送的 task/1
  Then 每次都被拒收并给出对应原因,hub 上没有出现任何违规的引用,黑板不变;混入违规分支的推送整体被拒;删除被拒且分支保留

Scenario: ssh 强制命令只放行对 hub 仓库的 git 操作并固定单元号(critical)
  Tags: critical
  Test: hub_serve_pins_the_unit_and_only_allows_git_on_the_hub_repo
  Given 强制命令 `serve --unit m07 --repo <hub>`
  When 原始命令为空、普通命令、带额外命令的 git 命令、指向别的仓库、git-upload-archive;以及经假 ssh 推送 #2(派给 m08,客户端自称 m08)与 #1,并拉取 main
  Then 前五种以退出码 3 拒绝;#2 被拒(钩子看到的单元是 m07),#1 被接收,拉取成功

### Rule: tooling — 初始化与单元侧命令
Scenario: 单元侧推送命令先做本地检查
  Test: task_push_checks_inputs_before_pushing
  Given 本地只有分支 task/1
  When 缺 `--verified-by`、任务号非数字、分支不存在、署名非法、远程不存在、未知参数
  Then 都以退出码 2 结束,hub 上没有 task/1

Scenario: init 可重跑且拒绝非裸仓库
  Test: hub_init_is_rerunnable_and_refuses_a_non_bare_repo
  Given 一个已初始化并有引用的 hub 仓库
  When 重跑 init;对一个普通工作仓库 init;以不存在的黑板 init 新路径
  Then 重跑成功且引用不变、两个钩子可执行;后两种退出码 2,新路径没有被创建
