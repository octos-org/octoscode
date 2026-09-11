> **[HISTORICAL 2026-09-10]** 本 plan 为 2026-09-09 首轮实现记录(部分
> 计数/验收进度为当时状态,如"23 场景/23 tests"已过期——现为 44 场景
> / 57+31 tests,以 specs 与 docs/OLP_REVIEW_EVIDENCE.md 当前值为准)。
> 2026-09-10 合并后 review 修复见
> docs/superpowers/plans/2026-09-10-merged-evidence-review.md。

# 计划 — OctoLoop 行为证据互审流程 + Herdr pane 监控入口 (goal_01)

日期: 2026-09-09(Phase 5 修订版)
worktree: /Users/zhangalex/.local/tmp/octoloop-evolution-20260909/octoscode-behavior-review
HEAD 基线: 0a174d95ddec2b123adb3498432e29eb13affb81
契约: specs/task-evo-review-evidence.spec.md (lint 100%, --min-score 0.7 通过, 23 场景)
证据源: /Users/zhangalex/Work/Projects/FW/octoscode/.octos/reviews/pr-627-630-20260909/
Phase 5 裁决纪要: .octos/cross-design-dispositions.md(GLM/k3/外层逐条采纳/驳回)

## 阶段分步

1. [x] Phase 0 勘察: git status clean、brief/Active 读取、证据源与 MANIFEST 结构确认
2. [x] Phase 1 原生 goal: goal_create → goal_01;progress.md 落盘
3. [x] Phase 2 契约: specs/task-evo-review-evidence.spec.md 写入,agent-spec parse+lint 通过(100%,非零 scenarios)
4. [x] Phase 3 本计划落盘
5. [x] Phase 4 双只读 peer 独立设计审查(GLM primary 继承 / k3 model=strong,worktree=false,显式 goal_id):
   - 各自读 spec+plan+证据源,产出 .octos/design-glm.md / .octos/design-k3.md ✅ 已收齐
   - 收齐前不互读 ✅
6. [x] Phase 5 定点互审: 互读对方设计审查 → 逐条采纳/反驳/待验证 → 修 spec/plan → 重新 lint
   - 裁决落盘 .octos/cross-design-dispositions.md;外层 outer-feedback.md 全部 13 条定点改判吸收
   - spec 16→23 场景(pending peer/native 信号否决/伪造 cargo 日志/imported not-replayed/
     errored cross/flip 回边/approve happy path/分层 fallback/unknown-lifetime/哈希 ACK)
   - 删 estimate=2d;Allowed Changes 增 plans、scripts 通配、olp_watch_board.rs 定点修复条款
 7. [~] Phase 6 实现(master 单写 evidence 侧;监控侧由 monitor-impl-k3 peer
     独占并行 — 外层 2026-09-09 调度授权更新):
    a. [x] fixtures/review-evidence/**: 从前轮真实证据复制 8 反例 + 元数据;
       复制前后源目录 SHA256 复核,fixtures 自带 SHA256 清单 + provenance 头
       (FIXTURES-SHA256.json, 60+ 文件, commit 7ceccad)
    b. [x] scripts/olp-review-evidence.py: freeze/challenge/cross/status 子命令
       - freeze: 两份初审齐全+可解析+outcome 有效(外部信号: runtime-evidence active_thread、
         native result-N.md 否决报告自报)+**外部权威 fail-closed**(无 native/runtime
         终止收据 → peer-authority-missing;收据 slug 不符 → peer-authority-mismatch)
       - challenge: **内容校验为门+执行为本**: 结构锚必要非充分;执行验证实际执行
         argv 捕获退出码+输出绑定测试名(/usr/bin/false 等无关命令拒绝);
         imported → not-replayed;kind 仅 hint
       - cross: frozen+challenged 前置;逐 claim 覆盖;errored/pending 拒绝;
         反驳成立 → challenge-refuted 回边;cross 报告同样要求外部权威
       - status: 无证据绑定的两模型一致 → pending-behavioral-evidence;
         冻结后初审被删除/被改写 → first-review-tampered(fail-closed)
    c. [~] scripts/olp-review-monitor.py + tests/olp_review_monitor.rs:
       monitor-impl-k3 peer(strong) 独占并行实现中(外层授权),master 不互写;
       交付后 master 统一验证(all-targets/fmt/clippy + monitor target)
    d. [x] tests/olp_review_evidence.rs: 23 tests 全部以子进程真实调用脚本;
       外层反例回归 4 条按"先 RED 后修"(outer-probe-red.log 2 failed →
       outer-probe-green.log 全绿,SHA256 落盘 .octos/red-proof/);
       真实 Store harness 集成验收(临时 clone + include! 反例源码,真实
       cargo 执行 8 探针,28.9s,commit ee668e0);测试补写顺序如实记录
    e. [ ] docs/OLP_REVIEW_EVIDENCE.md: 使用手册 + 判词状态机两层词表 + PR 级聚合规则
    f. [x] housekeeping commit(a4455cf): tests/olp_watch_board.rs clippy 修复
8. [ ] Phase 7 双 peer 独立 review(同一 peer 名续轮)→ independent-glm/k3.md → 互读 → cross-glm/k3.md
9. [ ] Phase 8 必跑验证:
   - cargo test --all-targets -- --test-threads=8 (CARGO_BUILD_JOBS=4)
   - cargo clippy --all-targets -- -D warnings(housekeeping 后应 0 基线错误;豁免清单文件化防吞新告警)
   - cargo fmt --check
   - agent-spec lifecycle 相关 spec
   - grep 核对 23 个过滤名全部落地为 #[test]
   - 更新 progress.md + 提交(仅 git add 本任务文件)
10. [ ] Phase 9 外层复验采认(外层 Codex 复核,非本内环自报;内环完成后留 commit+报告+ACK(done),
    不无限轮询)

## 验收硬门

- 8 反例经完整 freeze→challenge→cross 流程后: #627/#628/#630 → blocked, #629 → residual
  (派生自外层 MANIFEST outer_recommendation + evidence,非硬编码)
- 负向: 伪造(含结构齐全的假 cargo 日志)/篡改/errored+pending peer/旧轮次/异HEAD/
  非行为证据/imported 未复验 → 全部被拒绝且报错码明确、错误输出可解析
- 监控: 四区块 + 复合 runtime 身份 + 分层 last-outcome + fallback 交叉核对(无 lifetime→unknown)
  + inplace ACK 哈希观测
- RED 阶段证据保留: 测试先失败(断言级)输出存 .octos/red-proof/ + SHA256+HEAD 清单
- 没有运行的检查必须标"未验证"
