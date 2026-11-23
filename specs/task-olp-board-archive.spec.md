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
- 归档与收件箱、单元占用、派单共用同一份黑板快照的判定。外环采认、改判作废、无单元的卡被打回可归档。
  带单元的卡打回后继续返工。裸 ACK、具名 ACK、单元推送与搁置都保留在活板上,直到有终态判词。
  采认后出现新的 ACK 或推送又回到待办。围栏内示例和以 `## Active` 明确结束的 `## Historical record` 区间不参与判定。老板末尾经写板助手追加的卡与回执继续生效。
  旧式 Markdown 板无需转换;`ACK:` 按既有约定保留原文。旧桩已有的单元与回执仍用于判断是否需要返工或复验。
  `--keep N`(默认 10)原样保留最近 N 张卡。
- 编号桩:原标题行 + 条目里所有围栏外"给机器读"的行(逐字、按原顺序)+
  `<!-- olp-archived: <归档文件名> -->` + 一个空行。给机器读的行是:`ACK(` 行、`unit:` 行
  (hub 的推送钩子在返工推送时读它)、`<!-- olp-key: … -->` 行(写板助手的幂等标记)、紧邻该键前且编号一致的完整 `<!-- olp-dispatch: n=… sha256=… pane=… -->` 派卡绑定,以及
  署名的行首定式——采认、打回、搁置、改判、单元的推送登记、内环的署名 ACK,以及进化环采集在活板上读的
  两种单元信号:负哨(`单元(U)·负哨(`)与单元转发的 MCP blocked/timeout 审计行(先归档后采集也不漏;
  ask、answer 审计行与普通进度行不是信号,随正文归档)。这些判词按 `#N`
  归属,物理上可能落在别的条目下面,不能随那个条目的正文离开黑板,否则收件箱与本地 PR
  状态会把别的卡误判成未审核。进化环采集按 `条目号 + 行内容哈希` 计算的 identity 不变。
  已是桩的条目跳过。
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
- src/olp/archive.rs、src/olp/board.rs(2026-11-23:归档的 Rust 实现;本 spec 的全部场景改为对 `octoscode olp board archive` 运行,输出与退出码不变)
- scripts/olp-board-archive.py
- scripts/olp-inbox.py
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
  Given 一块含前言、`##` 小节和五个条目的板,其中 #1、#5 在 ACK(done) 后获采认,#3 为无单元的卡,在 ACK(wontdo) 后被外环打回,#2 为 ACK(blocked),#4 无 ACK
  When 以 `--keep 0` 运行归档工具
  Then #1、#3、#5 在板上变为"标题 + 原 ACK 和判词 + 归档标记"的桩,#2、#4、前言与小节逐字不变,板上编号仍为 1 到 5;归档文件含三个条目的完整原文

Scenario: 最近的条目原样保留
  Test: board_archive_keeps_the_most_recent_entries_intact
  Given 同一块五条目的板
  When 以 `--keep 2` 运行;以及在另一份拷贝上以默认参数运行
  Then `--keep 2` 时已闭环的 #5 保持原样而 #1、#3 被归档;默认(保留 10 个)时板不变且不产生归档文件

Scenario: 闭环判定只看围栏外本卡的最新有效事件
  Test: board_archive_follows_latest_event_outside_code_fences
  Given 条目分别为:围栏内含示例标题及采认、先采认后 blocked、旧格式 `ACK:`、先 blocked 后 done 并获采认
  When 以 `--keep 0` 运行
  Then 只有最后获得采认的条目被归档,它的桩按原顺序保留两条 ACK 行与 olp-key 标记;其余条目逐字不变

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

Scenario: 多机部署的卡按外环的结论闭环,给机器读的行留在板上(critical)
  Tags: critical
  Test: board_archive_follows_outer_verdicts_and_keeps_machine_lines
  Given 六张卡:#1 已采认,#2 采认后单元又推送,#3 被 #4 改判作废,#5 属于单元,远程 ACK 后被打回,#6 已采认;推送登记与判词都追加在 #6 下面
  When 以 `--keep 0` 运行
  Then 只有 #1、#3、#6 被归档;#1 的桩保留 `unit:` 行与键标记;#6 的桩逐字保留板尾登记与判词;#2、#4、#5 逐字不变;三段正文进入归档文件

Scenario: 采集要读的单元信号留在桩里,先归档后采集也不漏
  Test: board_archive_keeps_unit_signals_for_the_harvest
  Given 一张已采认的卡,条目下有负哨、单元转发的 MCP timeout 与 blocked 行,以及 ask、answer 审计行和一条普通进度行
  When 以 `--keep 0` 运行
  Then 桩逐字保留负哨、timeout、blocked 三行;ask、answer、进度行与正文进入归档文件,不在板上


Scenario: 归档保持收件箱、单元占用和派单结果一致(critical)
  Tags: critical
  Test: board_archive_preserves_open_work_and_unit_occupancy
  Given 单元卡分别为已打回、裸 ACK、远程具名 ACK、已采认、已搁置、旧版误归档后仍待返工;历史区记录重复的采认
  When 以 `--keep 0` 归档,再对返工中的单元派一张新卡
  Then 只归档已采认的正文;待办和单元占用视图逐项保持一致,历史区原样保留,新派单因原卡仍占用而被拒绝


Scenario: 归档桩保留紧邻编号键的原始投递绑定
  Test: board_archive_keeps_only_complete_adjacent_dispatch_bindings
  Given 已采认的卡中有派卡绑定、对应编号键,以及围栏内、引用内、编号不符与未紧邻键的示例
  When 归档该卡
  Then 编号桩保留正确绑定与对应键的原始相邻行,其余示例随正文进入归档文件
