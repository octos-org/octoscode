# 进度 — behavior-review 任务 (goal_01 / runtime /private/tmp/oe9-review)

worktree: /Users/zhangalex/.local/tmp/octoloop-evolution-20260909/octoscode-behavior-review
branch: feat/evo-behavior-review @ 2edb1ca(Phase 5 合约 commit)
native goal: goal_01 (goal_create 2026-09-09, 无预算上限由 runtime 分配 100M)
基线证据: /Users/zhangalex/Work/Projects/FW/octoscode/.octos/reviews/pr-627-630-20260909/
  - MANIFEST.json: PR627/628/630 head/base、8 反例、2383 passed/1 failed/4 ignored (GNU 全量轮)
  - 期望分类: #627/#628/#630 request-changes(阻塞), #629 conditional-approve-scope-and-specs(residual)
  - 4 个原 prNNN-glm peer outcome=errored，不得计入有效互审（回归数据集的一部分）

## 阶段状态

- [x] Phase 0: 读 brief/Active、git 状态、证据源勘察
- [x] Phase 1: goal_01 原生 goal 建立
- [x] Phase 2: specs/task-evo-review-evidence.spec.md v1 + agent-spec parse/lint --min-score 0.7 (16 场景)
- [x] Phase 3: docs/superpowers/plans/2026-09-09-review-evidence.md
- [x] Phase 4: GLM/k3 双只读 peer 独立设计审查 → .octos/design-glm.md / design-k3.md（收齐前未互读）
- [x] Phase 5: 定点互审 → .octos/cross-design-dispositions.md（含外层 outer-feedback.md 13 条定点改判）
      → 合约修订 16→23 场景，lint 100% 通过 → commit 2edb1ca
- [ ] Phase 6: master 单写实现（真实 RED→GREEN harness）
      - 6a fixtures 复制+SHA256 清单 ✓ (FIXTURES-SHA256.json, 60+ files)
      - 6b evidence CLI ✓ scripts/olp-review-evidence.py (781 行)
      - 6c evidence tests ✓ 22 passed（含 4 条外层实测反例回归:
            outer-probe-red.log 2 failed RED → outer-probe-green.log 22 GREEN）
      - 6d 外层反例修复: peer-authority-missing/mismatch(跨runtime冒充)、
            verify_no_tamper 文件缺失 fail-closed、/usr/bin/false 拒绝 ✓
      - 6e spec 增补 fail-closed 条款 + lint 通过 ✓
      - 6f 监控两文件 = monitor-impl-k3 peer 并行进行中（不与 master 互写）
      - 6g housekeeping commit: olp_watch_board.rs clippy ✓ (a4455cf)
- [ ] Phase 7: 双 peer 独立 review + 交叉互审（independent-*/cross-*.md）
- [ ] Phase 8: 必跑验证 + 外层复验采认

## Phase 5 关键裁决（详见 cross-design-dispositions.md）

- **并行调度（外层授权更新）**: monitor-impl-k3 peer(strong) 独占写
  scripts/olp-review-monitor.py + tests/olp_review_monitor.rs;
  master 只写 evidence CLI/tests/spec/docs，互不覆盖；peer 交付后
  master 统一完整验证(all-targets/fmt/clippy)再 commit

- 行为证据 = 执行为本：结构锚(测试名/panicked/test result:FAILED)仅必要非充分，
  生产入口实际执行 argv/脚本+退出码+输出绑定；imported → not-replayed；
  结构齐全但不可执行的伪造日志 → evidence-not-executed（外层第11条改判，
  推翻两 peer 的 regex 冒充方案）
- 状态机补全：challenge-refuted 回边、pending(暂)/unverified(终)/not-replayed 分层、
  challenge-flip vs blocked-on-evidence 判别式（HEAD 执行复现失败→blocked；
  遗留路径→flip/residual）、PR 级=f(claim×severity×introduced_vs_existing)、
  期望分类来自外层 MANIFEST（禁 PR 编号硬编码）
- 监控分层：last-outcome=最近终止结果（旧completed+新running 不混写为
  running/inconsistent）；有 authority 优先 lifetime.json，无 → unknown；
  禁 next_seq vs turn 数值比较（量纲不同，外层第13条改判）
- 新场景: pending peer / native 信号否决报告自报 / errored cross / approve happy
  path / 伪造日志 / imported / 分层 fallback / unknown-lifetime / ACK 哈希比对
- Allowed Changes: scripts/olp-review-*.py 通配 + plans + .octos 显式枚举收窄 +
  tests/olp_watch_board.rs 定点 clippy 非行为修复条款；删 estimate=2d
- 外层基线: cargo test --all-targets 2369 passed/0 failed (.octos/baseline-summary.json)；
  clippy 基线唯一错误 = tests/olp_watch_board.rs:411 overly_complex_bool_expr，
  已授权单独 housekeeping commit 修复

## 纪律快照

- 禁止 Codex/Claude 子进程；peers 只读（worktree=false）
- cargo: CARGO_BUILD_JOBS=4, --test-threads=8,
  CARGO_TARGET_DIR=/Users/zhangalex/Work/Projects/FW/octoscode/target, df≥50GB ✓ (265G free)
- 编译槽: 外层基线已跑完，当前无并发 cargo；本任务编译前再查
- 没有运行的检查必须标"未验证"
- 下一步: 等 monitor-impl-k3 交付监控两文件 → master 全量验证
  (all-targets/fmt/clippy + monitor test target) → 统一 commit → Phase 7
  双模型独立复审+交叉互审（新只读上下文）
