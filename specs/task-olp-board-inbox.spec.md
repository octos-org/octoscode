spec: task
name: "黑板收件箱:从黑板算出每张卡等谁(收件箱 / 进行中 / 搁置 / 已了结)"
id: task-olp-board-inbox
status: implemented
tags: [olp, board, inbox]
estimate: 1d
---

## Intent

黑板是按时间追加的流水账。多个单元并行之后,外环要从头扫一遍才知道哪些事在等它决定。借鉴
T3 Code 的 Inbox / Working / Settled 模型:只把"等我决定"的事放进收件箱,正在跑的和已经
了结的各归各位。内环的规则"执行编号最小的未 ACK 条目"已经是内环的收件箱;本任务补上外环
的收件箱和"了结"的明确定义。

做这件事时发现协议有一个缺口:改判与 R2 记档有行首定式,但"采认"只要求署名、没有格式,
脚本无法判断一张卡是否已经通过。本任务补上采认、打回、搁置三个定式。

## Decisions

- 工具 `scripts/olp-inbox.py <board> [--json] [--unit U] [--all] [--today YYYY-MM-DD]`,纯
  Python 标准库(兼容 Python 3.9),只读黑板,不写任何文件。
- **状态不存进黑板**,每次从现有的行算出来;黑板保持只追加。
- 新增三个行首定式(逐字照抄;采集哨不触发):
  - `> 外环(<署名>)·采认(#N):<证据>`
  - `> 外环(<署名>)·打回(#N):<原因>`——整改内容仍按 R1/R5 另立新条目
  - `> 外环(<署名>)·搁置(#N) 至 YYYY-MM-DD:<等什么>`
- 识别规则:只认行首定式,代码围栏内不算;行首的 `> `(可重复)、`**` 与空白先剥掉(与
  采集哨一致)。判词类的行(采认、打回、搁置、改判作废、单元推送登记)按 `#N` 归属,写在
  板上任何位置都算——追加在板尾的判词物理上落在最后一个条目下面,不能按所在条目归属。
  `ACK(` 行、`unit:` 行与归档标记按所在条目归属。
- 内环的署名写法也算 ACK:`> 内环(<署名>)·ACK(done|wontdo|blocked)…` 与
  `> 内环(<署名>)·#N ACK(…)…`(本仓库的活板上实际出现的两种写法)。带 `#N`(写在 ACK 前,或
  紧跟在冒号后)时归到 #N,否则归到所在条目。协议规定的写法仍是行首 `ACK(…):`,这里只是
  读的时候从宽。
- 分区判定:对同一条目取**行号最大**的那件事——
  采认或打回 → 已了结;`ACK(done)` → 收件箱(待复验);`ACK(wontdo)` → 收件箱(待回应);
  `ACK(blocked)` → 收件箱(待解阻塞);单元推送登记 → 收件箱(待审核);都没有 → 进行中。
  改判作废与归档桩不看先后,直接是已了结。
- 搁置:只有落在该条目最后一件事之后的搁置行才生效;今天早于搁置日期 → 搁置分区,否则回到
  原本的分区并在原因里注明已到期。之后有新的 ACK 或推送则搁置自动失效。日期无法解析的搁置
  行被忽略并报告。未 ACK 的条目也可以搁置,到期后进收件箱。
- 输出:文本按分区列出条目号、单元、标题与原因;默认不列已了结(只给数量),`--all` 全列。
  `--json` 给出 `today`、`counts`、`items`、`warnings`。`--unit` 只看派给该单元的条目。
  `--today` 用于测试与回放。
- 顺带报告结构问题(`warnings`,文本模式走 stderr):同一编号出现两次(以第一次为准)、搁置
  日期无法解析、判词指向板上不存在的条目。
- 范围:本任务只读黑板。"已采认、等人合并"需要读 GitHub,"人的收件箱"留给后续工作包。
- 退出码:0 成功;2 用法错误或板不存在。

## Boundaries

