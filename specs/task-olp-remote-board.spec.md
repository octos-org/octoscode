spec: task
name: "多机部署:单元远程写板、离线队列与只读镜像"
id: task-olp-remote-board
status: implemented
tags: [olp, cluster, board, ssh, queue]
estimate: 1d
---

## Intent

多机部署时权威黑板只在 hub 上。单元上的内环需要写 ACK、报进度、把提问送到大外环,但不能
直接写那个文件。本任务让单元的写入经 ssh 在 hub 本机、在 flock 下完成;hub 不可达时写入
排队并在恢复后按顺序补写,不重复;单元读的是本机的只读镜像。同时在 hub 侧挡住单元伪造
外环判词——后续的"推 GitHub 前检查采认"信任黑板上的采认行,这道门必须有。

## Decisions

- 工具 `scripts/olp-board-remote.py`(纯 Python 标准库,兼容 Python 3.9)。单元侧子命令
  `append`、`ack`、`flush`、`queue`、`mirror`;hub 侧子命令 `serve`(ssh 强制命令)。
- 传输:单元执行 `ssh -o BatchMode=yes -o ConnectTimeout=10 <hub> append <键>`(正文走
  stdin)或 `… read`。hub 的 `authorized_keys` 用强制命令
  `serve --unit <U> --board <B>` 接管,黑板路径与单元号由 hub 决定。`OLP_SSH` 可替换 ssh
  程序(测试用)。
- `serve` 的规则(违反即退出码 3,不写板):动作只能是 `append <键>` 或 `read`;键必须匹配
  `<U>:[A-Za-z0-9._:/@+=-]{1,180}`;正文非空、UTF-8、不超过 256 KiB;正文任何一行在剥掉
  `> `、`**` 与空白后不得以 `外环(` 开头;任何一行不得以 `### <数字>` 或 `### {N}` 开头。
  通过后调用同目录的 `olp-board-append.sh --key <键> <B>`,stdout 与退出码原样返回。
- 键:单元侧给每次写入生成 `<U>:<后缀>`。不带 `--key` 时后缀是正文 sha256 的前 16 位,
  所以同一段正文重发由 hub 去重;带 `--key` 时自动补上单元前缀。
- ACK:`ack <N> done|wontdo|blocked <说明>` 追加
  `> 内环(<U>)·#<N> ACK(<kind>): <说明>`。远程写板只能追加,无法在条目下面原地写,所以用
  带条目号的署名写法;黑板收件箱按 `#N` 归属。说明必须非空、单行。
- 离线队列:ssh 退出码 255 或超时视为 hub 不可达——写入存为
  `${OLP_QUEUE_DIR:-~/.octos/outer/queue}/<纳秒时间戳>-<键哈希>.entry`(首行是键,其后是
  正文),命令退出码 75;同一个键已在队列里则不重复入队。每次 `append`/`ack` 先按文件名
  顺序重发队列,队列发不完则新写入也入队,保证顺序。重发时被 hub 拒绝的条目改名为
  `.rejected` 并跳过。hub 明确拒绝(退出码非 255)的新写入不入队。
- "hub 已写入但回执丢失":客户端按不可达处理并保留队列项,之后重发由 hub 的键去重吸收,
  板上只出现一次。
- 只读镜像:`mirror --to <路径>` 经 `read` 取全板,写临时文件、设为 0444、原子替换。hub
  不可达时退出码 75,镜像不变。
- 配置优先级:命令行 > 环境变量(`OLP_HUB`、`OLP_UNIT_ID`)> 配置文件
  `${OLP_UNIT_CONF:-~/.octos/outer/unit.conf}`(`hub=`、`unit=`)。hub 目标必须以字母或
  数字开头且只含 `[A-Za-z0-9._@-]`。缺配置时退出码 2,不发起连接。
