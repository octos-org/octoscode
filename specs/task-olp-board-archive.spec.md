spec: task
name: "黑板归档工具:已闭环条目移入归档文件,板上留编号桩"
id: task-olp-board-archive
status: implemented
tags: [olp, board, archive, cluster]
estimate: 1d
---

## Intent

黑板只追加。集群方案里十个单元向同一块板写入,条目增长很快,而内环每轮任务开始前都要读
全板。本任务提供一个归档工具:把已闭环条目的正文移到归档文件,板上只留编号桩。桩保证三件
事不被破坏:编号连续(写板时的下一个编号仍是板上最大号 +1)、内环"执行最小未 ACK 条目"的
规则(桩带着原 ACK 行)、`#N` 引用可查。

"统一黑板路径"不在本任务内:`AGENTS.md` 已规定 `.octos/OUTER_LOOP_REVIEW.md` 是活板、
`docs/` 下同名文件是冻结快照,路径已经是确定的。

## Decisions

- 工具:`scripts/olp-board-archive.py <board.md> [--archive <file>] [--keep N] [--dry-run]`,
  纯 Python 标准库。属于偶尔运行的本机胶水,按 BOOT 的"原型/发行判据"可以用 Python。
- 条目边界:一个 `### <数字>` 标题到下一个 `### ` 或 `## ` 标题之前的全部行;代码围栏
  (``` 或 ~~~)内的标题不算边界。板的前言、`##` 小节、非编号标题的块原样保留,顺序不变。
  追加在板尾的无标题行在物理上属于最后一个条目,会随该条目一起归档。
- 闭环判定:条目里围栏外最后一条以 `ACK(` 开头的行是 `ACK(done` 或 `ACK(wontdo`。
  `ACK(blocked`、无 ACK、旧格式 `ACK:` 的条目不动。是否已复验由运行工具的持锁外环负责,
  `--keep N`(默认 10)让最近 N 个编号条目不论是否闭环都原样保留。
- 编号桩:原标题行 + 条目里所有围栏外的 `ACK(` 行(逐字、按原顺序)+
  `<!-- olp-archived: <归档文件名> -->` + 一个空行。ACK 行逐字保留,进化环采集按
  `条目号 + 行内容哈希` 计算的 identity 不变,重扫不产生重复卡片。已是桩的条目跳过。
- 归档文件:默认与板同目录的 `<板名去掉 .md>.archive.md`,只追加;每个条目前写
  `<!-- olp-archived-entry: <N> at <UTC 时间> -->`。条目号已在归档文件里的不再写入,
  所以中断后重跑不会重复。
- 写入顺序:先追加归档文件并 fsync,再把新板写到同目录临时文件、fsync、`rename` 原子替换。
  任何一步失败,条目要么两边都有,要么只在板上,不会两边都没有。文件权限位沿用原板。
- 并发:全程持有 `<board>.lock` 的 `flock`(Python `fcntl.flock`,与 `olp-board-append.sh`
  用的 flock(1) 是同一种锁)。内环的原地 ACK 不走这把锁,所以替换前核对板的
  inode/大小/mtime 与读取时一致,不一致则不替换,退出码 4。
- `--dry-run`:只列出会归档的条目;不取锁、不建任何文件。
- 退出码:0 成功(含无可归档);2 用法错误或板不存在;4 归档期间板被改动。
- 测试注入点:环境变量 `OLP_BOARD_ARCHIVE_TEST_CONCURRENT_EDIT` 非空时,工具在最终核对前
  把它的值追加到板上,模拟未持锁的写入方。仅供测试。
- `olp-watch-board.sh` 不改:它在行数变少时已会重取基线。

## Boundaries

### Allowed Changes
- scripts/olp-board-archive.py
- tests/olp_board_archive.rs
- docs/OLP_OUTER_BOOT.md(仅 §7 末尾新增"黑板归档"小节;§0–§6 不动)
- specs/task-olp-board-archive.spec.md

### Forbidden
- 不改 scripts/olp-board-append.sh、scripts/olp-watch-board.sh、scripts/olp-evo-harvest.sh。
- 不改 docs/OLP_OUTER_BOOT.md 的 §0–§6(被 tests/olp_evo_retro.rs 按哈希锁定)。
- 不删除任何条目的标题行与 ACK 行。
- 不写 docs/OUTER_LOOP_REVIEW.md(冻结快照)。

## Out of Scope

- 自动定时归档、按日期或大小触发。
- 归档文件的轮转与异地镜像。
- 从归档文件恢复条目到板上。
- 让内环的原地 ACK 也走 flock。

## Completion Criteria

### Rule: archive — 归档与编号桩
Scenario: 已闭环条目移入归档并留编号桩(critical)
  Tags: critical
  Test: board_archive_moves_closed_entries_and_leaves_numbered_stubs
  Given 一块含前言、`##` 小节和五个条目的板,其中 #1、#5 为 ACK(done),#3 为 ACK(wontdo),#2 为 ACK(blocked),#4 无 ACK
  When 以 `--keep 0` 运行归档工具
  Then #1、#3、#5 在板上变为"标题 + 原 ACK 行 + 归档标记"的桩,#2、#4、前言与小节逐字不变,板上编号仍为 1 到 5;归档文件含三个条目的完整原文

Scenario: 最近的条目原样保留
  Test: board_archive_keeps_the_most_recent_entries_intact
  Given 同一块五条目的板
  When 以 `--keep 2` 运行;以及在另一份拷贝上以默认参数运行
  Then `--keep 2` 时已闭环的 #5 保持原样而 #1、#3 被归档;默认(保留 10 个)时板不变且不产生归档文件

Scenario: 闭环判定只看围栏外的最后一条 ACK
  Test: board_archive_closure_uses_last_ack_outside_code_fences
  Given 条目分别为:围栏内含示例标题与 ACK(done)、先 done 后 blocked、旧格式 `ACK:`、先 blocked 后 done
  When 以 `--keep 0` 运行
  Then 只有"先 blocked 后 done"的条目被归档,它的桩按原顺序保留两条 ACK 行;其余条目逐字不变

### Rule: safety — 幂等与并发
Scenario: 重跑幂等,中断后不重复归档(critical)
  Tags: critical
  Test: board_archive_rerun_is_idempotent
  Given 归档文件里已有 #1(上次运行在替换黑板前中断)
  When 运行归档工具,然后再运行一次
  Then 归档文件里 #1 只有一份;第二次运行报告没有可归档的条目,板与归档文件都不变

Scenario: 归档期间板被改动则不替换(critical)
  Tags: critical
  Test: board_archive_aborts_when_board_changes_underneath
  Given 归档工具读取黑板之后、替换之前,有一行未持锁的写入落到板上
  When 工具执行最终核对
  Then 退出码 4,板保留那一行且没有任何条目被替换成桩,不留临时文件;重跑成功且归档文件不重复

Scenario: 与写板助手共用一把锁
  Test: board_archive_serialises_with_board_append
  Given 30 个进程经 `olp-board-append.sh` 并发追加新条目
  When 同时反复运行归档工具
  Then 每次归档都以 0 退出,30 个追加的条目在板上各出现恰好一次

Scenario: dry-run 不写任何文件
  Test: board_archive_dry_run_writes_nothing
  Given 一块有可归档条目的板
  When 以 `--dry-run` 运行
  Then stdout 列出会归档的条目,板不变,目录里没有锁文件与归档文件

Scenario: 用法错误不触碰任何文件
  Test: board_archive_rejects_bad_usage
  Given 不存在的板、负的 `--keep`、把归档文件指向黑板本身、未知参数
  When 分别运行归档工具
  Then 退出码均为 2,板不变,没有产生锁文件与归档文件
