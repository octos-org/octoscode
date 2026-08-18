spec: task
name: "Loops 菜单每 loop 一行 + 动作子菜单"
inherits: project
tags: [tui, autonomy, loop, menu, ux]
estimate: 0.25d
---

## 意图

实测(2026-08-09,用户截图):上游 loops 菜单按"每个 loop × 每个动作 = 一行"
渲染,一个 loop 出三行、每行都以相同的 id 和状态开头,动作词藏在行中间——
用户直接误读为"三个同名 loop"。三个 loop 就是九行,无法扫读。

改为两级:清单层**每个 loop 恰好一行**(区分信息前置:图标、id、状态、节奏、
提示词摘要),回车进入该 loop 的**动作子菜单**(pause/resume/fire-now/delete)。

## 已定决策

- 清单行动作为 `LocalAction::OpenLoopActions(loop_id)`:记录目标到
  `AppState.loop_actions_target`,压栈打开 `MENU_LOOP_ACTIONS`。Esc 沿既有
  菜单栈语义逐层返回。
- 子菜单头部为**只读详情行**(完整状态·节奏 + 提示词摘要,`non_selectable`),
  光标自动跳过;动作行沿用原 `RunSlashCommand("/loop <verb> <id>")` 派发路径,
  保持能力门控与关栈语义不变。
- 动词按状态裁剪:active → pause/fire-now/delete;paused → resume/fire-now/
  delete;其他 → delete。paused 也可 fire-now(与服务端 control_loop 一致)。
- 目标 loop 在子菜单打开期间消失(他处删除/过期)时,子菜单显示明确的
  "已不存在" 说明而非报错动词;`loop_actions_target` 不主动清理——它仅在
  `MENU_LOOP_ACTIONS` 在栈上时被读取,下次打开前必被覆写。
- 空清单行为不变(仍为可用菜单 + 创建提示行,见 task-loop-list-transcript)。
- (2026-08-09 用户追加)清单行支持**→ 快速切换**:`MenuItem.right_action`
  为通用二级动作槽,loops 行填 `QuickLoopToggle(loop_id)`——active → 派发
  pause,paused → 派发 resume,其他状态无操作;**菜单保持打开**,变更结果
  经镜像刷新回填行内。无 `right_action` 的行/菜单按下 → 仍为空操作,
  不影响其他菜单。高频操作(暂停/恢复)一键完成;子菜单只承担低频动作。
- (2026-08-09 补齐已批准图示)时钟经 `MenuAppSnapshot.now_ms` 显式注入
  (store 在快照构建时取当前时间;测试传固定值,构建保持确定性)。清单行对
  active 且有 `next_run_at_ms` 的 loop 追加 `· next <时长>`;子菜单详情行
  追加 `· next <时长>` 与 `· <时长> left`(取 `expires_at_ms`)。`now_ms`
  缺失或时刻已过期时**省略该段,不虚构**。菜单不逐帧重绘,倒计时按事件
  刷新粒度更新即可——秒级精度由常驻的自主指示行负责。

## 边界

### Allowed Changes
- src/menu/providers.rs
- src/menu/types.rs
- src/menu/registry.rs
- src/model.rs
- src/store.rs
- src/event_loop.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不改变 `/loop <verb>` 斜杠命令的解析与派发协议。
- 不改变 loops 菜单的打开时机(用户显式 `/loop list`,见 task-loop-list-transcript)。
- 不新增 crate 依赖。

## 排除范围

- 跨会话全局菜单(菜单数据源仍为活跃会话镜像)。

## 完成条件

场景: 清单层每个 loop 恰好一行
  测试: loops_menu_one_row_per_loop_opens_action_submenu
  假设 存在一个 active 与一个 paused loop
  当 构建 loops 菜单
  那么 恰好有两行 loop 行
  并且 每行动作为打开对应 loop 的动作子菜单
  并且 行文本包含 id 与状态

场景: 子菜单按目标 loop 提供动词
  测试: loop_actions_menu_offers_verbs_for_the_target
  假设 目标为一个 active loop
  当 构建动作子菜单
  那么 标题包含该 loop id
  并且 pause 行派发针对该 id 的斜杠命令
  并且 至少两行只读详情行

场景: paused loop 提供 resume 而非 pause
  测试: loop_actions_menu_for_paused_loop_offers_resume
  假设 目标为一个 paused loop
  当 构建动作子菜单
  那么 存在 resume 行
  并且 不存在 pause 行

场景: 目标消失时子菜单明确说明
  测试: loop_actions_menu_says_gone_when_target_vanished
  假设 目标 id 不在当前 loop 镜像中
  当 构建动作子菜单
  那么 返回不可用说明而非动词行

场景: 清单行显示下次触发倒计时
  测试: loops_menu_row_shows_next_run_countdown
  假设 一个 active loop 的 next_run_at_ms 晚于注入的 now_ms 十四分钟
  当 构建 loops 菜单
  那么 该行包含 next 与 14m 字样

场景: 时钟缺失时不虚构倒计时
  测试: loops_menu_row_omits_next_without_clock
  假设 快照未注入 now_ms
  当 构建 loops 菜单
  那么 行内不含 next 字样

场景: 子菜单详情行包含剩余寿命
  测试: loop_actions_detail_shows_next_and_expiry
  假设 目标 loop 有 next_run_at_ms 与 expires_at_ms 且晚于 now_ms
  当 构建动作子菜单
  那么 详情行包含 next 字样
  并且 包含 left 字样

场景: 右方向键快速切换 active loop 为暂停
  测试: right_arrow_on_active_loop_dispatches_pause
  假设 loops 菜单打开且选中一个 active loop 行
  当 按下右方向键
  那么 派发针对该 loop 的 pause 命令
  并且 菜单保持打开

场景: 右方向键快速恢复 paused loop
  测试: right_arrow_on_paused_loop_dispatches_resume
  假设 loops 菜单打开且选中一个 paused loop 行
  当 按下右方向键
  那么 派发针对该 loop 的 resume 命令

场景: 无二级动作的行按右键无操作
  测试: right_arrow_without_right_action_is_noop
  假设 菜单选中行未声明 right_action
  当 按下右方向键
  那么 不派发任何命令
  并且 菜单保持打开

场景: 激活清单行打开子菜单并记录目标
  测试: activating_loop_row_opens_action_submenu
  假设 loops 菜单中选中某个 loop 行
  当 激活该行
  那么 `loop_actions_target` 为该 loop id
  并且 活跃菜单为 `MENU_LOOP_ACTIONS`
