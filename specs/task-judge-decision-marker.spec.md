spec: task
name: "状态行 judge 标记:消费 judge/decision 通知(judge.decisions.v1),显示两方判定与置信度"
tags: [ui-protocol, status-row, judge, observability]
estimate: 0.5d
---

## Intent

octos 服务端将新增可选特性 `judge.decisions.v1`,每个 turn 推送一条 `judge/decision` 通知(关键词分层结果、Jev 判定、
置信度、是否一致、错误码、`has_cjk`)。本任务让 octoscode 协商该特性、消费通知,并在 harness 状态行的 actions 芯片之后
显示一个静音风格的 `judge` 标记,让用户一眼看出这一轮"两方判定是否一致、把握多少、有没有失败"。为不依赖 octos-core
的 rev 提升,采用仓库既有的 TUI 本地解码模式(如 `APPUI_METHOD_BACKGROUND_ACTIVITY`):本地常量 + 本地 params 结构,
在 vendored 解码器之前拦截。

## Decisions

<!-- lint-ack: verification-metadata-suggestion — 全部场景为进程内 apply_event / 渲染断言,无外部 I/O -->
<!-- lint-ack: decision-coverage — "旧服务器模式不广告"由 transport 的 token 列表测试守门;文档表格行由 tests/docs_drift.rs 机械守门 -->
<!-- lint-ack: observable-decision-coverage — "不改 Cargo.toml / Cargo.lock"是 Boundaries 约束,由外环隔离复验的 git diff 机械守门,无运行时可观察行为 -->

- 本地解码:`src/model.rs` 新增 `APPUI_METHOD_JUDGE_DECISION = "judge/decision"`、`APPUI_FEATURE_JUDGE_DECISIONS_V1 = "judge.decisions.v1"`
  与 `JudgeDecisionParams { session_id: String, turn_id: Option<String>, keyword_tier: String, judge_tier: Option<String>,
  judge_confidence: Option<f64>, judge_agrees: Option<bool>, judge_error: Option<String>, judge_model: Option<String>, has_cjk: bool }`
  (`serde(default)`);`src/transport.rs:3618` 附近的本地拦截链增加该方法,产生新的 `ClientEvent::JudgeDecision(JudgeDecisionParams)`(`src/client_event.rs`,与 `BackgroundActivity` 同形)。
- 特性广告:`appui_feature_tokens_for(old_server)`(`src/transport.rs:3257`)只在 **现代列表** 追加 `judge.decisions.v1`;
  `OCTOSCODE_OLD_SERVER_FEATURES=1` 的 baseline 列表不变。
- 状态:`SessionAutonomyState` 新增 `last_judge: Option<JudgeDecisionView>`(`turn_id`、`keyword_tier`、`judge_tier`、`confidence`、
  `agrees`、`error`),收到通知时按 session 写入;新 turn 开始(`turn/started`)时清空,turn 结束不清(保留到下一轮开始)。
- 渲染:`harness_status_lines_impl`(`src/app.rs:5276`)在 actions 芯片(`:5367`)之后、agents 芯片之前插入一个芯片,样式
  `palette.muted().bg(palette.surface)`,前缀 `" · "`。文案由 locale 键给出,三种形态:
  - 一致:`app.statusbar.judge_agree` → en `"judge ✓ {conf}"` / zh `"判定 ✓ {conf}"`,`{conf}` 为两位小数;
  - 不一致:`app.statusbar.judge_disagree` → en `"judge ✗ {judge}≠{keyword} {conf}"` / zh `"判定 ✗ {judge}≠{keyword} {conf}"`;
  - 失败:`app.statusbar.judge_error` → en `"judge ⚠ {error}"` / zh `"判定 ⚠ {error}"`。
  `last_judge` 为 `None` 时不渲染任何 judge 内容(现有状态行逐字不变)。
- 文档:`docs/ARCHITECTURE.md` 通知表增加 `judge/decision` 一行(`tests/docs_drift.rs` 守门)。
- 不改 `Cargo.toml` 的 octos-core rev、不改 `Cargo.lock`、不加依赖;将来 rev 提升后再把本地解码换成 vendored 变体(独立任务)。
- 测试命名沿用各文件既有蛇形风格。

