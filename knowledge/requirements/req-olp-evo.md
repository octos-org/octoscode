---
kind: requirement
id: REQ-OLP-EVO
title: "进化环阶段 0:外环侧三源采集哨、进化黑板、缺陷记录目录"
status: accepted
liveness: auto
tags: [olp, evolution, harness, observability]
---

## Problem

内环 harness 的缺陷(预算耗尽无 ACK、围栏 peer 冷编译、goal 陈旧卡态)只在事故后靠人肉复盘沉淀成散文,下一场战役再踩。外环需要一个机械的、幂等的、崩溃一致的采集面,把内环摩擦从三个既有来源增量落成带稳定身份的症状卡,并有一个入库的缺陷记录目录供主审合并与立案。阶段 0 只读:不改运行时、不改协议、不写 ACK。

## Requirements

[REQ-OLP-EVO-SOURCES] 采集脚本 `scripts/olp-evo-harvest.sh` MUST 只从活板 `<repo>/.octos/OUTER_LOOP_REVIEW.md`、实例 `events.jsonl`、MCP 审计板 `OUTER_LOOP_MCP.md` 三个来源读取,并 MUST NOT 读取 `docs/OUTER_LOOP_REVIEW.md` 冻结快照。

[REQ-OLP-EVO-SYNTAX-BOARD] 活板触发行 MUST 是去掉前导空白后以 `ACK(blocked):` 或 `ACK(wontdo):` 开头的 R1 回执行;出现在其它位置的同名子串 MUST NOT 触发。

[REQ-OLP-EVO-SYNTAX-MCP] MCP 审计板触发行 MUST 匹配 `- <时间戳> MCP(ask_outer) <kind>: ` 行形且 kind 为 `blocked`(记为 report_blocked)或 `timeout`(记为 ask_outer_timeout);kind 为 `ask`、`answer`、`refusal` 的行 MUST NOT 触发。

[REQ-OLP-EVO-SYNTAX-EVENTS] events.jsonl 每行 MUST 先按 JSON 解析,`kind` 为字符串 `escalation` 或 `turn_error` 时触发,`kind` 为 `goal_transition` 且解析后的 `detail` 等于 ``goal transitioned to `blocked` `` 或 ``goal transitioned to `budget_limited` `` 时触发;无法解析的行 MUST 在 stderr 打印 `malformed:` 并跳过。

[REQ-OLP-EVO-IDENTITY] 每张卡 MUST 携带来源感知的 `identity` 字段:活板为 `board:<活板规范路径>#<所属 ### 条目编号>#<ACK 类型>#<行 sha256>`,events 为 `events:<路径>#<ts>#<kind>#<goal_id|slug|session 之一或 ->#<行 sha256>`,MCP 为 `mcp:<路径>#<时间戳>#<kind>#<ask id 或 ->#<行 sha256>`,其中 sha256 为完整 64 位十六进制。

[REQ-OLP-EVO-CARD] 每张卡 MUST 以 `### EVO-NNNN` 开头并按顺序携带 `trigger:`、`source:`、`identity:`、`envelope:`(单行,字段顺序为 `line=<n> offset=<bytes> ts=<UTC RFC3339>`)、`symptom:` 五个字段行,且 MUST NOT 含建议或修复方案字段。

[REQ-OLP-EVO-PRIVACY] MCP 来源的卡 MUST 只在 `symptom` 保留 kind、ask id 与 `reason=` 之后至多 80 字符,MUST NOT 复制 `question=`、`context=`、`tried=` 之后的正文;活板与 events 来源的 `symptom` MUST 截断为触发行的前 200 字符。

[REQ-OLP-EVO-IDEMPOTENT] 相同 `identity` MUST NOT 产生第二张卡;不同活板条目下逐字相同的 ACK 行 MUST 各产生一张卡。

[REQ-OLP-EVO-CURSOR] 每源游标的权威字段 MUST 是最后一条已提交的完整换行记录之后的字节偏移,并与文件 identity(设备号与 inode)和偏移前最多 64 字节的 sha256 一起保存;文件尾无换行的半条记录 MUST NOT 触发且 MUST NOT 推进偏移。

[REQ-OLP-EVO-RESET] 来源文件 identity 改变、偏移前缀摘要不匹配或当前字节数小于偏移时,脚本 MUST 把该源偏移重置为 0、在 stderr 打印 `reset:` 并只靠 identity 去重,MUST NOT 重复落卡。

[REQ-OLP-EVO-STATE] 状态 MUST 位于 `<OLP_EVO_STATE 或 ~/.octos/outer/evo>/<仓库规范路径 sha256 前 16 位>/` 目录下,EVO 编号 MUST 按项目从 `EVO-0001` 起单调递增,缺省状态目录 MUST NOT 位于 /tmp。