### Allowed Changes
- scripts/olp-inbox.py
- tests/olp_inbox.rs
- docs/OCTOLOOP_AGENTS.md
- docs/OCTOLOOP_AGENTS.zh-CN.md
- docs/OLP_OUTER_BOOT.md(仅 §7"进化环批注定式"之后新增一段;§0–§6 不动)
- specs/task-olp-board-inbox.spec.md

### Forbidden
- 不改既有定式(ACK、改判、R2 记档)的写法与采集哨的触发规则。
- 不往黑板写任何内容。
- 不改 docs/OLP_OUTER_BOOT.md 的 §0–§6。

## Out of Scope

- 读本地 git 仓库与 GitHub 的状态(等人合并、已合并)。
- 归档工具改用"已采认"作为闭环判定。
- 内环提问(第五信道)进入收件箱。
- 在 TUI 或全景页面里展示。

## Completion Criteria

### Rule: partitions — 分区判定
Scenario: 每个条目按最后发生的事归入分区(critical)
  Tags: critical
  Test: inbox_sorts_entries_by_their_latest_event
  Given 一块板,十个条目分别是:已采认、ACK(done) 未采认、未 ACK、ACK(blocked)、ACK(wontdo)、被打回、单元已推送、被改判作废、作废它的新条目、已归档的桩;判词追加在板尾
  When 运行收件箱
  Then 分区依次为 已了结、收件箱、进行中、收件箱、收件箱、已了结、收件箱、已了结、进行中、已了结,原因与之相符;默认输出不列已了结但计数在;`--unit m01` 只剩派给 m01 的两条

Scenario: 判词之后的新动作让条目回到收件箱(critical)
  Tags: critical
  Test: inbox_reopens_when_something_happens_after_a_verdict
  Given #1 推送后被打回、返工后再次推送;#2 采认后没有新动作;#3 条目内先 ACK(blocked) 后 ACK(done)
  When 运行收件箱
  Then #1 在收件箱(待审核),#2 已了结,#3 在收件箱(待复验)

Scenario: 搁置到期前不进收件箱,到期或有新动作后回来
  Test: inbox_snooze_hides_until_the_date_or_new_activity
  Given #1 阻塞并搁置到 12 月 1 日;#2 的搁置行在它最后一条 ACK 之前;#3 未 ACK 即搁置;#4 的搁置日期写坏
  When 在 11 月 12 日、搁置日前一天和搁置日当天分别运行
  Then 搁置日之前 #1、#3 在搁置分区;#2 在收件箱;#4 在收件箱并有警告;搁置日当天 #1、#3 回到收件箱且原因注明到期

### Rule: parsing — 识别规则
Scenario: 只认行首定式,代码围栏与正文提及不算
  Test: inbox_only_honours_line_start_forms_outside_code_fences
  Given #1 的代码围栏里有采认行、ACK 行和一个示例标题,正文中间也提到了采认;#2 的采认行多一层引用并加粗
  When 运行收件箱
  Then #1 仍是进行中,#2 已了结,围栏里的示例标题不成为条目

Scenario: 内环的署名写法也算 ACK
  Test: inbox_accepts_signed_inner_ack_lines
  Given 署名 ACK 带条目号且落在别的条目下面、署名 ACK 在自己的条目里、署名 ACK 不带条目号、以及一行署名但不是 ACK 的 checkpoint
  When 运行收件箱
  Then 带条目号的归到所指条目而不是所在条目,其余归到所在条目;checkpoint 行不算 ACK

Scenario: 报告黑板的结构问题
  Test: inbox_reports_board_problems_as_warnings
  Given 编号 #1 出现两次,且有一条采认指向不存在的 #7
  When 运行收件箱(JSON 与文本两种输出)
  Then 警告里有撞号与悬空判词;第一次出现的 #1 生效,重复条目的 ACK 不串到它身上;文本输出按分区给出数量,警告走 stderr

Scenario: 用法错误
  Test: inbox_rejects_bad_usage
  Given 非法的 `--today`、未知参数、不存在的板
  When 分别运行
  Then 退出码均为 2,板没有被改动
