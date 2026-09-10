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


## 后发 review 修复轮(2026-09-10 晚,PR#636 BH3GEI 三组尾项)

### 初始 RED(两个真实 bug,外部 636-probes 复现)

1. frozen 缺 verdicts → cross `KeyError: verdicts` traceback;缺
   repo → challenge --live-cargo `KeyError: repo` traceback(均非
   结构化 JSON)。
2. latest.accepted=1(truthy 非 bool)无 receipt 可过 cross 记录
   (cross 用 truthiness 而 receipt 验真用 is True,层间标准不一致);
   accepted=true 无 receipt 则被正确拒。

### 修复(生产 scripts/olp-review-evidence.py)

- frozen 必需键: verdicts/challenges 必须 dict、repo 必须非空
  (strip)字符串 → state-shape-invalid 结构化拒绝。
- `_record_shape_ok` 共用形状门: latest/history 同门(accepted
  在场必须 bool,显式 null 非法;executed 在场必须 dict;其
  artifacts 在场必须 dict)。
- accepted 读点统一 is True(cross 门/any_challenge_accepted/
  refutation substantiated/apply 前置)。
- legacy 兼容保持: executed 整段缺失 + receipt 原件完整 → 单侧
  重建;工件门在 legacy 路径同样生效。

### 既有行为补覆盖(R3,TDD 正向补测)

- 快照 int 0 vs bool False 类型敏感(PASS selector 基线 int 0 →
  改 bool false 拒;latest/history 双侧)。
- latest.executed="bad"/artifacts=[] → state-shape-invalid。
- tracked 门三面: 前置拒、**同次执行后门**(probe 测试运行中真实
  改写 tracked src/lib.rs,断言执行后 phase + drift 标记)、
  untracked 负控、非 git 工作树 CLI 早拒、git status 失败门函数
  真实 I/O 直测(python importlib 导入生产 adapter,断言 rc=1 +
  tracked-source-modified + 查询失败)。
- T2(d) 补删除 stdout 错误码断言(live-receipt-tampered)。

### 新测试(5)

- olp_review_frozen_missing_keys_structured_status_and_live_cargo
- olp_review_latest_accepted_must_be_bool
- olp_review_snapshot_core_false_vs_zero_rejected
- olp_review_latest_executed_shape_gates
- olp_review_adapter_tracked_modified_before_and_after

### 过程修正记录(如实)

candidate 阶段 R3b 测试源码最初只删 history.executed、latest 保留
完整记录——报告声称"latest 也删"与源码不符,由 ROOT 审查发现并做
唯一整合补丁(所有 history 记录 + latest 都移除 executed),反例
条件此后真实成立。最终版以 ROOT source manifest 哈希为准。

### 计数(本轮实测)

- evidence suite: **62 pass / 0 fail / 1 ignored**。
- spec 场景: 44 + 5 = 49(agent-spec parse 实测)。
- fmt/clippy --all-targets -D warnings EXIT0(定点阶段);全量
  all-targets 结果见 final-validation.*(本文件同步后执行)。