[REQ-OLP-EVO-COMMIT] 在 `harvest.lock` 的 `flock -x` 内,脚本 MUST 按"解析并预检全部候选 → 从进化黑板已有卡的 identity 与状态文件对账恢复最大编号与已见集合 → 追加卡 → 以同目录临时文件加 fsync 加 rename 原子替换状态文件"顺序提交,MUST NOT 在追加卡之前推进任何状态。

[REQ-OLP-EVO-CONCURRENT] 两个并发运行的采集进程 MUST 产生唯一编号,且每个触发器恰好产生一张卡。

[REQ-OLP-EVO-REQUIRED] 活板缺失时脚本 MUST 以退出码 2 失败,且该检查 MUST 发生在创建任何目录、锁或状态文件之前。

[REQ-OLP-EVO-OPTIONAL] events.jsonl 或 MCP 审计板缺失时脚本 MUST 跳过该源、在 stderr 打印 `skip:` 并以退出码 0 结束。

[REQ-OLP-EVO-DRYRUN] `--dry-run` MUST 把将要落的卡打印到 stdout,MUST NOT 创建或修改状态目录、锁、进化黑板中的任何文件字节。

[REQ-OLP-EVO-RECORDS] 缺陷记录、修复记忆、算子表 MUST 位于本仓库 `knowledge/context/evolution/`,每份 `FLAW-*.md` 的 frontmatter MUST 含 `kind: context`、唯一 `id`、`repo`(`octos-org/octos` 或 `ZhangHanDong/octoscode` 形式的 owner/name)、`layers`、`status`(open、consolidated、filed、accepted、specified、patched、verified、closed、rejected、reopened 之一)、`severity`(S1、S2、S3 之一)、非负整数 `recurrence`、非空 `fingerprint`。

[REQ-OLP-EVO-ISSUE] `status` 为 `filed` 或其后任一状态的缺陷记录 MUST 在 frontmatter 含以 `https://github.com/` 开头的 `issue` 链接。

[REQ-OLP-EVO-INIT] `scripts/olp-init.sh` MUST 以 `git check-ignore -q .octos/EVOLUTION.md` 判定,未被忽略时把 `.octos/EVOLUTION.md` 追加进 `.gitignore`,已被忽略(含整目录 `.octos` 规则)时 MUST 跳过且 MUST NOT 重复追加。

## Scenarios

Scenario: 全部触发器种类各落一卡
  Given 夹具活板含 ACK(blocked) 与 ACK(wontdo) 各一行,events.jsonl 含 escalation、turn_error、goal_transition(blocked)、goal_transition(budget_limited) 各一行,MCP 审计板含 blocked 与 timeout 各一行
  When 以空状态目录运行 olp-evo-harvest.sh
  Then 进化黑板中以 `### EVO-` 开头的行数等于 8
  And 编号依次为 EVO-0001 到 EVO-0008
  And 每张卡含以 `identity:`、`envelope:`、`symptom:` 开头的行

Scenario: 负例矩阵零卡
  Given 活板任务书正文含 `落 ACK(45a done|blocked)` 字样但无行首 ACK 回执,MCP 审计板只含 ask、answer、refusal 行,events.jsonl 只含 goal_transition(complete) 与一行非法 JSON
  When 运行 olp-evo-harvest.sh
  Then 进化黑板中以 `### EVO-` 开头的行数等于 0
  And stderr 含 `malformed:`

Scenario: identity 区分条目并在重跑时去重
  Given 活板条目 ### 12 与 ### 13 下各有一行逐字相同的 ACK(blocked)
  When 运行 olp-evo-harvest.sh 两次
  Then 进化黑板中以 `### EVO-` 开头的行数等于 2
  And 两张卡的 identity 字段不相等

Scenario: 采集从不触碰活板
  Given 任意夹具
  When 运行 olp-evo-harvest.sh
  Then 活板的 sha256 与运行前相等
  And 进化黑板中以 `ACK(` 开头的行数等于 0

Scenario: docs 冻结快照被忽略
  Given docs/OUTER_LOOP_REVIEW.md 含一行新的 ACK(blocked) 而活板无新行
  When 运行 olp-evo-harvest.sh
  Then 进化黑板中以 `### EVO-` 开头的行数与运行前相等

Scenario: 半行不触发,补齐换行后恰一卡
  Given events.jsonl 尾部是一条无换行的 turn_error 记录
  When 运行 olp-evo-harvest.sh,再补上换行符后再次运行
  Then 第一次运行后卡数等于 0 且偏移不变
  And 第二次运行后卡数等于 1

