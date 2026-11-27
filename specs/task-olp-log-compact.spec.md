spec: task
name: "OctoLoop 事件日志的压缩:已了结的卡移入归档,日常命令只读还需要的部分"
id: task-olp-log-compact
status: implemented
tags: [olp, store, jsonl, compaction]
estimate: 2d
---

## Intent

每条命令都重放整个事件日志,所以日志多大,每条命令就多慢、多占内存(实测 20 万条事件 0.13 秒、
224 MB;100 万条 0.68 秒、1 GB)。2026-11-28 operator 定:做归档,按业内成熟做法。采用事件溯源
存储的通行办法——快照加退役旧分段(Raft / etcd 的日志压缩、Redis 的 AOF 重写、数据库的检查点
同属这一类):把还需要的事件写成一份基线,基线之前的分段原样搬进归档,日志在新分段里接着写。
没有采用按键只留最新一条(Kafka 式压缩):一张卡的状态由多条事件共同决定。也没有另建索引库:
那是换存储。

## Decisions

- 基线 `base.jsonl`(`src/olp/log.rs`):第一行是说明(`upto` 截止序号、`segment` 日志接着写的
  分段、`made`、`max_card` 发出过的最大卡号、`dropped` 累计移走的事件数),随后是只在归档里的
  卡(卡号、单元、标题、立卡的键、首末序号、末条时间),再后是留下的事件——保持原序号、原内容。
  读日志 = 基线留下的事件 + `upto` 之后的分段;序号只增不重排,留下的事件之间有空洞,各处
  值守记的"读到第几条"不用改。
- 归档不改写历史:分段原字节改名搬到 `<日志目录>/archive/`,分段编号不复用。归档 + 在用的
  分段 = 完整历史,序号从 1 连续(`Log::history`,`olp log history`)。
- 崩溃安全:新基线写到旁边的文件、落盘、原子改名覆盖旧基线,之后才搬分段。中途断在改名
  之后:基线已覆盖的分段还在原处,读的一方按基线的 `segment` 跳过,下次压缩再搬走。
- 并发:压缩与所有写入共用目录锁。不拿锁的读在开头与结尾各看一次基线是哪一份,不同(或
  要读的分段刚被搬走)就从新基线重读;拿着"读到哪里"的增量读遇到新基线同样从头读。
- 哪些移走(`src/olp/compact.rs` 的 `plan`):卡已了结、最后一条事件早于 `--keep-days`(默认 14)
  天、不在最近了结的 `--keep`(默认 10)张之内、单元已领取(还在单元投递收件箱里的不移)、
  也不是某张留下的卡的回测对象——这样的卡全部事件移走;与卡无关的事件过了 `--keep-days`
  天也移走,还没回答的提问除外。没有派卡事件的卡号(悬空的判词)不动。时间读不出的事件
  不算旧。
- 移走的卡留下的三件事:卡号不再发出(`max_card`);立卡的键仍指那张卡(`card open`、
  `card dispatch` 用同一个键得到原卡号,不新建);之后再写这张卡被拒收——外环的判词退出码 1,
  单元经 hub 强制命令的写入被拒,提示"已了结并归档"。给归档的卡派回测卡仍可以,按归档里
  记的单元判断"不能派给原单元"。与卡无关的旧事件移走后,它们的幂等键不再记得:14 天后的
  重发不算重试。
- 先核对再替换(`same_reading`):用留下的事件重算,与完整日志逐项比较——每张留下的卡的
  分区与原因、未答的提问、每个单元的待办与在办、最大卡号;要归档的卡必须都已了结。有一项
  不同就不写(退出码 5)。
- `octoscode olp log compact <日志目录> [--keep-days N] [--keep N] [--if-over MiB] [--dry-run]
  [--json]`:`--if-over` 是给定时任务的——在用的日志不到这么多、或不到上次压缩留下的两倍时
  什么都不做(Redis 重写 AOF 的触发规则)。不需要主审权租约:压缩不改变任何结论。
- 自动触发:外环值守(`olp outer watch`)每轮看一眼在用日志的大小,超过 `OLP_LOG_COMPACT_MIB`
  (默认 32,0 为不压缩)且达到上次压缩留下的两倍就压缩(保留 14 天、最近 10 张);试过之后
  要再长四分之一才再试,不会每轮重读。
- `board import` 对压缩过的日志同样拒绝(归档里有事件)。
- 外环值守自动压缩时保留的天数与张数可用 `OLP_LOG_COMPACT_KEEP_DAYS`(默认 14)、
  `OLP_LOG_COMPACT_KEEP`(默认 10)改。`tuned` 事件(旋钮的值,`task-olp-log-evolve`)不随
  旧事件移走。

## Boundaries

### Allowed Changes
- src/olp/log.rs、src/olp/compact.rs、src/olp/mod.rs、src/cmd/olp.rs
- src/olp/card.rs、src/olp/dispatch.rs、src/olp/serve.rs、src/olp/import.rs、src/olp/outerwatch.rs
- tests/olp_log_compact.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-log-compact.spec.md、specs/task-olp-event-log.spec.md、specs/task-olp-outer-watch.spec.md

### Forbidden
- 不改写、不删除任何已写入的事件;不重排序号;不复用卡号与分段编号。
- 不引入数据库依赖。

## Out of Scope

- 单元一侧的事件缓存(`<镜像>.events.jsonl`)不单独压缩;hub 归档了缓存里最后一条事件时,
  下次同步按 hub 现有的内容重建(`task-olp-unit-client`)。
