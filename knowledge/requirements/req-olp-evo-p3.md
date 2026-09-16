---
kind: requirement
id: REQ-OLP-EVO-P3
title: "进化环阶段 3:采集挂外环节拍、缺陷记录直出规格骨架、散文沉淀索引化、跨源停摆诊断"
status: accepted
liveness: auto
tags: [olp, evolution, harness, automation]
---

## Problem

阶段 0–2 让采集、retro、回放基线与窗口化指标全部机械化,但四处仍靠主审手动:①采集只在 retro 前手跑,活板在战役中间的摩擦要等收官才进黑板,主审复盘时上下文已冷;②FLAW 记录到任务契约之间是一段手写(FLAW-001/002 各手写一份 octos 契约),记录里已有的症状/责任步/根因/锚点/结案/保护门没有被机械复用;③`docs/OUTER_LOOP_PROTOCOL.md` 的"实战沉淀"散文与 FLAW 记录并存,新读者不知道哪条已被记录;④"派单后无 ACK"与"伪 verified(外环 R2 记档)"两项跨源信号今天只在主审记忆里,阶段 2 明确留待本阶段。另有两类摩擦没有任何事件来源:内环达到迭代上限停机、内环就地补丁失败留下编译错误(2026-09-05 octos 活板 #48 实测),本阶段只登记为 kind 候选。

## Requirements

[REQ-OLP-EVO-P3-CADENCE] `scripts/olp-watch-board.sh` MUST 提供 `--harvest <repo-root>` 选项:命中 token 时先调用采集脚本(依次查 `<repo-root>/scripts/olp-evo-harvest.sh`、脚本同目录;都缺按失败处理,安装版监视器因此可用),再打印 `BOARD-SIGNAL` 与命中行,然后把基线推进到当前行数并继续监视(不退出);不带该选项时保持既有一击退出语义。采集非零退出 MUST NOT 中断监视,MUST 在 stderr 打印 `harvest: failed (exit N)` 一行。

[REQ-OLP-EVO-P3-CADENCE-IDEMPOTENT] 节拍采集 MUST 与主审手跑采集共用同一状态根(`OLP_EVO_STATE` 或缺省)与同一 `harvest.lock`,同一触发行在节拍与手跑并发时 MUST 只落一张卡;监视器 MUST NOT 改写 `OLP_EVO_STATE`。

[REQ-OLP-EVO-P3-SKELETON] `scripts/olp-evo-spec-skeleton.sh <FLAW-NNN.md> [--out <file>]` MUST 从 FLAW 记录生成一份 `agent-spec parse` 可解析的任务契约骨架,段映射:`## 意图` ← 症状 + 根因;`## 已定决策` ← `## 修复`,缺则 `## 结案`;`### Forbidden` ← `## 预防`,缺则 `## 保护门`;`### Allowed Changes` ← 责任步与锚点两段中含 `/` 的反引号路径(去重、去行号后缀);`## 完成条件` ← 决策来源段的每个列表项一条场景(无列表则整段一项),测试选择器 `pending_<ASCII slug>`,slug 无 ASCII 字符时为 `pending_item_<n>`;缺段以 `<!-- TODO: <段名> -->` 占位而非失败;frontmatter `satisfies` 取 FLAW frontmatter 的 `req` 字段,缺则 `[]` 并在 `## 问题` 段列出"未绑定需求"。

[REQ-OLP-EVO-P3-SKELETON-NOAUTH] 骨架 MUST NOT 被自动写入 `specs/`;缺省输出到 stdout,`--out` 只接受仓库外路径或 `specs/drafts/` 下路径(目录不存在则创建),其它路径退出 2;判定以目标路径 realpath 所属仓库为准,MUST NOT 依赖调用时的 cwd。

[REQ-OLP-EVO-P3-INDEX] `scripts/olp-evo-index.sh <repo-root>` MUST 由 `knowledge/context/evolution/FLAW-*.md` 生成 `knowledge/context/evolution/INDEX.md`:每条 FLAW 一行(id、status、layers、issue/pr、被其取代的散文——由 `docs/OUTER_LOOP_PROTOCOL.md` 中 `> 已记录:FLAW-NNN` 引用行所在的最近上级标题给出,无则 `—`),末尾 `retired_prose: N`;内容相同时 MUST NOT 重写文件(mtime 不变)。

[REQ-OLP-EVO-P3-INDEX-RETIRE] `docs/OUTER_LOOP_PROTOCOL.md` 实战沉淀条目被 FLAW 取代时 MUST 只加一行 `> 已记录:FLAW-NNN`,MUST NOT 删除原文;现存 FLAW-001/002 在 PROTOCOL 中无对应散文,MUST NOT 为其伪造引用;迭代上限条目 MUST 加一行 `> 已登记:kind 候选 iteration_cap(见 knowledge/context/evolution/README.md)`,该行不计入 `retired_prose`。

[REQ-OLP-EVO-P3-STALL] `scripts/olp-evo-metrics.sh` MUST 新增 `--stall <review-board> [--stall-threshold <minutes>] [--now <ISO8601>]` 诊断:派单行 = 去掉行首 `> `、`**`、署名后匹配 `派单 <片号>` 或 `<片号> 派单`(两种既有词序),片号形如 `[0-9]+[a-z]?(?:-[0-9a-z]+)*`(覆盖 `43c-2`、`43-r1`、`48b-r`、`27c`;`不派单`、`立案并派单`、`立案+派单` 不匹配);ACK 行 = 以 `ACK(<片号> done|blocked|wontdo` 开头且片号最长匹配;派单时间取派单行的 `(YYYY-MM-DD HH:MM` 或 `[YYYY-MM-DDTHH:MM`,缺则取所属 `### N.` 标题中的 `(YYYY-MM-DD`(按当日 00:00),仍缺则视为未知;无 ACK 且(时间未知或距 `--now` 超阈值)者输出 `stall: <片号> <分钟|open>`(未知时间写 `open`),汇总 `stalls: N`;既有输出行不改,退出码恒 0。

[REQ-OLP-EVO-P3-FAKEVERIFIED] 指标 MUST 新增固定行 `fake_verified: N`(窗口内 trigger 为 `r2_record` 的卡数;外环未记档则为 0),`--json` 中为 `fake_verified` 键与 `stalls` 列表;stall 与 fake_verified MUST 与既有 `note:` 同为诊断,MUST NOT 作为通过/失败判据出现在任何脚本退出码或文案中。

[REQ-OLP-EVO-P3-KINDCANDIDATES] `knowledge/context/evolution/README.md` MUST 增加"kind 候选"一段,登记 `iteration_cap`(内环达到迭代上限停机)与 `patch_failed`(就地补丁失败留下编译错误)及其出处(2026-09-05 octos 活板 #48"48b 中断记录"),标注"发射点属 octos 侧,另立契约"。

[REQ-OLP-EVO-P3-NOWRITE] 节拍采集之外,本阶段脚本 MUST NOT 修改 `specs/**`(`specs/drafts/` 除外)、`.octos/OUTER_LOOP_REVIEW.md`、`src/**`;索引脚本只写 `INDEX.md`;指标脚本零写入。

## Scenarios

Scenario: 节拍采集在命中时落卡、推进基线且不退出
  Given 监视器以 --harvest 启动、活板基线 N 行、事件流为空
  When 活板追加一条 ACK(blocked) 行并经过两个监视间隔
  Then 进化黑板恰新增一张 ack_blocked 卡、stdout 恰一次 BOARD-SIGNAL、监视器仍在运行

Scenario: 节拍与手跑并发只落一张卡
  Given 监视器以 --harvest 与缺省状态根运行
  When 同一触发行出现后主审同时手跑 olp-evo-harvest.sh
  Then 进化黑板中该 identity 只出现一次

Scenario: 采集失败不中断监视
  Given --harvest 指向一个不存在的仓库根
  When 活板追加一条命中行
  Then stderr 含 harvest: failed 且 stdout 含 BOARD-SIGNAL 且监视器仍在运行

Scenario: FLAW 直出骨架可解析且不入库
  Given 模板五段齐全的 FLAW 夹具与真实 FLAW-001.md(无修复/预防段、无 req)
  When 分别运行 olp-evo-spec-skeleton.sh
  Then 两者 stdout 均可被 agent-spec parse 解析;前者场景数等于修复项数,后者含 TODO 占位、satisfies: [] 与未绑定需求;specs/ 根无新文件

Scenario: 索引幂等并识别退役散文
  Given 两条 FLAW 与 PROTOCOL 中一条带 `> 已记录:FLAW-001` 的条目
  When 连续两次运行 olp-evo-index.sh
  Then INDEX.md 含两行 FLAW、retired_prose: 1,第二次运行后 mtime 不变

Scenario: 停摆与伪 verified 诊断
  Given 活板含标题日期 2026-09-05 的 `派单 43c-2` 无 ACK、`派单 43-r1` 有 `ACK(43-r1 done)`、一行 `不派单`,进化黑板含一张 r2_record 卡
  When 以 --stall <板> --stall-threshold 30 --now 2026-09-05T00:45:00 运行指标脚本
  Then 输出 stall: 43c-2 45、stalls: 1、fake_verified: 1,不含 43-r1,退出码 0

## Dependencies

- REQ-OLP-EVO、REQ-OLP-EVO-RETRO、REQ-OLP-EVO-P2(卡片、harvest.lock、lib、指标脚本)

## Source Trace

- proposal:LEP-003(阶段计划:采集挂外环 watch 节拍、规格直出契约、散文沉淀退役)
- REQ-OLP-EVO-P2 Open Questions:"无 ACK 停摆"与"伪 verified"跨源指标留阶段 3
- 实测 2026-09-05:octos 内环 48b 在 50 次迭代上限停机、python 就地补丁失败留下编译错(octos 活板 #48"48b 中断记录")
- 实测 2026-09-04/05:FLAW-001/002 → octos 契约均为手写;两记录段名为 症状/责任步/根因/锚点/复发史/保护门/异议/结案,无 req 字段
- PR 复审 codex gpt-6 2026-09-05(#619):CI 未装 agent-spec、--out 保护依赖 cwd、安装版找不到采集脚本、片号子串与日期串带、字段映射偏差、索引测试共享临时目录、docs 提交破坏阶段 1 golden
- 对抗复审 codex 2026-09-05(`~/.octos/outer/evo/reviews/p3-codex.md`):one-shot 哨需显式兼容模式与 base 推进;真实 FLAW schema 无五段;派单两种词序(`27c 派单`)、ACK 状态词前缀;Allowed 取锚点段
- 对抗复审 grok 2026-09-05(`~/.octos/outer/evo/reviews/p3-grok.md`):watch 一击退出与常驻冲突、FLAW 段名别名、派单/ACK 定式抽样、stall 奖励空 ACK、PROTOCOL 无 FLAW-001/002 散文

## Open Questions

- `iteration_cap`/`patch_failed` 发射点是否并入 REQ-OLP-OBS 第三次修订(octos 侧)。
- LEP-004:canon 是否扩展 `flaw` 类型并进 lint(另立提案)。
- 空 ACK(写 `ACK(... done)` 但无 commit)的识别需活板与 git 对账,不在本需求内。
- 阶段 3 的运行验收"两个战役内主审无需手写契约与手跑采集"不由场景机械证明。

## Next

Single exit: compile this requirement into a task contract(`specs/task-req-olp-evo-p3.spec.md`)。
