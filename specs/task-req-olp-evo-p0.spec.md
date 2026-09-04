spec: task
name: "进化环阶段 0:三源采集哨、进化黑板、缺陷记录目录(只读影子试点)"
tags: [olp, evolution, harness, observability]
satisfies: [REQ-OLP-EVO]
estimate: 1.5d
---

## 意图

为 OctoLoop 外环增加一个机械的、幂等的、崩溃一致的采集面:`scripts/olp-evo-harvest.sh` 从
活板、events.jsonl、MCP 审计板三个既有来源增量识别内环摩擦,落成带稳定 identity 的症状卡
追加到 `<repo>/.octos/EVOLUTION.md`;同时在本仓库 `knowledge/context/evolution/` 建立缺陷
记录目录并给它一个 frontmatter 校验器。这是 LEP-003 进化环的阶段 0,只读影子试点:不改
运行时代码、不改协议、不写 ACK。触发器贴 R1 v1 定式与 `src/olp_mcp.rs` 的真实审计行形,
不用任意子串。

## 已定决策

- 用法 `scripts/olp-evo-harvest.sh <repo-root> [--dry-run]`。环境变量:`OLP_EVO_REVIEW_BOARD`
  (缺省 `<repo>/.octos/OUTER_LOOP_REVIEW.md`)、`OLP_EVO_EVENTS`(缺省空,空即跳过)、
  `OLP_EVO_MCP_BOARD`(缺省 `~/.octos/outer/OUTER_LOOP_MCP.md`)、`OLP_EVO_BOARD`(缺省
  `<repo>/.octos/EVOLUTION.md`)、`OLP_EVO_STATE`(状态根,缺省 `~/.octos/outer/evo`)。
- 项目状态目录 = `<状态根>/<sha256(realpath(repo-root)) 前 16 位>/`,内含 `state.json`、
  `harvest.lock`。`state.json` 形状:`{"next_id":N,"seen":["<identity sha256>",...],"sources":{"review":{"path":"…","offset":N,"dev":N,"ino":N,"prefix_sha256":"…"},"events":{…},"mcp":{…}}}`,缺源无该键。
- 活板触发器:去掉前导空白后以 `ACK(blocked):` 或 `ACK(wontdo):` 开头的行(与
  `tests/olp_contract.rs` 的 v1 文法同形);所属条目编号取该行之上最近一个 `### <数字>` 标题的
  数字,无标题则为 `-`。
- events 触发器:每行经 `python3 -c` 标准库 `json.loads`;`kind` 为 `escalation` 或
  `turn_error` 触发;`kind` 为 `goal_transition` 且 `detail` 等于 ``goal transitioned to `blocked` ``
  或 ``goal transitioned to `budget_limited` `` 触发;解析失败打印 `malformed: <path>:<line>` 到
  stderr 并跳过(游标照常推进)。
- MCP 触发器:正则 `^- (\S+ \S+) MCP\(ask_outer\) (blocked|timeout): (.*)$`,kind 为 `blocked`
  记 trigger `report_blocked`,`timeout` 记 `ask_outer_timeout`;`ask`、`answer`、`refusal` 不触发;
  ask id 从 detail 的 `id=<hex>` 取,无则 `-`。
- identity(完整 64 位 sha256 十六进制,`sha256sum` 计算):活板
  `board:<realpath>#<条目编号>#<blocked|wontdo>#<行 sha256>`;events
  `events:<realpath>#<ts>#<kind>#<goal_id 或 slug 或 session 或 ->#<行 sha256>`;MCP
  `mcp:<realpath>#<时间戳>#<kind>#<ask id 或 ->#<行 sha256>`。`seen` 保存 identity 字符串的 sha256。
- 卡片格式(经 `scripts/olp-board-append.sh` 追加,正文走 stdin),五行顺序固定:
  `### EVO-NNNN（<UTC RFC3339>，harvest）` / `trigger: <ack_blocked|ack_wontdo|escalation|turn_error|goal_blocked|goal_budget_limited|report_blocked|ask_outer_timeout>` /
  `source: <review|events|mcp> <realpath>` / `identity: <identity>` /
  `envelope: line=<n> offset=<bytes> ts=<UTC RFC3339>` / `symptom: <文本>`。