Scenario: 截断或替换后重置且不重复
  Given 已采集过的 events.jsonl 先被截断为最后一行,再被替换为含两条新 escalation 的更大文件
  When 每次变更后运行 olp-evo-harvest.sh
  Then 截断后 stderr 含 `reset:` 且卡数不变
  And 替换后 stderr 含 `reset:` 且卡数恰增加 2

Scenario: 追加卡后提交状态前崩溃可恢复
  Given 以 OLP_EVO_TEST=1 与 OLP_EVO_FAULT=after-append 运行一次采集使其在追加卡后退出
  When 去掉故障注入再次运行
  Then 进化黑板中以 `### EVO-` 开头的行数等于触发器数
  And 状态文件中的偏移等于来源文件字节数

Scenario: 并发采集编号唯一
  Given 同一夹具与状态目录
  When 同时启动两个 olp-evo-harvest.sh 进程并等待二者退出
  Then 进化黑板中以 `### EVO-` 开头的行数等于触发器数
  And 所有 EVO 编号互不相同

Scenario: 活板缺失即失败且零创建
  Given 仓库目录下不存在 .octos/OUTER_LOOP_REVIEW.md 且状态根目录可写
  When 运行 olp-evo-harvest.sh
  Then 退出码等于 2
  And 状态根目录下不存在任何新文件或目录

Scenario: dry-run 对已有状态零写入
  Given 已运行过一次采集且来源新增一条触发行
  When 以 --dry-run 运行 olp-evo-harvest.sh
  Then stdout 含 `### EVO-`
  And 进化黑板与状态目录内所有文件的 sha256 与运行前相等

Scenario: 可选来源缺失时跳过
  Given 只有活板存在,OLP_EVO_EVENTS 为空且 OLP_EVO_MCP_BOARD 指向不存在的路径
  When 运行 olp-evo-harvest.sh
  Then 退出码等于 0
  And stderr 含 `skip:`

Scenario: 状态按项目隔离
  Given 两个仓库目录各有一块含一行 ACK(blocked) 的活板
  When 对两个仓库各运行一次 olp-evo-harvest.sh
  Then 两块进化黑板各含一张 EVO-0001
  And 状态根目录下存在两个不同的项目子目录

Scenario: 记录目录与记录校验
  Given 仓库检出
  When 解析 knowledge/context/evolution/ 下全部 FLAW-*.md 的 frontmatter
  Then README.md、FLAW-template.md、memory.md、operators.md 四个文件存在
  And 每份记录的必填字段齐全、id 唯一、status 与 severity 取值合法、recurrence 为非负整数、fingerprint 非空
  And FLAW-001.md 与 FLAW-002.md 的 issue 字段分别含 `issues/2236` 与 `issues/2237`

Scenario: olp-init 为只忽略活板的项目追加 EVOLUTION.md 忽略
  Given 一个临时 git 仓库,其 .gitignore 只含 `.octos/OUTER_LOOP_REVIEW.md`
  When 运行 scripts/olp-init.sh 两次
  Then .gitignore 中 `.octos/EVOLUTION.md` 恰出现一次

Scenario: olp-init 对整目录已忽略的项目不追加
  Given 一个临时 git 仓库,其 .gitignore 含 `.octos`
  When 运行 scripts/olp-init.sh
  Then .gitignore 不含 `.octos/EVOLUTION.md`

## Dependencies

- REQ-OLP-OBS(events.jsonl 字段与 kind 集合)
- REQ-OLP-PROTO(ACK v1 定式)

## Source Trace

- proposal:LEP-003(operator 2026-09-04 直令"进化环开始落地,依然 sdd";契约经 codex 第二外环对抗复审 REQUEST_CHANGES 后修订)
- issue:octos-org/octos#2236、#2237(进化环首两条候选,已立案)
- 实测:2026-09-04 octos 活板 #45 战役,围栏 peer 冷编译耗尽 50 迭代、goal_create 被 archived 挡住
- 代码:`src/olp_mcp.rs` L150–L172(审计行形 `- <ts> MCP(ask_outer) <kind>: <detail>`,kind 为 ask/answer/timeout/refusal/blocked)

## Open Questions

- events.jsonl 的实例发现:阶段 0 由调用方显式传路径,自动发现留待阶段 2。
- 改判与 R2 打回的固定行形属规程改动(operator-tier),阶段 0 不作为触发器。

## Next

Single exit: compile this requirement into a task contract(`specs/task-req-olp-evo-p0.spec.md`)。
