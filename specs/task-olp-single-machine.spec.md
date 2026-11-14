spec: task
name: "单机部署:hub 与单元在同一台机器,集群工具不经 ssh 照常工作"
id: task-olp-single-machine
status: implemented
tags: [olp, cluster, single-machine]
estimate: 1d
---

## Intent

单机使用应当是集群的一个特例:一台机器既是 hub 又是一台单元。此前集群的每条信道都经
ssh(强制命令是信任锚),单机上要么开 sshd 给自己发密钥,要么整套集群工具都用不上。本任务
让 hub 一侧的处理程序(与 ssh 强制命令同一段代码、同样的规则)在本机直接运行,只换传输,
不换规则。

## Decisions

- 单元一侧把 hub 写成 `local`(`unit.conf` 的 `hub = local`)。`olp-board-remote.py` 在
  本机以子进程运行 `serve --unit <单元号> --board <板>`,把动作放进 `SSH_ORIGINAL_COMMAND`,
  与 ssh 强制命令收到的完全一样。此时黑板的绝对路径来自 `board=`(或 `OLP_BOARD`),缺失
  时退出码 2。镜像路径恰好就是这块黑板时,`mirror` 什么也不做并说明原因——否则会把权威
  黑板改成只读。
- hub 判断"这台单元就是本机":登记的主机名等于本机主机名(`olp-unit.py where <单元>`
  输出 `local`/`remote`)。据此,`olp-answer.py` 在本机运行单元一侧的 `serve`(不经 ssh,
  也可显式 `--to local`),`olp-dispatch.py --wake` 调用 `herdr agent prompt` 时不带
  `--machine`。单元一侧 `serve` 的信箱根目录与 MCP 服务一致,认 `OLP_MCP_OUTER_ROOT`。
- git:`olp-git-hub.py connect-local --unit <单元> --repo <裸仓库>` 在单元的工作仓库里
  执行一次,把 hub 远程指向本机的裸仓库,并把 `remote.<名>.receivepack`/`uploadpack` 设成
  `olp-git-hub.py local-pack`;后者把 git 的调用改写成 `SSH_ORIGINAL_COMMAND` 后进入同一个
  `serve`,单元号由这里给出,推送规则与钩子完全不变。
- 心跳:两个保活工具的 `heartbeat --hub local` 生成
  `olp-unit.py heartbeat … | olp-unit.py receive --unit <单元>`,不经 ssh。
- 不做身份隔离:同一个操作系统用户下,单元号来自配置,直接 `git push <裸仓库路径>` 也能
  绕开 `serve`。单机模式防的是误操作(内环推错分支、给自己写采认),不防有意绕开;要真正
  的隔离就用两台机器或两个用户加 ssh。文档里写明。

## Boundaries

### Allowed Changes
- scripts/olp-board-remote.py
- scripts/olp-unit.py
- scripts/olp-git-hub.py
- scripts/olp-answer.py
- scripts/olp-dispatch.py
- scripts/olp-launchd.py
- scripts/olp-systemd.py
- tests/olp_single_machine.rs
- docs/OLP_CLUSTER.md、docs/OLP_REMOTE_BOARD.md、docs/OLP_LOCAL_GIT.md、docs/OLP_KEEPALIVE.md
- specs/task-olp-single-machine.spec.md

### Forbidden
- 不改 hub 一侧的规则(写板限制、推送规则、回答的时限)。
- 不改 ssh 模式的行为:没写 `local` 的配置与登记在别的机器上的单元照旧走 ssh。

## Out of Scope

- 单机上的身份隔离(多用户、容器)。
- 一台机器上跑多台"单元"(多个单元号共用一台机器的主机名时,`where` 都会判为 local,
  能用,但占用与健康都按单元号各自计算,没有专门测试)。

## Completion Criteria

Scenario: hub = local 时写板、ACK、镜像都在本机完成,规则照旧(critical)
  Tags: critical
  Test: single_machine_board_writes_go_through_the_hub_rules_without_ssh
  Given `unit.conf` 写着 `hub = local`、黑板路径,镜像路径就是黑板本身;ssh 是一个记录调用并失败的替身
  When 写 ACK、带署名追加、伪造采认、自己立卡、同步镜像、同步到另一个路径、去掉 `board=` 再写
  Then ACK 与进度落到黑板;伪造采认与立卡以退出码 3 拒绝;镜像什么也不做且黑板仍可写;另一个路径得到一致的副本;缺 `board=` 时退出码 2;ssh 一次也没被调用

Scenario: 本机的推送经同一个 serve,推送规则照旧(critical)
  Tags: critical
  Test: single_machine_pushes_go_through_serve_with_the_unit_id
  Given 本机的 hub 裸仓库,和一个经 `connect-local --unit m01` 接上的工作克隆
  When 不带复验记录推 `task/1`、推到 `main`、由复验方推送、拉取、对不存在的仓库 connect-local
  Then 前两次被钩子拒收;复验方的推送成功且黑板出现 `单元(m01)·本地复验通过(#1)`;拉取成功;不存在的仓库以退出码 2 拒绝

Scenario: hub 回答本机单元的提问、唤醒本机的窗格,不经 ssh(critical)
  Tags: critical
  Test: single_machine_answers_and_wakes_reach_the_local_unit
  Given m01 以本机主机名登记,m02 登记在别的主机名;m01 的信箱里有一条在等的提问
  When `where` 两台单元与一个未登记的;列出并回答 m01 的提问;列出 m02 的提问;给两台单元各派一张带唤醒的卡
  Then m01 为 local、m02 为 remote、未登记的退出码 2;m01 的回答写进信箱且没有调用 ssh;m02 走 ssh;唤醒 m01 时 herdr 不带 `--machine`,唤醒 m02 时带

Scenario: 心跳任务在单机部署时直接交给本机的 receive
  Test: single_machine_heartbeat_job_hands_the_beat_to_the_local_hub
  Given 两个保活工具
  When 以 `--hub local` 注册心跳任务
  Then 任务的命令是 `olp-unit.py heartbeat … | olp-unit.py receive --unit m01`,不含 ssh
