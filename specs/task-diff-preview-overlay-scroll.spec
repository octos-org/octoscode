spec: task
name: "差异预览全屏 overlay:独立状态位、键盘所有权与精确滚动"
inherits: project
tags: [tui, diff, overlay, modal, ux]
estimate: 1d
---

## 意图

Ctrl+O 展开的差异预览此前渲染在有界的内联 live-tail 视口里,超长 diff 被
静默截断且没有自己的滚动面;首版 overlay(commit da0bf15)把展开态挂在全局
`expanded_tool_outputs` 上,带来一串正确性问题:开着全局展开的用户被自动弹
出的预览甩进全屏模态、Esc 顺带摧毁全局展开偏好、overlay 渲染在最顶层却在按
键路由中排最后(用户对着被遮住的授权卡盲打)、Ctrl+U 清空被遮住的 composer
草稿。本任务给 overlay 一个独立的 `diff_preview.expanded` 状态位,统一渲染
/按键/滚轮/键盘所有权四处的判定,并让滚动数学精确(截断长行,不换行)。

## 已定决策

- 展开态是 `DiffPreviewPaneState.expanded` 独立位;`overlay_active()`
  (active && expanded && has_renderable_diff)是渲染、按键路由、滚轮路由与
  `modal_owns_keyboard` 共用的唯一判定。
- Ctrl+O 语义分层:预览可渲染且无其他模态持有键盘时切换 overlay;overlay
  已展开时收起;其余情况保持原有全局 `toggle_tool_output_expansion`。
- overlay 是最低层 detail 模态:渲染在其他 detail 模态之下,按键路由排在
  它们之后;授权/提问对话框变为可见时强制收起 overlay(store 各到达点 +
  session 切换晋升点)。
- overlay 行以 `clip_line_spans` 硬截断到面板宽度、Paragraph 不启用 wrap,
  因此逻辑行数即显示行数;`diff_preview_overlay_max_scroll` 按同一构建函数
  计算上限,事件循环在每次向上滚动后钳制,防止顶部死区。
- overlay 复用 inline 的 `push_diff_hunk_body`:渲染全部 hunk 体、选中 hunk
  用与 inline 相同的 `›` 标记,`v`(side-by-side)在 overlay 内可见生效,
  窄于 `DIFF_SIDE_BY_SIDE_MIN_WIDTH` 时与 inline 同规则回落 unified。
- `c` 暂存选中 hunk 后收起 overlay(无可暂存上下文时保持展开)。
- 底部提示与展开/收起状态文案走 i18n(`app.hint.diff_preview_modal`、
  `status.diff_overlay_expanded/collapsed`,en/zh 双语)。

## 边界

### Allowed Changes
- src/model.rs
- src/store.rs
- src/event_loop.rs
- src/app.rs
- src/app/render.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不改变收起态 inline 差异预览的渲染与暂存语义。
- 不改变全局 `expanded_tool_outputs` 对 transcript 工具输出的语义。
- 不改 server 协议与差异预览载荷。
- 不新增 crate 依赖。

## 排除范围

- overlay 内水平滚动(与 side-by-side v1 相同:超宽行截断)。
- 其余 detail 模态(task-output/artifact 等)的换行滚动数学修复。
- 差异预览的获取/刷新协议流程。

## 完成条件

<!-- lint-ack: decision-coverage — R1 断言独立 expanded 位与 overlay_active 判定(测试中直接读取该字段);i18n 文案键由 R7/R8 渲染断言间接覆盖,文案本身非行为语义 -->
<!-- lint-ack: error-path — R2(越界钳制)、R3(按键泄漏防护)、R4(空预览回退)、R5(模态遮挡防护)均为异常/防御路径,linter 关键词启发未识别 -->


场景: R1 展开、滚动并收起而不关闭预览
  测试: diff_preview_overlay_scrolls_and_collapses
  假设 一个可渲染的差异预览已打开且终端为 100x30
  当 用户按 Ctrl+O 后用 j/k/PgUp/PgDn/End 与滚轮滚动,再按 Esc
  那么 overlay 以独立 expanded 位展开且不改动全局 expanded_tool_outputs
  并且 滚动遵循 from-bottom 约定
  并且 Esc 收起 overlay 而 diff_preview.active 保持为真

场景: R2 顶部越界被钳制且无死区
  测试: diff_preview_overlay_scrolls_and_collapses
  假设 overlay 已展开且内容行数超过可视高度
  当 用户连续按 PgUp 越过内容顶部
  那么 scroll 停在 diff_preview_overlay_max_scroll
  并且 紧接的一次 PgDown 立即使视图移动 8 行

场景: R3 overlay 持有键盘时 Ctrl+U 不清空隐藏的 composer
  测试: diff_overlay_owns_keyboard_and_ctrl_o_falls_back_without_renderable_diff
  假设 overlay 已展开且 composer 中有草稿
  当 用户按 Ctrl+U
  那么 composer 草稿原样保留

场景: R4 空预览时 Ctrl+O 回退为全局展开而不弹出空模态
  测试: diff_overlay_owns_keyboard_and_ctrl_o_falls_back_without_renderable_diff
  假设 差异预览 active 但没有可渲染的 hunk
  当 用户按 Ctrl+O
  那么 expanded 保持为假
  并且 expanded_tool_outputs 翻转为真

场景: R5 决策对话框变为可见时 overlay 被收起
  测试: showing_a_pending_decision_collapses_the_diff_overlay
  假设 overlay 已展开且存在一个隐藏的待决授权
  当 该授权被显示(show_pending_approval)
  那么 expanded 翻转为假
  并且 diff_preview.active 保持为真

场景: R6 c 暂存选中 hunk 后 overlay 收起且 composer 可见
  测试: diff_overlay_stage_key_collapses_so_the_composer_is_visible
  假设 overlay 已展开且存在选中 hunk
  当 用户按 c
  那么 expanded 翻转为假
  并且 composer 内容包含被暂存的 hunk 上下文
  并且 焦点位于 composer

场景: R7 overlay 渲染全部 hunk、选中标记并截断超宽行
  测试: diff_overlay_modal_marks_selection_renders_all_hunks_and_never_wraps
  假设 预览含两个 hunk 且第二个被选中,其中一行宽 300 列
  当 渲染 overlay
  那么 选中 hunk 头带 › 标记而非选中 hunk 头带 ├ 标记
  并且 非选中 hunk 的正文也被渲染
  并且 超宽行在模态区域内只占一行且行尾被截断

场景: R8 v 切换在 overlay 内可见生效
  测试: diff_overlay_modal_honors_side_by_side_view_mode
  假设 预览含成对的删除/新增行且 side_by_side 已开启
  当 渲染 overlay
  那么 同一行同时出现旧侧与新侧内容并以 │ 分隔