- MCP 提问到达 hub:`append` 忽略多余的位置参数,所以单元上的
  `~/.octos/outer/board_append.sh` 可以写成对它的一行转发,`ask_outer`/`report_blocked` 的
  审计行就落到 hub 的板上。回答的回传不在本任务内(写入文档)。
- 跨机唤醒用 herdr 自带的 `--machine`,只写进文档,不包脚本。
- 退出码:0 成功;2 用法或配置错误;3 hub 拒绝;75 hub 不可达(已排队或镜像未更新);其余
  为 hub 上写板脚本的退出码。

## Boundaries

### Allowed Changes
- scripts/olp-board-remote.py
- tests/olp_board_remote.rs
- docs/OLP_REMOTE_BOARD.md
- specs/task-olp-remote-board.spec.md

### Forbidden
- 不改 scripts/olp-board-append.sh 与其他现有脚本。
- 不改 src/olp_mcp.rs(MCP 的落板钩子保持调用 `board_append.sh`)。
- 不把黑板放到网络文件系统上。

## Out of Scope

- 把 MCP 的回答送回单元的信箱。
- 进化环采集识别署名写法的 ACK。
- 了结时清理单元上的工作树。
- 队列的定时重发(用保活工具注册 `flush`)。

## Completion Criteria

### Rule: write — 远程写入
Scenario: 单元的写入落到 hub 的黑板,重发不重复(critical)
  Tags: critical
  Test: remote_append_reaches_the_hub_board_and_is_idempotent
  Given hub 可达,黑板上有 #3
  When 追加一段正文,再原样追加一次;带显式键追加;执行 `ack 3 done …`
  Then 正文与带单元前缀的键标记出现在板上,第二次追加不改变黑板;显式键被补上单元前缀;ACK 以 `> 内环(m07)·#3 ACK(done): …` 的形式落板;队列为空

Scenario: hub 不可达时排队,恢复后按顺序补写且不重复(critical)
  Tags: critical
  Test: remote_writes_queue_while_the_hub_is_unreachable_and_replay_in_order
  Given hub 不可达
  When 追加第一条、ack 一条、重复追加第一条;随后 hub 进入"写入成功但回执丢失"状态并 flush;最后 hub 恢复,追加第三条
  Then 不可达期间三次命令都以退出码 75 结束、队列里是两条、黑板不变;回执丢失后第一条已在板上但仍留在队列;恢复后队列清空,板上第一条只出现一次,三条内容按原顺序排列

### Rule: guard — hub 侧的限制
Scenario: hub 拒绝伪造权威的写入和别的单元的键(critical)
  Tags: critical
  Test: hub_serve_rejects_forged_authority_and_foreign_keys
  Given 强制命令 `serve --unit m07`
  When 用别的单元的键、无前缀的键、含空格的键;正文含外环采认、带引用与加粗的外环改判、编号标题、`{N}` 标题、只有空白;动作不是 append/read
  Then 都以退出码 3 拒绝且黑板不变;经客户端发出的伪造采认同样被拒且不入队;行中间提到外环的正文被接受

### Rule: mirror — 只读镜像
Scenario: 只读镜像
  Test: remote_mirror_is_a_read_only_copy_of_the_hub_board
  Given hub 上的黑板
  When 拉取镜像;hub 的黑板变化后再拉取;hub 不可达时拉取
  Then 镜像内容等于黑板且权限为 0444;第二次拉取替换了只读文件;不可达时退出码 75 且镜像保持原样

### Rule: config — 配置与输入
Scenario: 配置缺失或输入非法时不发起连接
  Test: remote_requires_configuration_and_valid_input
  Given 没有任何配置;只有 hub;hub 写成 ssh 选项的样子;以及一份配置文件
  When 分别运行 append;在有配置文件时给出空正文、非法键、非法 ACK 种类、非数字条目号、缺说明、缺 `--to`、未知子命令
  Then 都以退出码 2 结束,黑板不变、队列为空;只凭配置文件的一次正常追加成功
