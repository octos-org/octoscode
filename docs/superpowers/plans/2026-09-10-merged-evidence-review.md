# Merged evidence review repair — 2026-09-10

基线 d03e7f2,来源 merged-review-audit-20260910(#632 尾项)。
全部 fixed/wontdo 与真实测试绑定见 specs 与下方记录。

## Fixed(真实 RED→GREEN)

1. state 形状校验全入口结构化(state-shape-invalid): 顶层/frozen/
   reviews 嵌套/challenges/history/latest/verdicts/cross;frozen 必带
   reviews(glm+k3)+head。36 形状探针全结构化(含 latest-number、
   accepted-no-receipt)。
   - olp_review_malformed_state_all_entries_structured
2. accepted live 记录全生命周期 fail-closed(live-receipt-tampered):
   无条件 receipt 路径+sha(apply_live_verdict 是唯一 writer);receipt
  _kind 必须 cargo-test-execution;核心字段类型敏感一致(False≠0);
   latest 与 history 同验;工件删除/改写/缺声明拒绝;legacy 仅快照内
   缺字段由 hash 验证过的完整 receipt 重建。ROOT live-integrity
   18/18。
   - olp_review_live_receipt_lifecycle_tamper_rejected
3. adapter 工件唯一(mkstemp)+ tracked-source-modified 前后同门
   (untracked probe/artifact 允许;git status 失败≠clean)。
   - olp_review_adapter_same_selector_two_runs_no_overwrite
   - olp_review_k3_cargo_adapter_lib_entry_full_artifacts(独立
     tiny lib fixture,主仓开发树被正确拒绝)
4. 初审 verdict 白名单 {approve, request-changes, comment}(合约
   初审初始态;cross accept/refute 与执行后状态非法)。
   - olp_review_first_review_verdict_whitelist
5. monitor closed 跨仓 parity: 可信 lifetime+ident_ok 保留身份,
   execution 恒 closed;foreign/malformed → null;无 lifetime/旧失败
   语义不变。
   - olp_review_monitor_closed_keeps_identity_on_trusted_lifetime
6. status 持锁一致快照(review_lock 单次,与 classify ExitStack 协议
   不嵌套)。

## Wontdo(技术理由)

- "无 dirty 才执行"整体拒绝: 排除合法测试注入面;已改为精确
  tracked-vs-untracked 门(tracked 修改拒,untracked probe/artifact
  允许,与 probe 源 hash 锚互补)。证据: ROOT live-integrity
  untracked-probe 正例 + dirty-tracked-source-rejected 反例均过。
- 自造 accepted-history schema 兼容: apply_live_verdict 唯一 writer
  (ROOT-2 裁决),不存在合法无 receipt 的 accepted 记录。

## 计数(实测)

- agent-spec parse: 44 scenarios;lint min-score 0.7 EXIT0。
- cargo test: evidence 57 pass/0 fail/1 ignored;monitor 31 pass/
  0 fail;fmt/clippy --all-targets -D warnings/cargo test
  --all-targets 全 EXIT0(45 suites)。