- symptom:活板与 events 取触发行前 200 个字符;MCP 取 `kind=<kind> id=<ask id> ` 加
  `reason=` 之后至多 80 个字符,绝不复制 `question=`、`context=`、`tried=` 之后的内容。
- 游标:每源 `offset` 为最后一条已提交完整换行记录之后的字节偏移;尾部无换行的半条记录不触发
  不推进;`dev`/`ino` 来自 `stat -c '%d %i'`;`prefix_sha256` 为 `[max(0,offset-64), offset)`
  区间字节的 sha256。文件 identity 变化、前缀摘要不匹配或 size < offset 时打印 `reset: <path>`
  到 stderr、offset 归 0,靠 `seen` 去重。
- 提交协议(`flock -x` 于 `harvest.lock`):①活板存在性预检在任何 mkdir/flock 之前;②读
  `state.json`,并扫描进化黑板已有 `identity:` 行与 `### EVO-` 编号对账(黑板为准恢复
  `next_id` 与 `seen`);③解析三源候选;④逐卡追加;⑤写 `state.json.tmp` + `sync` +
  `mv -f` 原子替换。不得在④之前推进任何状态。
- 故障注入:仅当 `OLP_EVO_TEST=1` 时读取 `OLP_EVO_FAULT`;值 `after-append` 表示追加全部卡后、
  写状态前以退出码 70 退出。
- `--dry-run`:只读活板、来源与已有状态,把将落的卡打印到 stdout(编号按已有状态推算),不创建
  或修改状态目录、锁、进化黑板。
- 退出码:活板缺失 2;可选来源缺失打印 `skip: <path>` 退出 0;故障注入 70;其它错误 1。
- 记录目录 `knowledge/context/evolution/`:`operators.md`、`FLAW-001.md`、`FLAW-002.md` 已由主审
  提交(不改内容);本任务新增 `README.md`(声明本目录为本地约定、非 canon 新类型;列出状态机
  open→consolidated→filed→accepted→specified→patched→verified→closed,任一状态可转
  rejected,rejected 仅带新卡片可转 reopened)、`FLAW-template.md`(frontmatter 全字段)、
  `memory.md`(表头 `| FLAW | 结果 | 原因类 | 适用条件 | issue / PR |`)。
- 校验器:`tests/olp_evo_harvest.rs` 内以纯 Rust 解析每份 `FLAW-*.md` 的 frontmatter(`---` 之间
  的 `key: value` 行),断言 REQ-OLP-EVO-RECORDS 与 REQ-OLP-EVO-ISSUE 的字段与取值。
- `scripts/olp-init.sh`:在既有活板 gitignore 块之后追加同样式逻辑,用
  `git check-ignore -q .octos/EVOLUTION.md` 判定,未忽略则追加一行 `.octos/EVOLUTION.md`。
- 测试为 Rust 集成测试 `tests/olp_evo_harvest.rs`,经 `std::process::Command` 调脚本;夹具源文件放
  `fixtures/evolution/`,每个测试复制到 `std::env::temp_dir()` 下唯一子目录,`OLP_EVO_STATE` 指向
  该子目录内的状态根。
- 不新增 Cargo 依赖;脚本只依赖 bash、coreutils、flock、python3、sha256sum、stat。测试在缺 GNU flock 或 `stat -c` 的宿主(如 macOS 本地)显式打印 SKIP 并跳过,CI(ubuntu)保持全量严格。

<!-- lint-ack: decision-coverage — 用法、状态形状、故障注入、退出码等决策由多个场景共同行使,不单列场景 -->

## 边界

### Allowed Changes
- specs/task-req-olp-evo-p0.spec.md
  <!-- self-allowance: 决策文字补充(如工具依赖说明)可改;场景与断言不可改 -->
