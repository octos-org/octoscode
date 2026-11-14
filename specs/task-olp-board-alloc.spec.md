spec: task
name: "写板时在锁内分配编号、按自然键去重;OLP 脚本的工具预检共享化"
id: task-olp-board-alloc
status: implemented
tags: [olp, board, flock, macos, preflight]
estimate: 1d
---

## Intent

`scripts/olp-board-append.sh` 只做互斥追加,不分配编号:调用方先 grep 最大号再自己写
`### <N>.`,取号与写入之间没有锁,多个写入方并发时必然撞号。集群方案里十个单元都向同一块
黑板写入,任务号就是黑板条目号,撞号会让 ACK、采认与分支名指向错误的条目。同时,写板脚本
直接调用 `flock` 命令,macOS 上它来自 Homebrew,而非交互 ssh 的 PATH 不含 Homebrew 目录,
远程调用会在没有任何提示的情况下失败。

本任务让编号在锁内分配、让可能重发的写入按自然键幂等,提供同一机制的通用编号分配助手
(API、应用、墙编号),并把工具预检抽成共享函数接入 retro 与 skeleton 脚本。

## Decisions

- 编号分配:正文首行以 `### {N}` 起头时,助手在锁内取板上所有 `### <数字>` 标题的**最大值**
  +1(不是最后一个——板上条目可以乱序),替换正文里所有 `{N}`,并把编号打印到 stdout。板
  不存在或没有编号标题时从 1 开始。只有首行以 `### {N}` 起头才触发分配:编号靠板上的标题
  延续,不落成标题的分配不会被下一次看到。首行不是 `### {N}` 时,正文别处的 `{N}` 字样是
  普通文本,不替换也不报错——采集哨转抄的原文里可能碰巧含有它,写板不该因此失败。
- 幂等重发:`--key <自然键>`。追加时在条目末尾写一行标记 `<!-- olp-key: <键> n=<编号> -->`
  (未分配编号的条目为 `<!-- olp-key: <键> -->`)。同一个键已在板上时不追加,stdout 返回原
  编号(原条目无编号时无输出),退出码 0。标记写在板上而不是旁路文件里:板是唯一的事实源,
  镜像与恢复后去重仍然成立。键限定为 `[A-Za-z0-9._:/@+=-]`、最长 200 字节,逐字比较
  (一个键是另一个键的前缀时互不命中)。
- 旧写法兼容:首行不是 `### {N}` 且不带 `--key` 时原样追加、无输出;空正文仍是空操作。
- 换行收口:条目末尾缺换行时补上;板的最后一行缺换行时先补上再追加,新条目总是从行首
  开始。
- 条目间隔:分配了编号的条目(首行 `### {N}`)追加时,板的最后一行不是空行就先空一行,
  标题不贴着上一条的末行(例如模板里的一行 `ACK:`)。不带 `{N}` 的旧写法不加空行,逐字追加。
- 写板脚本保持**单文件自包含**:`olp-init.sh` 把它单独复制到 `~/.octos/outer/`,所以它不
  source 共享预检,而是内联一段只找 `flock` 的发现逻辑。
- 通用编号分配:新脚本 `scripts/olp-id-alloc.sh <登记文件.tsv> <前缀> <位数> <自然键>`。在
  `<登记文件>.lock` 的 flock 下,同一 (前缀, 自然键) 已登记则返回原编号,否则取该前缀已登记
  的最大序号 +1、追加一行 `<编号>\t<前缀>\t<自然键>\t<UTC 时间>`。各前缀独立计数。自然键
  逐字节比较(经环境变量传给 awk,不解释反斜杠),不得含控制字符。
- 共享预检:新文件 `scripts/olp-preflight.sh`(被 source),`olp_preflight <脚本名> <能力>...`。
  能力:`flock`、`realpath`、`realpath-m`、`stat-c`、`sha256sum`、`python3`。全部可用时不改
  环境。macOS 上有缺失时补 PATH 后重新探测:coreutils 类把 Homebrew coreutils 的 gnubin 放
  到 PATH 最前(与 `scripts/verify.sh` 一致);`flock`/`python3` 把所在的 Homebrew bin 目录
  追加到 PATH **末尾**,不改变其他命令的解析顺序。其他平台不做发现。仍缺失则向 stderr 列出
  缺失项、返回 2。`OLP_PREFLIGHT_DISCOVER=0` 关闭发现(只会更严格)。
