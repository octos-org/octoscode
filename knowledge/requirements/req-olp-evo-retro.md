---
kind: requirement
id: REQ-OLP-EVO-RETRO
title: "进化环阶段 1:retro 简报入口、两条带署名的固定批注行形、issue 模板"
status: accepted
liveness: auto
tags: [olp, evolution, harness, retro]
---

## Problem

阶段 0 让感知机械化(采集哨落卡)、记录入库(缺陷记录目录),但"诊断"仍靠外环人肉:读一堆卡、手数复发、手写记录骨架、手起草 issue。本阶段给外环一个机械的 retro 入口:把上次 retro 之后的卡按候选分组、给出锚点与复发提示、层提示与记录草稿,输出一份 retro 简报;判断(归层、锚定、立案、写记录)仍由持主审锁的外环做。同时把外环两种高价值批注(改判、R2 记档)定成带署名的行首定式,让采集哨也能收它们,且不改变阶段 0 已钉的 ACK 触发面。

## Requirements

[REQ-OLP-EVO-RETRO-INPUT] `scripts/olp-evo-retro.sh <repo-root> [--dry-run]` MUST 只读取 `<repo>/.octos/EVOLUTION.md` 中编号大于 `retro.json` 的 `last_id`(文件缺失视为 0)的卡,并 MUST NOT 读取或写入审查活板、进化黑板与记录目录。

[REQ-OLP-EVO-RETRO-TEXT] 分组文本 MUST 按 trigger 取:活板类(ack_blocked、ack_wontdo、override、r2_record)取 symptom 去掉行首定式前缀后的正文;events 类取 symptom 按 JSON 解析后的 `detail` 字段(解析失败退回整行);MCP 类取 `reason=` 之后的文本。

[REQ-OLP-EVO-RETRO-NORM] 归一化 MUST 按顺序执行:Unicode 小写;形如 `/x/y` 的路径片段替换为 `<path>`;不与字母相邻的 ≥ 8 位十六进制串替换为 `<hex>`;不与字母相邻的纯数字串替换为 `<num>`(形如 `E0596`、`goal_03` 的字母加数字 token 保留);ASCII 空白折叠为单空格并去首尾;截前 80 个 Unicode code point。候选键 = `trigger` + `|` + 归一化文本。

[REQ-OLP-EVO-RETRO-ANCHOR] 每张卡的锚点 MUST 从 `identity:` 行用从右侧 `rsplit('#')` 取:`board:` 取倒数第 3 段(条目编号),`events:` 与 `mcp:` 取倒数第 2 段(goal_id|slug|session 或 ask id);锚点为 `-` 时 MUST 以该卡的 `EVO-NNNN` 编号作唯一锚点。

[REQ-OLP-EVO-RETRO-RECURRENCE] 每个候选 MUST 列出其去重后的锚点集合 `anchors` 并给出 `recurrence_hint` = 该集合大小;简报 MUST 注明 hint 不是跨 goal 计数。

[REQ-OLP-EVO-RETRO-LAYER] 层提示 MUST 只取 `operators.md` 已有层名:ack_blocked、ack_wontdo、goal_blocked、goal_budget_limited、escalation → `Lifecycle`;report_blocked、ask_outer_timeout → `Tooling`;turn_error → `Execution`;override → `Governance`;r2_record → `Verification`;未知 trigger → `Observability`。

[REQ-OLP-EVO-RETRO-BRIEF] 简报 MUST 写到 `<项目状态目录>/retro/<UTC 时间戳>-<run 序号>.md`,MUST 含 `cards:` 与 `candidates:` 计数、每候选的键、触发器、层提示、anchors、recurrence_hint、卡片编号与每卡的 `source`/`envelope`,以及一段 frontmatter 含 `draft: true` 的记录草稿。

[REQ-OLP-EVO-RETRO-DRAFT] 记录草稿 MUST 标注 `draft: true` 且 `repo`、`severity`、`fingerprint` 为 `TODO`,简报 MUST 写明"TODO 未消除前不得保存为 FLAW-NNN.md";草稿 `id` MUST 取 `knowledge/context/evolution/FLAW-*.md` 现有最大编号加一,候选间依次递增。

[REQ-OLP-EVO-RETRO-COMMIT] 非 dry-run 运行 MUST 在 `<项目状态目录>/retro.lock` 的 `flock -x` 内完成:写简报(临时文件、fsync、rename)→ 追加 `runs` 记录 → 原子替换 `retro.json`;`last_id` MUST 推进到本次见到的最大卡编号(含畸形卡);`runs` MUST 只追加不覆盖。

[REQ-OLP-EVO-RETRO-CONCURRENT] 两个并发运行 MUST 产生两份不同文件名的简报或其中一个报告零新卡,`runs` 记录 MUST 不丢失。

[REQ-OLP-EVO-RETRO-DRYRUN] `--dry-run` MUST 把简报打印到 stdout,并 MUST NOT 创建或修改 `retro.json`、`retro.lock` 与 `retro/` 目录。

[REQ-OLP-EVO-RETRO-MALFORMED] 缺少 `trigger:`、`identity:` 或 `symptom:` 行的卡 MUST 在 stderr 以 `malformed-card: EVO-NNNN` 报告并跳过,MUST NOT 中止运行,且因 `last_id` 推进 MUST NOT 在下次运行重复报告。

