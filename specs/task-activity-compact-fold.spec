spec: task
name: "活动流水紧凑折叠展示"
inherits: project
tags: [tui, transcript, activity, ux]
estimate: 1d
---

## 意图

k3 等 agentic 模型单回合产生几十条无参数工具调用(实测 2026-08-02:一回合
30+ 条裸 "⏺ Bash" 行),即便胶囊延续消掉了重复头部,流水仍逐行堆积刷屏。
本任务引入 Claude-Code 风格的密度控制:同名裸行合并为 `⏺ Bash ×N`;折叠
提示升级为醒目的 `◈ N more · Ctrl+O`;scrollback 的纯成功裸批次压成单行
摘要(`Bash ×4 · Read ×1`);harness 行增加实时动作计数。带命令参数的行、
失败行、运行中行永不合并——它们本身就是信息。

## 已定决策

- 合并只作用于"零信息重复行":连续、同显示名、无调用文本(invocation)、
  同失败/运行状态的条目合并为一行 `⏺ 名称 ×N`;任一条件不满足即逐行渲染。
  实现收敛到 `push_agent_task_children`,头部路径与胶囊延续路径共用。
- 折叠行文案与样式:`◈ %{count} more · Ctrl+O`(en)/
  `◈ 还有 %{count} 条 · Ctrl+O 展开`(zh),用 selected 高亮。显示上限沿用
  既有语义(折叠 3 行 / Ctrl+O 展开 12 行),但按**渲染行**而非条目数计——
  一段合并的 `×N` 只占一行预算。
- scrollback 批次密度由共享的子行渲染器(`push_agent_task_children`)的
  run-length 合并直接提供:胶囊延续批次里的同名裸成功行压成一行
  `⏺ Bash ×N`;混合名批次每个工具各占一行(`⏺ Bash ×4` + `⏺ Read`),
  不再引入第二套摘要机制。含失败/运行中/带命令的行保持逐行——不可变归档
  是审计轨迹。
- harness 行动作计数取 `flow_activity_items(app).len()`,仅在 >0 时渲染
  ` · N actions` 段;文案入 en/zh 两个语言文件。
- 不提供用户配置开关(合并的都是零信息行,YAGNI);Ctrl+O 展开与
  /activity 导航器保持完整明细入口不变。

## 边界

### Allowed Changes
- src/app.rs
- src/app/transcript_build.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不改变带调用文本行、失败行、运行中行的逐行渲染。
- 不修改 /activity 导航器与 Ctrl+O 展开语义。
- reserve==render 不变量不得破坏(行数变化须同步高度预算)。
- 不新增 crate 依赖与用户配置项。

## 排除范围

- 子代理输出进主转录(授权 UX 合约的排除项,同样适用)。
- inspector / ps 布局的同等处理。

## 完成条件

场景: 连续同名裸工具行合并为单行计数
  测试: consecutive_bare_tool_rows_merge_into_one_run_length_line
  假设 同回合连续 5 条无调用文本的同名工具条目
  当 渲染转录
  那么 输出恰好一行 "⏺ Bash" 且含 "×5"

场景: 带调用文本的行永不合并
  测试: rows_with_invocations_never_merge
  假设 两条同名但各带不同命令文本的工具条目
  当 渲染转录
  那么 两条命令文本均独立出现且无 "×2"

场景: 折叠提示醒目且带展开键提示
  测试: folded_activity_renders_prominent_more_row_with_expand_hint
  假设 活动条目数超过显示上限
  当 渲染转录
  那么 折叠行包含 "◈" 与 "more" 与 "Ctrl+O"

场景: 纯成功裸批次以单行摘要写入 scrollback
  测试: continuation_batch_of_bare_successes_flushes_as_one_digest_line
  假设 同回合胶囊延续批次含 4 条已成功的无调用文本条目
  当 增量 flush 该批次
  那么 仅写入一行且包含 "Bash ×4"

场景: 含失败的批次保持逐行明细
  测试: continuation_batch_with_a_failure_keeps_per_row_detail
  假设 胶囊延续批次含 2 条成功与 1 条失败条目
  当 增量 flush 该批次
  那么 写入行数不少于 3(逐行,不做摘要)

场景: harness 行显示实时动作计数
  测试: harness_row_summarizes_live_action_count
  假设 活动回合已有 4 条活动条目
  当 渲染 harness 状态行
  那么 输出包含 "4 actions" 或 "4 个动作"