- 接入:`olp-evo-retro.sh`(`realpath sha256sum python3`,非 dry-run 另加 `flock`)、
  `olp-evo-spec-skeleton.sh`(`python3`,带 `--out` 时另加 `realpath-m`)、`olp-id-alloc.sh`
  (`flock`)。预检都在任何写入(建目录、开锁文件)之前;失败退出码 2。
- 不动 `scripts/olp-evo-harvest.sh`:它的预检在另一个独立分支(PR #703)里修改,两个分支
  互不叠加。它调用写板脚本时走旧写法,行为不变。
- `docs/OLP_OUTER_BOOT.md` §1 里"先 grep 最大号再 +1"的说明本任务不改:该文件 §0–§6 被
  `tests/olp_evo_retro.rs` 按哈希锁定,另一个未合并的分支(PR #706)已改过同一个哈希,
  两边同时改会在合并后得到一个两边都没算过的哈希。待 #706 合并后单独更新。

## Boundaries

### Allowed Changes
- scripts/olp-board-append.sh
- scripts/olp-id-alloc.sh
- scripts/olp-preflight.sh
- scripts/olp-evo-retro.sh(仅预检接入与退出码注释)
- scripts/olp-evo-spec-skeleton.sh(仅预检接入)
- tests/olp_board_alloc.rs
- docs/OCTOLOOP_AGENTS.md
- docs/OCTOLOOP_AGENTS.zh-CN.md
- specs/task-olp-board-alloc.spec.md

### Forbidden
- 不改 scripts/olp-evo-harvest.sh。
- 不改 docs/OLP_OUTER_BOOT.md 与 tests/olp_evo_retro.rs 的 golden 哈希。
- 不改黑板条目的行首定式(`### <N>.`、`ACK(`、`> 外环(…)·`)。
- 不改 `~/.octos/outer/board_append.sh`(MCP 落板用的本机脚本)的约定。
- 不引入新的依赖,不自动安装任何工具。

## Out of Scope

- 经 ssh 的远程写板、离线队列与回放(下一个工作包,依赖本任务的 `--key`)。
- 多块黑板共用一个计数器。
- 登记文件的异地镜像与恢复时跳号。
- 让 harvest 改用共享预检(待 PR #703 合并后)。

## Completion Criteria

### Rule: alloc — 编号在锁内分配
Scenario: 锁内分配下一个编号(critical)
  Tags: critical
  Test: board_append_allocates_next_number_under_lock
  Given 一块标题乱序(19 在 18 之前)且末行缺换行的板
  When 以首行 `### {N}. …` 的正文调用写板脚本
  Then stdout 为 20,板上新条目从行首开始且正文里的 `{N}` 全部替换为 20;不存在的板从 1 开始

Scenario: 100 个并发写入方无重号、无交错(critical)
  Tags: critical
  Test: board_append_concurrent_writers_get_unique_numbers_and_intact_entries
  Given 一块空板
  When 100 个进程同时各写入一个三行条目
  Then 返回的编号恰为 1 到 100 各一次,板上每个条目的三行连续且正文里的编号与标题一致

Scenario: 标题不贴着上一条的末行
  Test: board_append_separates_a_heading_from_a_non_blank_last_line
  Given 一块以一行 `ACK:` 结尾的板(olp-init.sh 铺的模板),以及一块以空行结尾的板
  When 追加一个 `### {N}. …` 条目,再追加一行不带标题的判词
  Then 新标题与上一条的末行之间恰好隔一个空行;判词紧接在上一行之后;已以空行结尾的板不再多空一行

### Rule: dedup — 自然键幂等
Scenario: 同一自然键重发返回原编号(critical)
  Tags: critical
  Test: board_append_same_key_returns_original_number_without_duplicate
  Given 一个带 `--key` 写入并得到编号 5 的条目,其后又有别的条目写入
  When 以同一个键再次写入;以及 20 个进程同时以另一个新键写入
  Then 重发返回 5 且板上不新增条目;并发的 20 次写入只落一个条目、返回同一个编号;前缀键不命中

Scenario: 不带编号的条目按键去重
  Test: board_append_key_dedups_entries_without_a_number
  Given 一条采认行(无 `{N}`)带 `--key` 写入
  When 以同一个键再写一次
  Then 板上只有一条采认行与一行键标记,两次 stdout 均为空

### Rule: compat — 旧写法与非法输入
Scenario: 旧写法原样追加
  Test: board_append_legacy_body_is_appended_verbatim
  Given 一块已有条目的板
  When 以自带编号、不带 `--key` 的正文写入;以首行不是 `### {N}` 但正文含 `{N}` 字样的正文写入;再以空正文写入
  Then 正文都原样追加(`{N}` 字样不被替换)且 stdout 为空;空正文不改变板

Scenario: 非法输入在触碰黑板前被拒绝
  Test: board_append_rejects_invalid_input_before_touching_board
  Given 非法的键、空键、带键的空正文
  When 分别调用写板脚本
  Then 退出码均为 2,板文件与锁文件都没有被创建

### Rule: preflight — 工具预检
Scenario: 找不到 flock 时在任何写入前失败(critical)
  Tags: critical
  Test: board_append_fails_before_write_when_flock_missing
  Given 一个不含 flock 的 PATH,且关闭了发现
  When 调用写板脚本
  Then 退出码 2,stderr 提到 flock,板文件与锁文件都没有被创建

Scenario: 最小 PATH 下可写板
  Test: board_append_works_with_minimal_path
  Given PATH 仅为 `/usr/bin:/bin`(非交互 ssh 的情形)
  When 调用写板脚本
  Then 写入成功并返回编号 1

Scenario: retro 缺工具时在任何写入前失败
  Test: evo_retro_fails_before_write_when_tools_missing
  Given 一个只含 dirname 与 uname 的 PATH,且关闭了发现
  When 运行 `olp-evo-retro.sh`
  Then 退出码 2,stderr 列出 sha256sum、python3、flock,状态目录没有被创建

Scenario: skeleton 缺工具时在任何写入前失败
  Test: evo_skeleton_fails_before_write_when_tools_missing
  Given 一个只含 dirname 与 uname 的 PATH,且关闭了发现
  When 以 `--out` 运行 `olp-evo-spec-skeleton.sh`
  Then 退出码 2,stderr 列出 python3 与 realpath-m,输出目录没有被创建

Scenario: 最小 PATH 下 skeleton 自行找到 GNU 工具
  Test: evo_skeleton_out_works_with_minimal_path
  Given PATH 仅为 `/usr/bin:/bin`
  When 以 `--out` 运行 `olp-evo-spec-skeleton.sh`
  Then 成功写出骨架文件

### Rule: id-alloc — 通用编号分配
Scenario: 同键同号,各前缀独立计数(critical)
  Tags: critical
  Test: id_alloc_same_key_returns_same_id_and_series_are_independent
  Given 一个空的登记文件
  When 以若干 (前缀, 自然键) 请求编号,其中有重复的键和含反斜杠的键
  Then 同一个键总得到同一个编号,不同前缀各自从 1 计数,登记文件每个 (前缀, 键) 只有一行

Scenario: 并发请求无重号且按键去重
  Test: id_alloc_concurrent_requests_are_unique_and_deduplicated
  Given 一个空的登记文件
  When 40 个键各被请求两次,80 个进程同时发起
  Then 每个键的两次请求得到同一个编号,编号恰为 W00001 到 W00040,登记文件 40 行

Scenario: 非法请求在触碰登记文件前被拒绝
  Test: id_alloc_rejects_invalid_input_before_touching_registry
  Given 非法前缀、非法位数、空键、含制表符或换行的键
  When 分别调用分配脚本
  Then 退出码均为 2,登记文件与锁文件都没有被创建