- 归档的压缩存放(gzip/zstd)与归档的过期删除。
- 派到窗格却从未被领取的卡了结后仍留在单元的投递收件箱里(`cards::pending_for` 的现状),
  所以这类卡不归档;要不要改投递收件箱的规则另议。

## Completion Criteria

Scenario: 已了结的卡移入归档,其余的读起来与原来一样(critical)
  Tags: critical
  Test: settled_cards_move_to_the_archive_and_the_rest_reads_the_same
  Given 三张卡:一张已采认、一张待复验、一张在做
  When 压缩(保留 0 天、最近 0 张)
  Then 只归档已采认的那张;其余卡在收件箱里的条目逐项不变;分段原样在 archive/ 里,完整历史与压缩前相同;新立的卡取下一个卡号,历史序号从 1 连续;再压缩一次什么都不变

Scenario: 归档的卡保留卡号与立卡的键,之后的写入被拒收(critical)
  Tags: critical
  Test: an_archived_card_keeps_its_number_and_key_and_takes_no_more_writes
  Given 一张已归档的卡
  When 用同一个键再立卡;外环打回它;它的单元经 hub 的强制命令补一条回执
  Then 立卡得到原卡号且不新建;打回退出码 1、单元的写入被拒,都提示已了结并归档;没有新事件

Scenario: 最近了结的、还不够久的、被回测的、单元没领取的卡都留下
  Test: recent_cards_and_cards_under_retest_stay
  Given 一张刚了结的卡
  When 按默认参数、只放宽天数、只放宽张数分别压缩;再给它派一张回测卡后压缩;回测卡了结但单元没领取时压缩;领取后压缩
  Then 前三次都不压缩、不产生基线;回测卡未了结或未领取时两张都不归档;领取后两张都归档

Scenario: 试运行与体积不够时什么都不写
  Test: a_dry_run_and_a_small_log_write_nothing
  Given 一张可归档的卡
  When 带 --dry-run 压缩;带 --if-over 1 压缩
  Then 试运行说出会归档哪张卡但不写;体积不够时说明跳过;没有基线、没有 archive/

Scenario: 没回答的提问不随旧事件归档
  Test: an_open_question_is_not_archived_with_old_events
  Given 单元的两个提问,一个已回答、一个没有
  When 压缩(保留 0 天)
  Then 已回答的提问与它的回答移走;没回答的仍在收件箱的待答提问里,内容不变

Scenario: 外环值守在日志长大后自己压缩,不重复尝试
  Test: the_outer_watcher_compacts_a_log_that_grew_large
  Given 12 张 30 天前了结的卡、一张在做的卡,日志超过 1 MiB
  When 值守在阈值为 0、为 1 MiB 时各跑一轮,再跑一轮
  Then 阈值为 0 时不动;为 1 时压缩并归档最早的 2 张,最近 10 张与在做的卡仍在,完整历史条数不变;第三轮不再压缩

Scenario: 压缩后的日志保持原序号,完整历史一条不少(critical)
  Tags: critical
  Test: a_compacted_log_keeps_its_numbers_and_its_whole_history
  Given 一个跨了多个分段的日志
  When 只留其中两条压缩,追加,再压缩,再追加;最后全部保留地压缩一次
  Then 留下的事件与写入时逐字节相同;分段都进了归档,新事件接着编号、写在新分段;完整历史始终从 1 连续;全部保留时什么都不变

Scenario: 基线留下的事件的幂等键仍然有效
  Test: a_key_the_base_kept_is_still_written_once
  Given 压缩后只留下带键的一条事件
  When 用同一个键再追加;用新键追加
  Then 前者不写入、返回原事件;后者取下一个序号

Scenario: 压缩断在新基线之后,不丢任何东西(critical)
  Tags: critical
  Test: a_compaction_cut_short_after_the_new_base_loses_nothing
  Given 压缩完成后把归档里的分段放回原处(等同于改名之后、搬走之前崩溃)
  When 读、追加、读完整历史、再压缩
  Then 读到的仍是基线留下的事件;追加接着编号;完整历史从 1 连续;下一次压缩把这些分段搬走

Scenario: 被压缩打断的增量读从新基线重读
  Test: a_read_a_compaction_crossed_starts_again_on_the_new_base
  Given 一次读到第 3 条的增量读
  When 日志被压缩并追加之后接着读,再追加再读
  Then 结果与从头读一致,之后照常接着读

Scenario: 要保留的事件必须是日志里的事件
  Test: what_a_compaction_keeps_must_be_in_the_log
  Given 一个三条事件的日志
  When 压缩时给出改过内容的事件、或顺序颠倒的事件
  Then 拒绝,不产生基线,日志不变

Scenario: 基线记得只在归档里的卡
  Test: the_floor_remembers_the_cards_that_are_only_in_the_archive
  Given 三张卡的派卡事件
  When 压缩两次,先后把卡记为已归档
  Then 没压缩过的日志基线为空;压缩后记得最大卡号、已归档的卡、键对应的卡号,后一次压缩在前一次之上累加

Scenario: 写入、读取与压缩并发,序号不丢不重(critical)
  Tags: critical
  Test: writers_readers_and_a_compactor_never_lose_or_repeat_a_number
  Given 4 个写入者各写 40 条,同时一个线程反复压缩、一个线程反复读
  When 全部结束
  Then 每次读到的序号都递增;完整历史是 1 到 160;最后一次压缩之后写入的事件全部读得到
