spec: task
name: "Assistant 双投影通道去重"
inherits: project
tags: [tui, transcript, streaming, projection, dedup]
estimate: 0.25d
---

## 意图

混合版本服务端可能同时投递 `projection/envelope` v1 streaming 与延迟的
`projection/envelope.v2` persisted 行。两者是同一 turn 的两份投影，不是两个回答；
客户端不得让迟到的 persisted 行改写已经刷入原生 scrollback 的正文并在提交时再次重放。

## 已定决策

- `projection.envelope.v2` 的 assistant delta 在匹配的 persisted 行到达前是
  provisional；只有连续 finalized segment 前缀可以刷入不可逆的原生 scrollback。
- 已协商 `projection.envelope.v2` 时，先到的兼容 v1 正文同样保持 provisional；
  延迟的 canonical v2 persisted 行可以接管并重建 live reply。
- 未协商 v2 的真正 v1-only 服务端不会有 canonical 接管，继续按既有规则渐进刷入
  scrollback。
- v2 最先产生正文时仍独占该 turn；后到的 v1 文本视为重复投影并忽略。
- lane 选择在 turn terminal、turn switch 与 stale hydrate 清理时释放。
- committed assistant 行保留 turn/thread identity。live coverage 必须匹配 session 和 turn
  （允许 v2 wire/local id 映射）；无 thread 的 legacy 行仅接受唯一 user-anchor 推导的精确行号。
  共用 user anchor 的 continuation 不得仅凭相同正文前缀互相去重。
- 选择状态跨 in-flight snapshot replay 保留，避免重连途中重新选择另一通道。
- capabilities 尚未协商（`None`）时，v1 正文同样保持 provisional、不刷入原生 scrollback
  （重连后恢复的续跑可能先于 capabilities 响应流入）；capabilities 到达后，legacy 立即放行，
  v2 继续保持直到 canonical 行到达。
- 以 error terminal（非用户中断）结束的 turn 会被记为 errored；其归档的 live coverage
  不得仅凭"提交正文以已刷前缀开头"去重后续 turn 的提交回复——它自己的提交行是失败卡片，
  coverage 才得以存活。anchor 精确匹配的去重仍适用。

## 边界

### Allowed Changes
- src/model.rs
- src/app.rs
- src/app/transcript_build.rs
- src/store.rs
- src/viewport.rs
- src/app/tests.rs
- README.md
- specs/**

### Forbidden
- 不按正文内容做模糊去重。
- 不改变 v1/v2 wire payload、terminal 与 hydrate 语义；仅允许 finalized
  canonical 前缀推进原生 scrollback watermark。
- 不改变 tool/activity 投影。

## 完成条件

场景: 共用 user anchor 的两个 continuation 有相同回复前缀
  测试: continuation_coverage_never_consumes_another_turn_with_the_same_prefix
  假设 coverage 属于 C，而先提交的 D 与 C 使用相同正文前缀和 user anchor
  当 D 与 C 的 committed 行刷入 scrollback
  那么 D 前缀完整保留，只有 C 自身已刷入的前缀可以被去重

场景: 无 thread 的 legacy 行共用 anchor 时不猜测 turn
  测试: legacy_reply_with_shared_anchor_retains_ambiguous_prefix
  假设 legacy assistant 行没有 thread，且两个 turn 共享同一 user anchor
  当 live coverage 与其正文前缀相同
  那么 正文完整保留，不把前缀当作已刷内容跳过

场景: v1 stream 与延迟 v2 persisted 交错时报告只显示一次
  测试: interleaved_v1_stream_and_v2_persisted_answer_render_once
  假设 已协商 v2 且 v1 先流入长回答前缀
  当 同 turn 的 v2 preamble/full-answer persisted 行在 v1 尾部和 terminal 前到达
  那么 provisional v1 前缀尚未刷入原生 scrollback
  并且 canonical v2 接管并提交 preamble 与完整回答
  并且 报告标题在 scrollback 中只出现一次

场景: 工具前完整回答等待 canonical persisted 后只显示一次
  测试: provisional_v2_tool_answer_waits_for_canonical_persist_before_scrollback
  假设 v2 delta 在工具调用前流出一个完整回答块
  当 工具后 final response 重复该块且 server 送达 canonical persisted 行
  那么 persisted 前该回答块不会进入原生 scrollback
  并且 terminal 后 canonical 前言与回答块各只出现一次

场景: v1-only 服务端保持渐进 scrollback
  测试: glued_completed_segment_flushes_via_boundary_so_live_tail_holds_only_current_segment
  假设 当前 lane 是 v1 且 capabilities 已协商（legacy 集合，不含 projection.envelope.v2）
  当 工具边界完成一个没有空行的正文 segment
  那么 已完成 segment 立即刷入 scrollback
  并且 当前 segment 继续留在 live tail

场景: stdio 重连 hydrate 相同 canonical 回答不重放原生 scrollback
  测试: reconnect_hydrate_of_identical_canonical_answer_keeps_native_scrollback_exactly_once
  假设 报告已由 v1 provisional + v2 persisted 路径恰好提交并显示一次
  当 后端重启、session/opened 后 hydrate 返回相同的 canonical 消息前缀
  那么 客户端保留已有 scrollback watermark，不请求 full transcript re-flush
  并且 报告标题在终端原生 scrollback 中仍恰好出现一次

场景: v2 最先产生正文时忽略后到的 v1 文本
  测试: v2_first_assistant_lane_ignores_parallel_v1_text
  假设 v2 persisted 已建立该 turn 的 live reply
  当 同 turn 的 v1 assistant delta 随后到达
  那么 live reply 保持 canonical v2 正文且不追加 v1 副本

场景: capabilities 未知时 v1 正文先保持 provisional
  测试: v1_bytes_streamed_before_capabilities_are_held_until_the_canonical_takeover_commits_once
  假设 重连后恢复的续跑先以 v1 delta 流入且 capabilities 尚未到达
  当 Capabilities(v2) 随后到达，再由 canonical v2 persisted 行接管并 terminal
  那么 capabilities 未知期间没有 v1 正文进入原生 scrollback
  并且 报告标题与前言在 scrollback 中各只出现一次

场景: 出错 turn 的 coverage 不去重后续同前缀回复
  测试: errored_turn_coverage_does_not_dedup_a_later_same_prefix_reply
  假设 T1 流出前缀 P 后以失败卡片结束（其 coverage 保留）
  当 T2 提交以 P 开头的新回复
  那么 T2 的回复完整刷入 scrollback，前缀不被当作 T1 已刷内容跳过

场景: error terminal 记录 errored turn，用户中断不记录
  测试: turn_error_marks_the_turn_errored_but_an_interrupt_does_not
  假设 turn 收到 code 不是 interrupted 的 turn/error
  当 store 应用该 terminal
  那么 该 turn 记为 errored
  并且 interrupted terminal 不记录（其提交行就是已流出的正文）
