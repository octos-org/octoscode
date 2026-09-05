spec: task
name: "进化环阶段 3:采集挂外环节拍、FLAW 直出规格骨架、散文沉淀索引、停摆与伪 verified 诊断"
tags: [olp, evolution, harness, automation]
satisfies: [REQ-OLP-EVO-P3]
estimate: 2d
---

## 意图

把进化环剩下的四段手工接成机械:①活板监视器命中即采集并常驻,让战役中的摩擦在上下文还热时进黑板;
②FLAW 记录直出任务契约骨架,主审只补测试选择器与场景细节;③FLAW 索引把 `docs/OUTER_LOOP_PROTOCOL.md`
的散文沉淀标成"已记录",原文不删;④指标脚本补"派单无 ACK"与"伪 verified"两项跨源诊断(仍是诊断,
不是判据)。另把 `iteration_cap`/`patch_failed` 登记为 kind 候选(发射点属 octos 侧)。不改运行时代码、
协议与 MCP 工具面;骨架不自动入库 `specs/`。契约 v4 已并入 grok/codex 对抗复审与 codex gpt-6 PR 复审(#619)。

## 已定决策

- `scripts/olp-watch-board.sh` 新增 `--harvest <repo-root>`(放进既有 `case`,与 `--interval`/`--skip-signature`
  并存):命中 token 时先运行采集脚本(定位顺序 `<repo-root>/scripts/olp-evo-harvest.sh` → `$(dirname "$0")/olp-evo-harvest.sh`,都缺则 stderr `harvest: script not found` 并按失败处理;安装版 `~/.octos/outer/watch-board.sh` 因此可直接用 `--harvest`),再打印 `BOARD-SIGNAL: <token>`
  与命中行(既有格式),然后 `base=$cur` 并 `continue`;不带 `--harvest` 时保持既有一击 `exit 0`。采集非零退出只在
  stderr 打 `harvest: failed (exit N)`,仍打印命中并 `base=$cur`(不重试、不重复打印);非命中的新增行不触发采集;
  监视器不 `export`/不改 `OLP_EVO_STATE`,与手跑共用 `harvest.lock`。
- `scripts/olp-evo-lib.py` **新增** `parse_flaw(text)`:返回 `{id, req, status, layers, issue, pr, sections{段名: 正文},
  paths[]}`;frontmatter 用简单 `key: value` 解析(不引入 yaml);`paths` = 责任步 + 锚点两段中含 `/` 的反引号内容,
  去掉 ` L123`、` L123–L140`、`:123`、`:123-140` 行号后缀并去重(`src/worker.rs:123` → `src/worker.rs`)。
- 所有 python 入口(skeleton、index、metrics、retro)以 `python3 -B` 运行或在 import 前设 `sys.dont_write_bytecode`;
  `scripts/` 下不得出现 `__pycache__`。
- `scripts/olp-evo-spec-skeleton.sh <FLAW-NNN.md> [--out <file>]`(bash + python3,import lib):输出中文段头契约
  (`## 意图`/`## 已定决策`/`## 边界`/`## 排除范围`/`## 完成条件`/`## 问题`);段别名:决策来源段 = `修复` 否则 `结案`,
  Forbidden 来源段 = `预防` 否则 `保护门`;来源段的 `- ` 列表项各成一项,无列表则整段一项;每项一条场景
  `场景: <项前 40 字>` / `测试: pending_<slug>`(slug = 项文本中 `[A-Za-z0-9_]+` 片段以 `_` 连接、小写,为空则
  `pending_item_<n>`,slug 按片段边界截到 48 字符,禁止 `pending_pending_`)/ 假设·当·那么三行占位;缺段 `<!-- TODO: <段名> -->`;`satisfies: [<req>]` 或 `[]` + `## 问题`
  列 `- 未绑定需求`。缺省 stdout;`--out` 保护**不依赖 cwd**:目标 realpath 所在目录 `git -C … rev-parse --show-toplevel` 求所属仓库(无仓库=仓外),相对路径以 `specs/` 开头且不以 `specs/drafts/` 开头则退出 2 并 stderr
  `refusing to write outside specs/drafts/`;`specs/drafts/` 不存在则创建。
- `scripts/olp-evo-index.sh <repo-root>`(bash + python3,import lib 的 `parse_flaw`):扫描 `knowledge/context/evolution/FLAW-*.md`
  与 `docs/OUTER_LOOP_PROTOCOL.md` 的 `> 已记录:FLAW-NNN` 行(取其上方最近的 `#` 标题为"取代散文"),生成
  `INDEX.md`:首行 `# FLAW 索引(生成,勿手改)`,表头 `| FLAW | 状态 | 层 | issue / PR | 取代散文 |`,按 id 排序,
  末行 `retired_prose: N`(N = 被引用的不同 FLAW 数);写前比对,内容相同不写。
- `docs/OUTER_LOOP_PROTOCOL.md`:只在"迭代预算是任务切分的硬约束"条目下加一行
  `> 已登记:kind 候选 iteration_cap(见 knowledge/context/evolution/README.md)`;不为 FLAW-001/002 加引用。
- `scripts/olp-evo-metrics.sh` 新增 `--stall <review-board>`、`--stall-threshold <minutes>`(缺省 0)、`--now <ISO8601>`
  (缺省当前时间):片号原子 `S = [0-9]+[a-z]?(?:-[0-9a-z]+)*`;派单行先剥 `^(> )?`、`\*\*`、`外环(...)·`/`外环·` 署名,
  再匹配 `(?:^|\s)(?:派单\s+(?P<a>S)|(?P<b>S)\s+派单)(?![0-9a-z])`(两种词序:`派单 43c-2` 与 `27c 派单`);
  `不派单`、`立案并派单`、`立案+派单` 因前一字符非空白/非行首或后无片号而不匹配;两分支各自独立加边界:正序 `派单\s+(S)(?![0-9a-z])`、反序 `(?<![0-9a-z])(S)\s+派单`(`派单 99xyz`、`abc27c 派单` 都不匹配);ACK 行匹配
  `^(?:> )?ACK\((?P<s>S)\s+(?:done|blocked|wontdo)\b`,同片号取最长;派单时间 = 行内 `\((\d{4}-\d\d-\d\d) (\d\d:\d\d)`
  或 `\[(\d{4}-\d\d-\d\d)T(\d\d:\d\d)`,缺则所属 `### N.` 标题的 `\((\d{4}-\d\d-\d\d)` 按 00:00(每遇新 `### N.` 标题先清空条目日期再提取,无日期标题下的派单为未知),仍缺为未知;
  无 ACK 且(未知或 `now - t` ≥ 阈值)输出 `stall: <片号> <整数分钟|open>`,汇总 `stalls: N`;`fake_verified: N` =
  窗口内 `r2_record` 卡数;`--json` 增 `stalls`(对象列表 `{slice, minutes|null}`)与 `fake_verified`;既有行不改,
  `note:` 行不变,退出码恒 0,零写入,输出不含 `regress`。
- README 增"kind 候选"一段(`iteration_cap`、`patch_failed`,出处 2026-09-05 octos 活板 #48"48b 中断记录",
  发射点属 octos 侧另立契约)与"索引"一段(INDEX.md 由脚本生成)。
- 夹具:`fixtures/evolution/skeleton/FLAW-sample.md`(模板五段 + `req: REQ-OLP-EVO`)、
  `fixtures/evolution/index/`(两条 FLAW + 带引用的 PROTOCOL 片段)、`fixtures/evolution/stall/review-board.md`
  (含 `派单 43c-2`、`派单 43-r1`+`ACK(43-r1 done)`、`27c 派单`+`ACK(27c done)`、`不派单`、标题日期)。
- 测试隔离:每条测试用含测试名的独立临时目录,只清理自己的目录;全套以 `cargo test --all-targets` 默认并行通过。
  skeleton 解析断言分别检查 `agent-spec parse` 退出码为 0 与输出中的场景计数行,不得用 `contains("3")` 兜底。
- CI:`.github/workflows/ci.yml` 的 test job 安装固定版本 `agent-spec 1.4.0`(`cargo install agent-spec --version 1.4.0
  --locked`,缓存 `~/.cargo/bin`),缺命令时测试失败而非跳过。
- skill:`## 自主性纪律`/`## 模式 init`/`## 模式 inner`/description 为阶段 1 golden 保护,逐字不动;`--harvest` 用法只写在 `## 模式 outer`。
- 测试:`tests/olp_watch_board.rs`(新增六场景,既有四断言不动)、`tests/olp_evo_skeleton.rs`、`tests/olp_evo_index.rs`、
  `tests/olp_evo_metrics.rs`(新增四场景);不新增 Cargo 依赖;脚本只依赖 bash、coreutils、python3。

<!-- lint-ack: decision-coverage — 输出格式细节由各脚本场景共同行使 -->
<!-- lint-ack: observable-decision-coverage — 每个脚本决策由其下多条场景共同行使 -->

## 边界

### Allowed Changes
- scripts/olp-watch-board.sh
- scripts/olp-evo-lib.py
- scripts/olp-evo-spec-skeleton.sh
- scripts/olp-evo-index.sh
- scripts/olp-evo-metrics.sh
- tests/olp_watch_board.rs
- tests/olp_evo_skeleton.rs
- tests/olp_evo_index.rs
- tests/olp_evo_metrics.rs
- fixtures/evolution/skeleton/**
- fixtures/evolution/index/**
- fixtures/evolution/stall/**
- .github/workflows/ci.yml
- .claude/skills/octoloop/SKILL.md
- knowledge/context/evolution/README.md
- knowledge/context/evolution/INDEX.md
- docs/OUTER_LOOP_PROTOCOL.md

### Forbidden
- 不改 `src/**`、`.octos/loop.md`、`AGENTS.md`;skill 只改 `## 模式 outer` 节,受保护节逐字不动(阶段 1 golden)。
- 不改阶段 0–2 已钉的触发器判定、卡片格式、简报格式、既有指标行与既有测试断言(只允许新增)。
- 不改 `fixtures/evolution/replay/**`;不改 `olp-watch-board.sh` 不带 `--harvest` 时的一击退出语义。
- 骨架脚本不得写入 `specs/`(`specs/drafts/` 除外);索引脚本只写 `INDEX.md`;指标脚本零写入。
- 不删除 `docs/OUTER_LOOP_PROTOCOL.md` 任何原文;不为 FLAW-001/002 伪造 `> 已记录` 引用。
- stall/fake_verified 不得进入退出码、不得输出 `regress`/`失败` 字样。
- 不新增 Cargo 依赖,不新增 MCP 工具。

## 排除范围

- `iteration_cap`/`patch_failed` 的 octos 侧发射点(另立 REQ-OLP-OBS 修订与契约)。
- 空 ACK(有 ACK 无 commit)的识别(需活板与 git 对账)。
- LEP-004 canon `flaw` 类型;`/octoloop` skill 步骤文案(主审另片)。
- 两个战役的运行验收(非机械场景)。

## 完成条件

场景: 节拍采集在命中时落卡、推进基线且常驻(critical)
  标签: critical
  测试: olp_watch_board_harvest_on_hit_writes_card_and_keeps_watching
  假设 监视器以 --harvest <临时仓库> --interval 1 与 token ACK 启动,活板基线 3 行,OLP_EVO_STATE 指向临时状态根
  当 活板追加一行 `ACK(blocked): waiting for outer decision` 并等待 3 秒
  那么 临时仓库进化黑板中以 ### EVO- 开头的行数等于 1
  并且 stdout 恰含一次 BOARD-SIGNAL 且监视器进程仍在运行

场景: 节拍与手跑并发只落一张卡
  测试: olp_watch_board_harvest_concurrent_with_manual_is_deduped
  假设 监视器以 --harvest 运行且 OLP_EVO_STATE 与手跑相同
  当 活板追加一行命中行后立即手跑 olp-evo-harvest.sh 两次
  那么 进化黑板中该行的 identity 只出现一次

场景: 采集失败不中断监视
  测试: olp_watch_board_harvest_failure_keeps_watching
  假设 监视器以 --harvest /nonexistent 启动
  当 活板追加一行命中 token 的文本
  那么 stderr 含 harvest: failed
  并且 stdout 含 BOARD-SIGNAL 且监视器进程仍在运行

场景: 非命中新增行不触发采集
  测试: olp_watch_board_harvest_ignores_non_hit_lines
  假设 监视器以 --harvest 启动
  当 活板追加两行不含 token 的文本并等待两个间隔
  那么 进化黑板不存在或以 ### EVO- 开头的行数等于 0
  并且 stdout 不含 BOARD-SIGNAL

场景: 采集失败后下一批命中仍可处理
  测试: olp_watch_board_harvest_recovers_after_failure
  假设 监视器以 --harvest <临时仓库> 启动且首批命中时 olp-evo-harvest.sh 因 OLP_EVO_STATE 指向只读目录而失败
  当 修复权限后活板再追加一行命中行
  那么 stderr 恰含一次 harvest: failed 且进化黑板最终恰含一张卡

场景: 安装版监视器也能采集
  测试: olp_watch_board_harvest_works_from_installed_copy
  假设 仅把 olp-watch-board.sh 复制到临时目录作为安装版,--harvest 指向包含 scripts/olp-evo-harvest.sh 的临时仓库
  当 活板追加一行命中行
  那么 stderr 不含 harvest: failed 且进化黑板恰一张卡

场景: 不带 --harvest 仍一击退出
  测试: olp_watch_board_without_harvest_exits_on_hit
  假设 监视器不带 --harvest 启动
  当 活板追加一行命中行
  那么 监视器退出码等于 0 且 stdout 含 BOARD-SIGNAL

场景: FLAW 直出骨架可解析(critical)
  标签: critical
  测试: olp_evo_skeleton_from_template_flaw_parses_and_maps_sections
  假设 夹具 fixtures/evolution/skeleton/FLAW-sample.md 含症状/责任步/根因/修复(3 项)/预防五段与 req: REQ-OLP-EVO
  当 运行 olp-evo-spec-skeleton.sh 该文件
  那么 stdout 经 agent-spec parse 报告场景数等于 3
  并且 stdout 含 satisfies: [REQ-OLP-EVO]、三行 测试: pending_ 与 预防项在 ### Forbidden 下

场景: 真实 FLAW-001 走别名段且缺段占位
  测试: olp_evo_skeleton_real_flaw_uses_aliases_and_todo
  假设 仓库内 knowledge/context/evolution/FLAW-001.md
  当 运行 olp-evo-spec-skeleton.sh
  那么 退出码等于 0 且 stdout 经 agent-spec parse 可解析
  并且 stdout 含 satisfies: []、未绑定需求、至少一行 测试: pending_ 与 crates/octos-cli/src/peers/mod.rs

场景: 骨架字段精确映射
  测试: olp_evo_skeleton_maps_root_cause_paths_and_item_slug_exactly
  假设 一份 FLAW 含症状与根因段、锚点含 `src/worker.rs:123`、修复段一项为纯中文
  当 运行 olp-evo-spec-skeleton.sh
  那么 stdout 的 ## 意图 含根因段文本、### Allowed Changes 含一行 `- src/worker.rs` 且不含 `:123`
  并且 含一行 `测试: pending_item_1` 且不含 pending_pending_

场景: 骨架拒绝写入 specs 根
  测试: olp_evo_skeleton_refuses_out_into_specs_root
  假设 --out 指向 <仓库>/specs/task-x.spec.md
  当 分别在仓库内与仓库外(cwd 为临时目录、用脚本绝对路径)运行 olp-evo-spec-skeleton.sh
  那么 两次退出码均等于 2 且 stderr 含 refusing to write outside specs/drafts/
  并且 该文件不存在

场景: 索引生成并识别退役散文(critical)
  标签: critical
  测试: olp_evo_index_lists_flaws_and_retired_prose
  假设 临时仓库含 fixtures/evolution/index/ 的两条 FLAW 与 PROTOCOL 片段(一行 `> 已记录:FLAW-001`)
  当 运行 olp-evo-index.sh 该仓库
  那么 INDEX.md 含两行以 | FLAW-00 开头、FLAW-001 行的取代散文列非 —、FLAW-002 行为 —、末行 retired_prose: 1

场景: 索引幂等不改 mtime
  测试: olp_evo_index_is_idempotent
  假设 同上仓库已生成 INDEX.md
  当 记录 mtime 后再次运行 olp-evo-index.sh
  那么 INDEX.md 内容与 mtime 均不变

场景: 停摆诊断按活板定式
  测试: olp_evo_metrics_stall_matches_board_grammar
  假设 fixtures/evolution/stall/review-board.md 含标题 `### 43. …(2026-09-05,` 下的 `> 外环·**派单 43c-2**` 无 ACK、`派单 43-r1` 与 `> ACK(43-r1 done):`、一行 `### 37. 挂账:…不派单`
  当 以 --stall <板> --stall-threshold 30 --now 2026-09-05T00:45:00 运行 olp-evo-metrics.sh
  那么 stdout 含 stall: 43c-2 45 与 stalls: 1
  并且 不含 stall: 43-r1、不含 stall: 37 且退出码等于 0

场景: 片号边界与无日期条目
  测试: olp_evo_metrics_stall_slice_boundaries_and_dateless_entry
  假设 夹具含 `派单 99xyz`、`abc27c 派单`,以及日期为 2026-09-01 的条目之后一条无日期的 `### 44.` 标题下的 `派单 44a` 无 ACK
  当 以 --stall <板> --now 2026-09-05T00:45:00 运行
  那么 stdout 不含 stall: 99x、不含 stall: 27c,含 stall: 44a open

场景: 反序派单与状态词前缀
  测试: olp_evo_metrics_stall_accepts_reverse_order_and_status_words
  假设 夹具含 `27c 派单` 与 `> ACK(27c done):`、`派单 27d` 与 `> ACK(27d blocked):`、`派单 27e` 无 ACK,以及正文行 `任务要求 ACK(27e done)`
  当 以 --stall <板> --now 2026-09-05T00:45:00 运行
  那么 stdout 含 stall: 27e 且不含 stall: 27c、不含 stall: 27d

场景: 阈值内的派单不报停摆
  测试: olp_evo_metrics_stall_respects_threshold
  假设 同上夹具
  当 以 --stall <板> --stall-threshold 60 --now 2026-09-05T00:45:00 运行
  那么 stdout 含 stalls: 0 且不含 stall: 43c-2

场景: 伪 verified 计数与 JSON
  测试: olp_evo_metrics_fake_verified_counts_r2_records
  假设 进化黑板含一张 r2_record 卡与两张 ack_blocked 卡
  当 分别以文本与 --json 运行 olp-evo-metrics.sh
  那么 文本含 fake_verified: 1 且 JSON 的 fake_verified 等于 1
  并且 输出不含 regress

场景: 全套并行通过
  测试: olp_evo_index_is_idempotent
  假设 仓库检出
  当 以 cargo test --all-targets 默认并行运行整个测试套
  那么 olp_evo_index 与 olp_evo_skeleton 两个二进制各自全部通过

场景: 受保护章节 golden 仍绿
  测试: olp_evo_retro_skill_step5_and_protected_sections_golden
  假设 本契约全部落地后的 skill 卡
  当 运行阶段 1 的 golden 测试
  那么 "## 自主性纪律"/"## 模式 init"/"## 模式 inner" 与 description 的 sha256 与阶段 0 基线相同

场景: README 登记 kind 候选
  测试: olp_evo_readme_lists_kind_candidates
  假设 仓库检出
  当 读取 knowledge/context/evolution/README.md
  那么 内容含 iteration_cap 与 patch_failed
