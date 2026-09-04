spec: task
name: "进化环阶段 1:retro 简报脚本、带署名的改判/R2 记档行形、issue 模板与 skill/BOOT 接入"
tags: [olp, evolution, harness, retro]
satisfies: [REQ-OLP-EVO-RETRO]
estimate: 1.5d
---

## 意图

阶段 0 之后诊断仍是外环人肉:读卡、数复发、手写记录骨架。本任务给外环一个机械的 retro 入口
`scripts/olp-evo-retro.sh`:读取上次 retro 之后的卡,按候选分组、列锚点、给层提示,输出一份带
记录草稿的简报;判断(归层、锚定、立案、写记录)仍由持主审锁的外环做。同时把改判与 R2 记档定成
带署名的行首定式并让采集哨收它们(不改阶段 0 的 ACK 触发面),补 issue 模板,把 retro 步骤写进
skill 卡 outer 模式与 BOOT §7。不改运行时代码,不改协议,不写 ACK。契约 v2 已并入 codex 与
grok 的对抗复审。

## 已定决策

- 用法 `scripts/olp-evo-retro.sh <repo-root> [--dry-run]`;状态目录复用阶段 0 的
  `<OLP_EVO_STATE 或 ~/.octos/outer/evo>/<sha256(realpath) 前 16 位>/`,新增 `retro.json`、
  `retro.lock`、`retro/` 目录;进化黑板路径可由 `OLP_EVO_BOARD` 覆盖。`retro.json` 形状:
  `{"last_id":N,"runs":[{"ts":"<UTC RFC3339>","run":<序号>,"cards":N,"candidates":N,"brief":"<绝对路径>"}]}`,
  文件缺失视为 `last_id=0`、`runs=[]`。
- 卡片解析(python3 标准库):以 `### EVO-NNNN（` 开头的行起一张卡,直到下一张;必需字段行
  `trigger:`、`identity:`、`symptom:`,缺任一 → stderr `malformed-card: EVO-NNNN` 并跳过;
  `source:`、`envelope:` 可选(缺失时简报里写 `-`)。
- 分组文本按 trigger 取:`ack_blocked`/`ack_wontdo`/`override`/`r2_record` 取 symptom 去掉行首定式
  前缀(`ACK(blocked):`、`ACK(wontdo):`、`外环(…)·改判(…):`、`外环(…)·R2 记档(…):`)后的正文;
  `escalation`/`turn_error`/`goal_blocked`/`goal_budget_limited` 先 `json.loads(symptom)` 取 `detail`
  (失败退回整行);`report_blocked`/`ask_outer_timeout` 取 `reason=` 之后到行尾。
- 归一化(python,按序):`str.lower()` → 正则 `(?<![\w])/[^\s]+` 替换为 `<path>` → 正则
  `(?<![A-Za-z])[0-9a-f]{8,}(?![A-Za-z])` 替换为 `<hex>` → 正则 `(?<![A-Za-z_])\d+(?![A-Za-z])`
  替换为 `<num>`(`E0596`、`goal_03` 保留)→ `re.sub(r'\s+',' ')` 并 `strip()` → 取前 80 个
  code point。候选键 = `trigger + '|' + 归一化文本`。
- 锚点:`identity` 去掉 `board:`/`events:`/`mcp:` 前缀后用 `rsplit('#')`——board 取倒数第 3 段,
  events/mcp 取倒数第 2 段;值为 `-` 时改用该卡的 `EVO-NNNN`。`anchors` = 去重集合(有序),
  `recurrence_hint` = 集合大小。
- 层提示表:ack_blocked、ack_wontdo、goal_blocked、goal_budget_limited、escalation → `Lifecycle`;
  report_blocked、ask_outer_timeout → `Tooling`;turn_error → `Execution`;override → `Governance`;
  r2_record → `Verification`;其它 → `Observability`。