[REQ-OLP-EVO-RETRO-EMPTY] 没有新卡时脚本 MUST 以退出码 0 结束并在 stdout 打印 `retro: 0 new card(s)`,MUST NOT 写简报。

[REQ-OLP-EVO-RETRO-OVERRIDE] 采集哨 MUST 把活板中去掉前导 `> `、`**` 与空白后匹配 `^外环\([^)]+\)·改判\(` 的行识别为 `override`,匹配 `^外环\([^)]+\)·R2 记档\(` 的行识别为 `r2_record`;该前缀剥离 MUST 只用于这两种识别,阶段 0 的 ACK 识别面 MUST NOT 改变。

[REQ-OLP-EVO-RETRO-ISSUE] `knowledge/context/evolution/ISSUE-template.md` MUST 存在并含 Summary、Environment、Reproduction、Root cause、Expected behavior、Tests requested、Related 七节标题。

[REQ-OLP-EVO-RETRO-SKILL] `/octoloop` skill 卡的 outer 模式 MUST 新增第 5 步 retro,写明触发时机、命令、每次最多推进 3 条记录、立案条件(hint ≥ 2 或主审目视跨 goal/跨条目复发,或 S1)、issue 发布归 operator、未持锁只读简报、采集哨只认带署名的行首定式;init/inner/自主性纪律章节与 description MUST 保持原文。

[REQ-OLP-EVO-RETRO-BOOT] `docs/OLP_OUTER_BOOT.md` §7 MUST 新增小节给出两条批注的行形与完整条目示例,并写明改判 MUST 落在新的未 ACK 编号条目中;§0 至 §6 MUST 保持原文。

## Scenarios

Scenario: 不同错误码不合并
  Given 两张 ack_blocked 卡的正文分别含 E0596 与 E0382
  When 运行 olp-evo-retro.sh
  Then 简报含 `candidates: 2`

Scenario: events 卡按 detail 分组
  Given 两张 turn_error 卡的 symptom 是 JSON 整行,detail 字段不同
  When 运行 olp-evo-retro.sh
  Then 简报含 `candidates: 2`

Scenario: 仅数字与路径不同的卡合并并数出锚点
  Given 两张 ack_blocked 卡正文仅路径与纯数字不同,identity 条目分别为 12 与 13
  When 运行 olp-evo-retro.sh
  Then 简报含 `candidates: 1` 且该候选 `recurrence_hint=2`

Scenario: 锚点为 - 的卡各计一次
  Given 两张 override 卡的 identity 条目段均为 -
  When 运行 olp-evo-retro.sh
  Then 该候选 `recurrence_hint=2`

Scenario: 草稿标注 draft 且用下一个 FLAW 编号
  Given 记录目录已有 FLAW-001 与 FLAW-002
  When 运行 olp-evo-retro.sh 得到两个候选
  Then 简报中的草稿 id 依次为 FLAW-003 与 FLAW-004 且各含 `draft: true`

Scenario: 游标与 runs 只追加
  Given 已对 3 张卡运行过一次,之后又追加 1 张卡并再运行一次
  When 读取 retro.json
  Then `last_id` 等于 4 且 `runs` 长度为 2 且第一条记录未变

Scenario: 并发两次运行不丢记录
  Given 同一状态目录与含新卡的进化黑板
  When 同时启动两个 retro 进程并等待退出
  Then `runs` 长度为 1 且另一进程输出 `retro: 0 new card(s)`,或 `runs` 长度为 2 且两份简报文件名不同

Scenario: 带署名的改判与 R2 记档行触发采集,既有写法不触发
  Given 活板含 `> 外环(claude)·改判(作废 #40):…`、`> 外环(codex)·R2 记档(#41):…`、`主审改判(见上)`、`**R2 违例记档**:…`、`(R2 记档,非恶意`、`> 判词(38-r1)…`、`> ACK(blocked): foo`
  When 运行 olp-evo-harvest.sh
  Then 进化黑板恰新增两张卡,trigger 分别为 override 与 r2_record

Scenario: 受保护章节原文不变
  Given 阶段 0 基线的 skill 卡 description、init、inner、自主性纪律章节与 BOOT §0 至 §6 的 sha256
  When 读取阶段 1 的文件
  Then 上述章节 sha256 与基线相等且 outer 模式含第 5 步

## Dependencies

- REQ-OLP-EVO(卡片格式、进化黑板、记录目录、状态目录)
- REQ-OLP-PROTO(黑板与 ACK 定式)

## Source Trace

- proposal:LEP-003 §Decision 第 2、3 项与 §Unresolved Questions(改判/R2 定式)
- operator 2026-09-05 直令"开始阶段 1,依然 sdd"
- 契约经 codex 与 grok 第二外环对抗复审(REQUEST_CHANGES,2026-09-05):分组键吞根因、行形缺署名、剥前缀作用域、草稿冒充记录、identity 分段、并发/游标、受保护章节 golden

## Open Questions

- 候选键在真实战役卡片上的误合并/误拆分率,阶段 2 用回放夹具校准。

## Next

Single exit: compile this requirement into a task contract(`specs/task-req-olp-evo-p1.spec.md`)。