## Boundaries

### Allowed Changes
- src/model.rs
- src/client_event.rs
- src/transport.rs
- src/store.rs
- src/app.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- docs/ARCHITECTURE.md
- specs/task-judge-decision-marker.spec.md

### Forbidden
- 不改 Cargo.toml / Cargo.lock(不提 octos-core rev,不加依赖)。
- 不改 baseline(旧服务器)特性列表。
- `last_judge` 为 `None` 时状态行渲染结果必须与本任务之前逐字相同。
- 不把判定结果用于任何客户端决策(纯显示)。

## Out of Scope

- 把本地解码切换为 vendored `UiNotification::JudgeDecision`(rev 提升后的独立任务)。
- 用户对判定"点错"的标注回传。
- 面板级的判定历史视图。

## Completion Criteria

### Rule: negotiate-and-decode — 现代模式广告特性,通知被本地解码
Scenario: 现代特性列表含 judge.decisions.v1 而 baseline 不含(critical)
  Tags: critical
  Test: appui_feature_tokens_include_judge_decisions_only_in_modern_mode
  Given `appui_feature_tokens_for(false)` 与 `appui_feature_tokens_for(true)`
  When 读取两个列表
  Then 前者含 `"judge.decisions.v1"`,后者不含,且后者与本任务之前的 baseline 逐项相同

Scenario: judge/decision 通知被本地解码为 ClientEvent
  Test: judge_decision_notification_decodes_locally
  Given 一条 method 为 `judge/decision` 的 JSON-RPC 通知,params 含 `keyword_tier: "cheap"`、`judge_tier: "strong"`、`judge_confidence: 0.87`、`judge_agrees: false`
  When 经 `notification_to_app_event` 处理
  Then 得到 `ClientEvent::JudgeDecision`,字段与 params 一致,不产生 `unknown_notification` 错误

Scenario: 缺省字段的通知也能解码
  Test: judge_decision_notification_decodes_with_absent_optional_fields
  Given params 只含 `session_id`、`keyword_tier`、`has_cjk`
  When 解码
  Then `judge_tier` / `judge_confidence` / `judge_agrees` / `judge_error` / `judge_model` 均为 `None`,不报错

### Rule: status-marker — 状态行显示判定标记
Scenario: 一致时显示 ✓ 与置信度(critical)
  Tags: critical
  Test: status_row_shows_judge_agree_marker_after_actions_chip
  Given 会话收到 `JudgeDecision`(`judge_agrees: true`,`judge_confidence: 0.99`)
  When 渲染 `harness_status_lines_static` 并展平为字符串
  Then 文本含 `"judge ✓ 0.99"`,且它出现在 actions 芯片文本之后

Scenario: 不一致时显示两方档位
  Test: status_row_shows_judge_disagree_marker_with_both_tiers
  Given `keyword_tier: "cheap"`,`judge_tier: "strong"`,`judge_agrees: false`,`judge_confidence: 0.87`
  When 渲染状态行
  Then 文本含 `"judge ✗ strong≠cheap 0.87"`

Scenario: 判定失败时显示错误码
  Test: status_row_shows_judge_error_marker
  Given `judge_error: "timeout"`,其余 judge 字段为 `None`
  When 渲染状态行
  Then 文本含 `"judge ⚠ timeout"`,不含 `✓` 或 `✗`

Scenario: 无判定时状态行逐字不变
  Test: status_row_unchanged_when_no_judge_decision
  Given 从未收到 `JudgeDecision` 的会话
  When 渲染状态行
  Then 文本不含 `"judge"`,且与本任务之前同一状态的渲染结果逐字相同

Scenario: 新 turn 开始时上一轮标记被清空
  Test: judge_marker_cleared_when_next_turn_starts
  Given 会话已有 `last_judge`
  When 施加 `turn/started` 事件
  Then `last_judge` 为 `None`,状态行不再含 `"judge"`