- 简报格式(Markdown):首行 `# retro <UTC RFC3339> · <项目键> · run <序号>`;`cards: N`、
  `candidates: N` 各一行;固定说明行 `note: recurrence_hint 是去重锚点数,不是跨 goal 计数;草稿
  TODO 未消除前不得保存为 FLAW-NNN.md`;每候选一节 `## C<k> <trigger> · recurrence_hint=<n> ·
  layer=<层>`,含 `key: <候选键>`、`anchors: <a1>, <a2>`、`cards: EVO-…, EVO-…`,每卡一行
  `- EVO-NNNN | source=<…> | envelope=<…> | symptom=<原文前 120 字>`,以及一个 ```yaml 块的记录
  草稿:`draft: true`、`kind: context`、`id: FLAW-NNN`、`title: "<分组文本前 60 字>"`、
  `repo: TODO`、`layers: [<层>]`、`status: open`、`severity: TODO`、`recurrence: <hint>`、
  `fingerprint: TODO`、`cards: [EVO-…]`。
- 下一个 FLAW 编号:扫描 `<repo>/knowledge/context/evolution/FLAW-[0-9]*.md` 取最大编号加一,
  候选间递增;目录不存在或无记录时从 FLAW-001 起。
- 提交协议(非 dry-run):`flock -x <项目状态目录>/retro.lock` 内——读 `retro.json` → 解析卡 →
  若新卡数为 0 则打印 `retro: 0 new card(s)` 退出 0 → 简报写 `retro/<UTC>-<run>.md.tmp`、
  `sync`、`mv -f` 为正式名(`run` = 既有 runs 长度 + 1)→ `runs` 追加一条 → `retro.json.tmp`
  写入、`sync`、`mv -f`。`last_id` = 本次见到的最大卡编号(含畸形卡)。dry-run:不取锁、不创建
  任何文件,简报打印到 stdout。
- 故障注入:仅当 `OLP_EVO_TEST=1` 时读 `OLP_EVO_FAULT`;值 `after-brief` 表示简报落盘后、写
  `retro.json` 前以退出码 70 退出(下次运行会重新处理同批卡并再写一份简报,不丢 runs)。
- 采集哨新增触发器(只改 `scripts/olp-evo-harvest.sh` 活板分支,阶段 0 的 ACK 判定一字不改):
  对每一行另做一次识别——复制该行,剥前导 `> `(可重复)、`**`、空白,若匹配
  `^外环\([^)]+\)·改判\(` → trigger `override`,匹配 `^外环\([^)]+\)·R2 记档\(` → `r2_record`;
  identity 的类型段分别为 `override`/`r2`;symptom 取原行前 200 字符。
- `knowledge/context/evolution/ISSUE-template.md`:frontmatter `repo:`、`evo:`、`layers:`、
  `severity:`;七节 `## Summary`、`## Environment`、`## Reproduction`、`## Root cause`、
  `## Expected behavior`、`## Tests requested`、`## Related`,每节一行占位说明。
- skill 卡:只改"模式 outer"节——`上岗四步` 改 `上岗五步`,第 4 步后新增第 5 步 "retro(进化环)",
  正文含五项:①触发(战役收官,或进化黑板新卡 ≥ 10 张);②命令
  `scripts/olp-evo-harvest.sh <repo> && scripts/olp-evo-retro.sh <repo>`;③处置(每次最多推进 3 条
  记录;立案条件 hint ≥ 2 或主审目视跨 goal/跨条目复发,或 S1;issue 由 operator 发布或明示委托);
  ④authority(未持 outer-duty 锁只读简报不写记录);⑤采集哨只认带署名的行首定式
  `> 外环(<署名>)·改判(作废 #N):` / `> 外环(<署名>)·R2 记档(#N):`,纪律里的散文"R2 记档"不落卡。
  description frontmatter、"模式 init"、"模式 inner"、"自主性纪律"章节逐字不变。
- BOOT §7 新增小节 "进化环批注定式":给出改判的完整条目示例(`### 4N. 改判作废 #40(日期,署名)`
  下一行 `> 外环(<署名>)·改判(作废 #40):<以本条为准的新指令>`)与 R2 记档行示例
  (`> 外环(<署名>)·R2 记档(#41):<声称 vs 复验事实>`),写明"改判必须落在新的未 ACK 编号条目
  (R1/R5),已闭环条目下的批注不唤醒内环;本行形同时供采集哨识别;行首定式,正文提及不算"。
  §0 至 §6 逐字不变。
- `docs/OCTOLOOP_FEATURES.md` "结果与审计"节新增一条"进化环(阶段 0/1)":是什么、缺省状态
  (手动运行)、用户怎么看到(`.octos/EVOLUTION.md` 与 retro 简报);固定短语"外环私有工作纸,不写入
  OLP 信道矩阵,不升协议版本"。
- `knowledge/context/evolution/README.md` 加 "retro" 一段:简报位置、草稿到记录的转换规则
  (消除 TODO、补锚点与根因后另存为 FLAW-NNN.md)。
- 受保护章节 golden:`tests/olp_evo_retro.rs` 内以常量保存阶段 0 基线(origin/main 18907aa)的
  sha256——skill 卡 description 行、"## 模式 init" 节、"## 模式 inner" 节、"## 自主性纪律" 节
  (各自从节标题到下一个 `## ` 标题前)、BOOT `## 0.` 到 `## 7.` 标题前的正文;测试用同样切法读当前
  文件比对。
- 测试:`tests/olp_evo_retro.rs`(Rust 集成测试,`std::process::Command` 调脚本,夹具复制到
  `std::env::temp_dir()` 唯一子目录,`OLP_EVO_STATE` 指向临时状态根;夹具放
  `fixtures/evolution/retro/`,其中进化黑板样本的 identity 行手写为阶段 0 `emit_card` 的真实形状,
  不得复制 `fixtures/evolution/events.jsonl` 派生);采集哨新增触发器的测试加在
  `tests/olp_evo_harvest.rs`。不新增 Cargo 依赖;脚本只依赖 bash、coreutils、flock、python3。

<!-- lint-ack: decision-coverage — 用法/状态形状/简报格式等决策由多个场景共同行使,不单列场景 -->

## 边界

### Allowed Changes
- scripts/olp-evo-retro.sh
- scripts/olp-evo-harvest.sh
- tests/olp_evo_retro.rs
- tests/olp_evo_harvest.rs
- fixtures/evolution/**
- knowledge/context/evolution/ISSUE-template.md
- knowledge/context/evolution/README.md
- .claude/skills/octoloop/SKILL.md
- docs/OLP_OUTER_BOOT.md
- docs/OCTOLOOP_FEATURES.md

### Forbidden
- 不改 `src/**` 任何运行时代码。
- 不改 `AGENTS.md`、`.octos/loop.md`、`docs/OUTER_LOOP_PROTOCOL.md`、`tests/olp_contract.rs`。
- 不改 skill 卡 description frontmatter 与"模式 init/inner"、"自主性纪律"章节(golden 钉住)。
- 不改 BOOT §0 至 §6(golden 钉住)。
- 不改 `knowledge/context/evolution/operators.md`、`FLAW-*.md`、`memory.md`。
- 不改阶段 0 的 ACK 触发判定;不向审查活板写入任何内容;retro 脚本不写进化黑板、不写记录目录。
- 不新增 Cargo 依赖,不新增 MCP 工具。

## 排除范围

- retro 的判断部分(归层、锚定、立案、写记录)——由外环模型按简报手工完成。
- events.jsonl 新 producer(octos 侧,阶段 2)。
- 指标脚本、回放夹具、采集挂外环 watch 节拍(阶段 2/3)。
- `docs/OUTER_LOOP_PROTOCOL.md` 的任何改动。

## 完成条件

场景: 不同错误码不合并(critical)
  标签: critical
  测试: olp_evo_retro_error_codes_are_distinct_candidates
  假设 进化黑板含两张 ack_blocked 卡,正文分别含 E0596 与 E0382,其余相同
  当 运行 olp-evo-retro.sh
  那么 简报含 candidates: 2

场景: events 卡按 detail 分组(critical)
  标签: critical
  测试: olp_evo_retro_events_group_by_detail
  假设 进化黑板含两张 turn_error 卡,symptom 为 JSON 整行,ts 与 goal_id 不同,detail 分别为 writer stalled 与 provider 429
  当 运行 olp-evo-retro.sh
  那么 简报含 candidates: 2
  并且 两个候选的 key 行分别含 writer stalled 与 provider

场景: 仅数字与路径不同的卡合并并数出锚点
  测试: olp_evo_retro_merges_num_path_variants_and_counts_anchors
  假设 进化黑板含两张 ack_blocked 卡,正文仅路径与纯数字不同,identity 条目分别为 12 与 13,另有一张 turn_error 卡
  当 运行 olp-evo-retro.sh
  那么 简报含 candidates: 2
  并且 ack_blocked 候选行含 recurrence_hint=2
  并且 该候选的 anchors 行含 12 与 13
  并且 turn_error 候选行含 layer=Execution

场景: 路径含井号时锚点仍正确
  测试: olp_evo_retro_anchor_rsplit_survives_hash_in_path
  假设 进化黑板含一张 board 卡,其 identity 路径含 # 字符,条目为 27
  当 运行 olp-evo-retro.sh
  那么 该候选的 anchors 行含 27

场景: 锚点为减号的卡各计一次
  测试: olp_evo_retro_dash_anchor_counts_each_card
  假设 进化黑板含两张 override 卡,identity 条目段均为 -
  当 运行 olp-evo-retro.sh
  那么 该候选行含 recurrence_hint=2

场景: 草稿标注 draft 且用下一个 FLAW 编号
  测试: olp_evo_retro_draft_marks_todo_and_next_flaw_id
  假设 仓库记录目录含 FLAW-001.md 与 FLAW-002.md,进化黑板含两个不同候选的卡
  当 运行 olp-evo-retro.sh
  那么 简报中出现 id: FLAW-003 与 id: FLAW-004
  并且 两段草稿均含 draft: true 与 fingerprint: TODO
  并且 简报含 不得保存为 FLAW-NNN.md
  并且 记录目录中不存在 FLAW-003.md

场景: 简报与状态 schema 完整
  测试: olp_evo_retro_brief_and_runs_schema
  假设 进化黑板含 3 张卡,运行一次后又追加 1 张卡再运行一次
  当 解析两份简报与 retro.json
  那么 每份简报含 cards:、candidates:、note: 行,每候选含 key:、anchors:、cards: 与逐卡 source= envelope= 行
  并且 retro.json 的 last_id 等于 4,runs 长度为 2,第一条记录未变,两条 brief 指向不同且存在的文件

场景: 游标推进后重跑零新卡
  测试: olp_evo_retro_cursor_advances_and_rerun_is_empty
  假设 已对含 3 张卡的进化黑板运行过一次 retro
  当 再次运行 olp-evo-retro.sh
  那么 stdout 含 retro: 0 new card(s)
  并且 retro 目录中的简报文件数等于 1

场景: 并发两次运行不丢记录
  测试: olp_evo_retro_concurrent_runs_keep_records
  假设 同一状态目录与含新卡的进化黑板
  当 同时启动两个 olp-evo-retro.sh 进程并等待退出
  那么 runs 长度为 1 且另一进程 stdout 含 retro: 0 new card(s)
  或者 runs 长度为 2 且两份简报文件名不同

场景: 简报落盘后写游标前崩溃可恢复
  测试: olp_evo_retro_recovers_after_crash_before_cursor
  假设 以 OLP_EVO_TEST=1 与 OLP_EVO_FAULT=after-brief 运行一次
  当 去掉故障注入再次运行
  那么 第一次退出码等于 70
  并且 第二次运行后 retro.json 的 last_id 等于最大卡编号且 runs 长度为 1

场景: dry-run 零写入
  测试: olp_evo_retro_dry_run_writes_nothing
  假设 进化黑板含新卡且状态目录不存在 retro.json
  当 以 --dry-run 运行 olp-evo-retro.sh
  那么 stdout 含 candidates:
  并且 状态目录中不存在 retro.json、retro.lock 与 retro 目录

场景: 畸形卡被报告一次并跳过
  测试: olp_evo_retro_malformed_card_reported_once
  假设 进化黑板含一张缺少 identity 行的卡与一张完整的卡
  当 运行 olp-evo-retro.sh 两次
  那么 第一次 stderr 含 malformed-card: 且简报含 candidates: 1
  并且 第二次 stderr 不含 malformed-card: 且 stdout 含 retro: 0 new card(s)

场景: 无新卡退出 0
  测试: olp_evo_retro_no_cards_exit_zero
  假设 进化黑板不存在或不含任何卡
  当 运行 olp-evo-retro.sh
  那么 退出码等于 0
  并且 stdout 含 retro: 0 new card(s)
  并且 状态目录中不存在 retro 目录

场景: 带署名的改判与 R2 记档行触发采集,既有写法不触发(critical)
  标签: critical
  测试: olp_evo_harvest_signed_override_and_r2_lines_trigger_only
  假设 活板含 > 外环(claude)·改判(作废 #40):以本条为准、> 外环(codex)·R2 记档(#41):声称 verified 复验不符、主审改判(见上)、**R2 违例记档**:…、被证伪(R2 记档,非恶意、> 判词(38-r1)…、> ACK(blocked): foo 七行
  当 运行 olp-evo-harvest.sh
  那么 进化黑板恰新增两张卡
  并且 两张卡的 trigger 行分别为 override 与 r2_record

场景: issue 模板与 FEATURES 就位
  测试: olp_evo_retro_issue_template_and_features_in_place
  假设 仓库检出
  当 读取 ISSUE-template.md 与 OCTOLOOP_FEATURES.md
  那么 模板含七节标题
  并且 OCTOLOOP_FEATURES.md 含 外环私有工作纸

场景: skill 卡 outer 第 5 步就位且受保护章节原文不变
  测试: olp_evo_retro_skill_step5_and_protected_sections_golden
  假设 阶段 0 基线的 description 行与 init、inner、自主性纪律章节 sha256 常量
  当 读取当前 SKILL.md
  那么 上述四段的 sha256 与常量相等
  并且 outer 节含 上岗五步 与 olp-evo-retro.sh 与 R2 记档 与 operator

场景: BOOT §7 新增定式且 §0 至 §6 原文不变
  测试: olp_evo_retro_boot_section7_and_golden
  假设 阶段 0 基线 BOOT §0 至 §6 正文的 sha256 常量
  当 读取当前 OLP_OUTER_BOOT.md
  那么 §0 至 §6 正文的 sha256 与常量相等
  并且 §7 含 改判(作废 # 与 R2 记档(# 与 未 ACK