- scripts/olp-evo-harvest.sh
- scripts/olp-init.sh
- tests/olp_evo_harvest.rs
- fixtures/evolution/**
- knowledge/context/evolution/README.md
- knowledge/context/evolution/FLAW-template.md
- knowledge/context/evolution/memory.md

### Forbidden
- 不改 `src/**` 任何运行时代码。
- 不改 `AGENTS.md`、`.octos/loop.md`、`.claude/skills/**`、`docs/**`、`tests/olp_contract.rs`。
- 不改 `knowledge/context/evolution/operators.md`、`FLAW-001.md`、`FLAW-002.md`。
- 不向活板 `OUTER_LOOP_REVIEW.md` 写入任何内容,不生成任何 `ACK(` 开头的行。
- 不新增 Cargo 依赖,不新增 MCP 工具,不改 `~/.octos/outer/mcp/` 路径语义。
- 脚本缺省状态不得落 /tmp。

## 排除范围

- retro 入口与 skill 卡改动(阶段 1,operator-tier)。
- events.jsonl 实例自动发现(阶段 2)。
- octos 侧新增事件 producer(阶段 2,REQ-OLP-OBS 修订)。
- 指标脚本 `olp-evo-metrics.sh`、回放夹具制作、`docs/OCTOLOOP_FEATURES.md` 产品条目。
- 改判与 R2 打回的固定行形(规程改动,operator-tier);阶段 0 不作为触发器。

## 完成条件

场景: 全部触发器种类各落一卡(critical)
  标签: critical
  测试: olp_evo_harvest_produces_cards_for_all_trigger_kinds
  假设 夹具活板含 ACK(blocked) 与 ACK(wontdo) 各一行,events.jsonl 含 escalation、turn_error、goal_transition(blocked)、goal_transition(budget_limited) 各一行,MCP 审计板含 blocked 与 timeout 各一行
  当 以空状态根运行 olp-evo-harvest.sh
  那么 进化黑板中以 ### EVO- 开头的行数等于 8
  并且 编号依次为 EVO-0001 到 EVO-0008
  并且 每张卡按顺序含 trigger:、source:、identity:、envelope:、symptom: 五行

场景: 负例矩阵零卡(critical)
  标签: critical
  测试: olp_evo_harvest_negative_matrix_yields_zero_cards
  假设 活板正文含 落 ACK(45a done|blocked) 与 继续或 ACK(blocked)) 字样但无行首 ACK 回执,MCP 审计板只含 ask、answer、refusal 行,events.jsonl 只含 goal_transition(complete) 与一行非法 JSON
  当 运行 olp-evo-harvest.sh
  那么 进化黑板中以 ### EVO- 开头的行数等于 0
  并且 stderr 含 malformed:
  并且 退出码等于 0

场景: identity 区分条目并在重跑时去重
  测试: olp_evo_harvest_identity_distinguishes_entries_and_dedups_reruns
  假设 活板条目 ### 12 与 ### 13 下各有一行逐字相同的 ACK(blocked)
  当 运行 olp-evo-harvest.sh 两次
  那么 进化黑板中以 ### EVO- 开头的行数等于 2
  并且 两张卡的 identity 行不相等
  并且 第二次运行后进化黑板与 state.json 的 sha256 与第一次运行后相等

场景: 采集从不触碰活板(critical)
  标签: critical
  测试: olp_evo_harvest_never_writes_review_board_or_ack
  假设 全触发器夹具
  当 运行 olp-evo-harvest.sh
  那么 活板的 sha256 与运行前相等
  并且 进化黑板中以 ACK( 开头的行数等于 0

场景: docs 冻结快照被忽略
  测试: olp_evo_harvest_ignores_docs_snapshot
  假设 docs/OUTER_LOOP_REVIEW.md 含一行新的 ACK(blocked) 而活板无新行
  当 运行 olp-evo-harvest.sh
  那么 进化黑板中以 ### EVO- 开头的行数等于 0

场景: MCP 卡不复制问询正文
  测试: olp_evo_harvest_mcp_symptom_excludes_question_text
  假设 MCP 审计板含一行 blocked,其 detail 含 reason=… needs=… 与一段 question=SECRET-QUESTION 文本
  当 运行 olp-evo-harvest.sh
  那么 进化黑板不含 SECRET-QUESTION
  并且 该卡的 symptom 行以 kind=blocked 开头

场景: 半行不触发,补齐换行后恰一卡
  测试: olp_evo_harvest_partial_line_then_completed_yields_one_card
  假设 events.jsonl 尾部是一条无换行的 turn_error 记录
  当 运行 olp-evo-harvest.sh,再补上换行符后再次运行
  那么 第一次运行后进化黑板不存在或卡数等于 0
  并且 第一次运行后 state.json 中 events 的 offset 等于半行之前的字节数
  并且 第二次运行后卡数等于 1

场景: 截断或替换后重置且不重复
  测试: olp_evo_harvest_resets_on_truncate_or_replace_without_duplicates
  假设 已采集过的 events.jsonl 先被截断为最后一行,再被替换为含两条新 escalation 的更大文件
  当 每次变更后运行 olp-evo-harvest.sh
  那么 截断后 stderr 含 reset: 且卡数不变
  并且 替换后 stderr 含 reset: 且卡数恰增加 2

场景: 追加卡后提交状态前崩溃可恢复
  测试: olp_evo_harvest_recovers_after_crash_between_append_and_commit
  假设 以 OLP_EVO_TEST=1 与 OLP_EVO_FAULT=after-append 运行一次全触发器夹具
  当 去掉故障注入再次运行
  那么 第一次运行退出码等于 70
  并且 第二次运行后进化黑板中以 ### EVO- 开头的行数等于 8
  并且 state.json 中每个来源的 offset 等于其文件字节数

场景: 并发采集编号唯一
  测试: olp_evo_harvest_concurrent_runs_allocate_unique_ids
  假设 全触发器夹具与同一状态根
  当 同时启动两个 olp-evo-harvest.sh 进程并等待二者退出
  那么 进化黑板中以 ### EVO- 开头的行数等于 8
  并且 8 个 EVO 编号互不相同

场景: 活板缺失即失败且零创建
  测试: olp_evo_harvest_fails_without_review_board_before_creating_state
  假设 仓库目录下不存在 .octos/OUTER_LOOP_REVIEW.md 且状态根目录存在且可写
  当 运行 olp-evo-harvest.sh
  那么 退出码等于 2
  并且 状态根目录为空
  并且 进化黑板文件不存在

场景: dry-run 对已有状态零写入
  测试: olp_evo_harvest_dry_run_is_read_only_with_existing_state
  假设 已运行过一次采集且活板新增一行 ACK(blocked)
  当 以 --dry-run 运行 olp-evo-harvest.sh
  那么 stdout 含 ### EVO-0009
  并且 进化黑板与状态目录内每个文件的 sha256 与运行前相等

场景: 可选来源缺失时跳过
  测试: olp_evo_harvest_skips_missing_optional_sources
  假设 只有活板存在,OLP_EVO_EVENTS 为空且 OLP_EVO_MCP_BOARD 指向不存在的路径
  当 运行 olp-evo-harvest.sh
  那么 退出码等于 0
  并且 stderr 含 skip:

场景: 状态按项目隔离
  测试: olp_evo_harvest_state_is_per_project
  假设 两个仓库目录各有一块含一行 ACK(blocked) 的活板
  当 对两个仓库各运行一次 olp-evo-harvest.sh
  那么 两块进化黑板各含一张 EVO-0001
  并且 状态根目录下存在两个不同的项目子目录

场景: 记录目录与记录校验
  测试: olp_evo_records_dir_frontmatter_is_valid
  假设 仓库检出
  当 解析 knowledge/context/evolution/ 下全部 FLAW-*.md 的 frontmatter
  那么 README.md、FLAW-template.md、memory.md、operators.md 四个文件存在
  并且 每份记录含 kind: context、唯一 id、repo、layers、status、severity、recurrence、fingerprint
  并且 status 与 severity 取值合法,recurrence 为非负整数,fingerprint 非空
  并且 FLAW-001.md 与 FLAW-002.md 的 issue 字段分别含 issues/2236 与 issues/2237

场景: olp-init 为只忽略活板的项目追加 EVOLUTION.md 忽略
  测试: olp_evo_init_appends_evolution_gitignore_once
  假设 一个临时 git 仓库,其 .gitignore 只含 .octos/OUTER_LOOP_REVIEW.md
  当 运行 scripts/olp-init.sh 两次
  那么 .gitignore 中 .octos/EVOLUTION.md 恰出现一次

场景: olp-init 对整目录已忽略的项目不追加
  测试: olp_evo_init_skips_when_octos_dir_ignored
  假设 一个临时 git 仓库,其 .gitignore 含 .octos
  当 运行 scripts/olp-init.sh
  那么 .gitignore 不含 .octos/EVOLUTION.md
