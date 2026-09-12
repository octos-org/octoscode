spec: task
name: "互审流程以冻结初审与独立行为证据为验收(octoscode)"
tags: [olp, review, evidence, herdr, octoscode]
depends: [task-req-olp-exec-peer]
satisfies: [REQ-OLP-REVIEW-EVIDENCE]
---

## 意图

前轮四 PR(#627–#630)审查暴露:初审与交叉互审之间缺少冻结点,行为证据(实际运行
的渲染/事件序列)缺席时双模型一致仍可能误判;errored/pending peer 的历史产物被
当作有效审查;监控入口把不同 runtime 的 goal_01 混为同一身份。本任务在 octoscode
建立可复用的行为证据互审流程与 Herdr pane 监控入口:冻结两份独立初审,接纳实际
生产渲染/事件序列的挑战证据,再收逐项采纳/反驳/待验证交叉报告;全程可审计。

## 已定决策

- 新入口为 `scripts/olp-review-evidence.py`(证据/判词管理,JSON 输出,可拆分
  `scripts/olp-review-*.py` 共享 helper)与 `scripts/olp-review-monitor.py`
  (Herdr pane 可读监控);不复用、不改写 `scripts/olp-cross-check-diagnostic.sh`
  (那是 Windows 编译诊断,语义不同)。
- 冻结(freeze):两份初审文件收齐且各自可解析后,记录 SHA256、git HEAD、
  base commit、runtime/session/goal/peer/turn 标识到 manifest;此后初审文件
  再被写入即视为篡改,拒绝采信并显式报错。manifest 写入与冻结状态迁移必须
  原子(并发/失败时不得出现半写 manifest 或先标冻结后写失败的不一致);
  错误输出亦为可解析 JSON(含 error code)。
- 挑战(challenge):行为证据必须是实际调用生产代码的渲染/事件序列 harness
  输出(8 个前轮真实反例为回归数据集);文档字符串、文件存在性、硬编码
  expected 不构成行为证据。每个判词必须绑定证据或显式 `unverified`。
- **行为证据判定为内容优先、执行为本**(外层定点改判):`kind` 声明仅为 hint。
  必要条件(全部可编程校验): (a) 测试过滤名行 (b) `panicked at` 行
  (c) `test result: FAILED. ... N failed` 汇总 (d) 绑定测试名可在 harness
  源解析。以上仅证"形似",**不构成执行证明**。执行验证必须由生产入口
  `olp-review-evidence.py` 经固定生产 cargo adapter(`--live-cargo`,唯一
  live 执行路径)实时执行,捕获真实退出码与 stdout/stderr,并把 HEAD/
  运行目录/测试名/源码与输出摘要绑定写入 manifest;证据携带的自报
  argv/脚本一律 `executor-not-trusted`,不被执行。
  外层 imported 日志(未独立执行)→ 判词 `not-replayed`,不得自动 accepted。
  纯构造 cargo 样式日志的伪造证据必须被拒绝。
- 交叉(cross):仅当两份初审已冻结且挑战证据已接纳后才允许;交叉报告必须对
  对方每条判词逐项标注采纳/反驳/待验证,反驳需引用挑战证据或代码行号。
- 有效性拒绝:peer outcome 为 pending/errored 的产物不得作为有效初审或交叉;
  最高编号 result-N.md 的 outcome 需与 turns.txt/runtime-evidence 交叉核对,
  不 solely 采信 result.md 或 mtime;异 HEAD、旧轮次(turn 序号非当前)一律拒绝。
  有效性校验必须比对**报告外部信号**(frontmatter 声称 completed 而 native
  result-N.md 为 errored → 拒绝;旧完成报告 + 当前新 running → 拒绝),
  不得只检查报告自己声明的 outcome。
  **外部权威 fail-closed(外层实测反例补入)**: freeze/cross 采信一份初审或
  交叉报告时,该 slug 必须有至少一种外部终止权威 — native `<root>/<slug>/`
  最高编号 result-N.md(slug 匹配,含 turns.txt)或 runtime-evidence 终止快照
  (该 slug 无 active_thread);两者皆缺 → `peer-authority-missing` 拒绝,
  仅自报 outcome:completed 不得冻结。native 收据 slug 与报告 slug 不符
  (跨 runtime 同名目录冒充)→ `peer-authority-mismatch` 拒绝。冻结后初审
  文件被删除与被改写同罪 → `first-review-tampered`(verify fail-closed,
  不得因文件缺失静默通过)。
- **判词状态机(两层词表)**:
  - claim 级: `approve` / `flipped` / `blocked-on-evidence` /
    `pending-behavioral-evidence`(暂态,流程进行中等行为证据)/
    `unverified`(终态,证据被拒或收口仍缺)/ `not-replayed`(imported
    证据未独立复验)/ `challenge-refuted`(cross 以新行为证据或行号证明
    反例不成立,判词回 approve 并标记)。
  - PR 级: `clean` / `residual` / `blocked`。
  - challenge-flip vs blocked-on-evidence 判别式: 反例在 PR HEAD 上**真实执行
    复现失败**(executed probe FAIL) → blocked-on-evidence;证据仅证明
    理论可达/遗留路径(legacy path) → flip,PR 级聚合为 residual。
    legacy path 的判定依据 = 同一 probe 在 BASE 与 HEAD 的**独立执行对照**
    (两者均 FAIL 且失败形态同构)或外层 ASSESSMENT 显式标注,
    禁止仅凭证据描述措辞推断。
  - PR 级 = f(claim 级 × severity × introduced_vs_existing) 聚合;分类输入
    来自外层独立评估(MANIFEST outer_recommendation / ASSESSMENT),工具只做
    聚合派生,禁止按 PR 编号硬编码。
    introduced_vs_existing 的参考证据锚: 同 probe BASE/HEAD 双执行 receipt
    (如 ../outer-verification-slot.json + base/head 双日志)为 existing/residual
    的实证;缺双执行证据时该维度=unassessed,聚合结果保守降级不得标 clean。
- 监控分离 lifecycle / current turn / last outcome / 交付物验收四个区块;
  runtime 身份必须含 runtime 路径+session 等复合标识,不能仅显示 goal_01。
  last-outcome 指最近**终止**结果: 旧 completed + 新 running 时保留
  last-outcome=completed、current-turn=running、accepted=false(分层不混写)。
  旧运行版本 fallback: 有 authority 时优先 native lifetime.json
  (turn_id/generation),无时 unknown + 原生 result-N.md/turns.txt 与
  ui-protocol thread 交叉核对;**禁止用 ui-protocol next_seq(流事件序号)与
  result-N turn(轮次计数)比较判定旧轮次**(量纲不同);archive 缺当前
  thread 时显示 unknown,不得推测。turns.txt 只有已结束轮次行,运行中证据
  来自 thread 完成态/lifetime。
- 对旧运行版本按 native result-N.md + turns.txt 与当前 ui-protocol thread 交叉
  核对,不能 solely result.md 或 mtime;对 inplace ACK 的内容变更以**哈希比对**
  观测(非仅行数),保持原官方 olp-watch-board.sh 正哨与 events/supervisor
  负哨接线,不擅改其既有协议。
- `scripts/olp-watch-board.sh` 既有协议(正哨)与 events/supervisor 负哨接线
  保持不变,新监控只读其输出,不擅改其协议。
- fixtures 从前轮真实证据复制: 复制前后对源目录做 SHA256 复核,fixtures 自带
  SHA256 清单与 provenance 头,注明与源 MANIFEST 的派生/脱敏关系。
- RED→GREEN 硬门: RED 输出必须含**断言级失败**(期望错误码的 assert 消息),
  编译错误/错误测试命中不算 RED;red-proof 落盘时同写 SHA256+HEAD 清单。

## 边界

### Allowed Changes(octoscode 仓库)
- scripts/olp-review-*.py(含共享 helper)
- tests/olp_review_evidence.rs
- tests/olp_review_models.py
- tests/olp_review_monitor.rs
- tests/olp_watch_board.rs(仅限 clippy 基线 overly_complex_bool_expr 的
  非行为改变验证修复: `while !read(...).map(...).unwrap_or(false) || true` 改为
  语义相同的 loop/deadline 结构,保留完整等待时长与原断言,单独 housekeeping
  commit,不碰脚本协议,不扩到其他历史 dirty 文件)
- fixtures/review-evidence/**
- specs/task-evo-review-evidence.spec.md
- docs/OLP_REVIEW_EVIDENCE.md
- docs/superpowers/plans/2026-09-09-review-evidence.md
- docs/superpowers/plans/2026-09-12-model-review-followup.md
- .octos/{progress.md, red-proof/, design-*.md, independent-*.md, cross-*.md,
  outer-feedback.md, loop.md, OUTER_LOOP_REVIEW.md, implementation-brief.md,
  baseline-*.log, baseline-summary.json, active-profile, octosfix/}

### Forbidden
- 不修改 scripts/olp-cross-check-diagnostic.sh 与 scripts/olp-watch-board.sh
- 不修改 src/、Cargo.toml 生产代码(本任务是流程工具,不进二进制)
- 不删除既有失败断言或把 8 failed 说成 8 passed
- 不伪造 runtime/goal/peer id;没有运行的检查必须标"未验证"
- 不按 PR 编号硬编码 residual/blocked 分类
- 不修改 .octos/reviews/(前轮证据源,只读)

## 排除范围

- 修复 #627/#628/#629/#630 四个 PR 本身的代码缺陷
- olp-watch-board.sh 协议演进(events/supervisor 负哨接线仅消费)
- 外层 Codex 侧工具(仅按其 MANIFEST 格式对接)

## 完成条件

Rule: review-freeze — 冻结完整性与身份可审计

场景: 冻结两份独立初审(critical)
  测试:
    包: octoscode
    过滤: olp_review_freeze_requires_both_first_reviews
  假设 仅有 glm 一份初审可解析,k3 初审缺失
  当 对该评审执行 freeze
  那么 报错初审未齐且 manifest 未写入

场景: 冻结记录可审计标识(critical)
  测试:
    包: octoscode
    过滤: olp_review_freeze_records_identity
  假设 两份初审均收齐且可解析
  当 freeze 成功
  那么 manifest 含两文件 SHA256、git HEAD、base、runtime/session/goal/peer/turn

场景: 冻结后篡改初审被拒绝(critical)
  测试:
    包: octoscode
    过滤: olp_review_freeze_rejects_tampered_first_review
  假设 两份初审已冻结
  当 其中一份文件内容被追加一行再执行 challenge 接纳
  那么 报篡改错误且该初审不被采信

场景: pending peer 产物不冒充有效初审(critical)
  测试:
    包: octoscode
    过滤: olp_review_rejects_pending_peer_as_first_review
  假设 一份初审 frontmatter outcome=completed 但 runtime-evidence 显示
    active_thread 非空(该 peer 仍有活跃轮次)
  当 尝试 freeze
  那么 报错 peer-outcome-invalid 且初审未齐

场景: errored peer 的 native 信号否决报告声明(critical)
  测试:
    包: octoscode
    过滤: olp_review_rejects_claimed_completed_but_native_errored
  假设 一份初审 frontmatter 声称 outcome=completed 而 native result-N.md 为
    errored(或旧完成报告+当前新 running)
  当 尝试 freeze
  那么 报错 peer-outcome-invalid(外部信号优先于报告自报)

场景: 异 HEAD 与旧轮次分别拒绝(critical)
  测试:
    包: octoscode
    过滤: olp_review_rejects_head_mismatch_and_stale_turn
  假设 manifest HEAD 为 H1;fixture A 初审头部 HEAD=H2,fixture B 初审 turn 低于
    该 peer 自己 turns.txt 最大行号
  当 分别对 A/B 尝试 freeze
  那么 A 报 head-mismatch,B 报 stale-turn(两错误码独立触发,per-peer 比较)

Rule: review-challenge — 挑战证据必须来自实际生产行为

场景: 行为证据驱动改判(critical)
  测试:
    包: octoscode
    过滤: olp_review_challenge_evidence_flips_verdict
  假设 初审 A 判词 X 为 approve 且已冻结
  当 以真实生产 harness 反例(渲染/事件序列断言失败输出,生产入口实际执行并
    捕获退出码与输出)作为挑战证据接入
  那么 判词 X 状态变为 flipped 或 blocked-on-evidence(按判别式: PR HEAD 执行
    复现失败→blocked-on-evidence;遗留路径→flipped),不得保持 approve

场景: 文档性证据不被接纳(critical)
  测试:
    包: octoscode
    过滤: olp_review_challenge_rejects_non_behavioral_evidence
  假设 挑战文件声称 PR#627 有问题但内容仅为字符串常量断言
  当 尝试将其接纳为行为证据
  那么 报错 evidence-not-behavioral 且判词状态保持原判但标 unverified

场景: 伪造 cargo 样式日志被拒绝(critical)
  测试:
    包: octoscode
    过滤: olp_review_challenge_rejects_fabricated_cargo_log
  假设 一份手工构造的文件含测试名行、panicked at 行、test result: FAILED 汇总
    (结构锚齐全)但不携带可执行的 argv/脚本或执行记录
  当 尝试接纳为行为证据
  那么 报错 evidence-not-executed(结构形似不构成执行证明)

场景: imported 证据不自动 accepted(critical)
  测试:
    包: octoscode
    过滤: olp_review_imported_evidence_not_replayed
  假设 证据为外层 imported 日志(未由本生产入口独立执行)
  当 尝试接纳并汇总状态
  那么 判词状态为 not-replayed 而非 accepted/flipped

Rule: review-cross — 交叉互审门槛与逐项裁决

场景: 交叉互审需初审齐全且证据已接纳(critical)
  测试:
    包: octoscode
    过滤: olp_review_cross_requires_frozen_and_challenged
  假设 初审已冻结但挑战证据尚未接纳
  当 尝试收录 cross 报告
  那么 报错 challenge-pending 且 cross 未被采信

场景: 交叉报告逐项裁决(critical)
  测试:
    包: octoscode
    过滤: olp_review_cross_requires_per_claim_verdicts
  假设 初审 A 含 3 条判词且挑战证据已接纳
  当 收录一份只回应其中 2 条的 cross 报告
  那么 报错 missing-claim-coverage 且该 cross 未被采信

场景: errored cross 报告不冒充有效(critical)
  测试:
    包: octoscode
    过滤: olp_review_rejects_errored_cross_report
  假设 一份 cross 报告 peer outcome=errored(或 pending)
  当 尝试收录该 cross 报告
  那么 报错 peer-outcome-invalid 且 cross 未被采信

场景: cross 反驳 challenge-flip 的回边(critical)
  测试:
    包: octoscode
    过滤: olp_review_cross_refutation_reverts_flip
  假设 判词 X 已因挑战证据翻转为 flipped 且 cross 阶段开始
  当 对方以新行为证据或代码行号反驳该反例成立(引用具体证据/行号)
  那么 判词 X 转为 challenge-refuted(回到 approve 语义并保留 refuted 标记与
    原 flip 证据的 refuted-by-cross 标记)

场景: 有充分行为证据时 approve 成立(critical)
  层级: 集成(真实子进程调用生产入口)
  测试:
    包: octoscode
    过滤: olp_review_k3_full_happy_path_accepted
  假设 两份首审一致 approve 且绑定经生产入口真实执行通过的行为证据,交叉
    覆盖齐全
  当 走完 freeze→challenge→cross 完整流程
  那么 判词 accepted(happy path 不强迫发现缺陷/改判)

场景: 两模型一致不能替代行为实验(critical)
  测试:
    包: octoscode
    过滤: olp_review_agreement_without_evidence_stays_pending
  假设 glm 与 k3 都判 PR#629 approve 且无行为证据绑定
  当 汇总最终状态
  那么 该判词显示 pending-behavioral-evidence 而非 approved

Rule: review-monitor — Herdr 监控:区块分离与身份防混

场景: 真实 native wire thread 识别(completed=false 精确绑定才 running)(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_real_wire_thread_recognition
  假设 真实 native thread wire 文件(v/session_id/thread_id/next_seq/completed,
    无 active 键;session_id 带 NUL+~cwd-hash),peer 目录 originator 与
    goal_01 均与当前 master 视角一致
  当 渲染监控(同一 wire master session/cwd 视角)
  那么 本 session 未完流(completed=false)→ running;completed=true → 非
    running;别的 session 同 slug 形状未完流 → unknown;同 channel 但 cwd
    哈希不同 → 不绑定;next_seq 是流事件序号永不与 result turn 比较

场景: peer originator 指向别的 wire master 不得 running(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_foreign_originator_thread_not_running
  假设 同 profile/channel/cwd 的 peer 目录,originator 文件为
    `octosfix:local:tui#other-master`(真实 native originator 通常无 cwd
    后缀,如 `octosfix:local:tui#coding`),goal 文件与当前 goal 一致,
    无 lifetime.json,且存在 NUL+~cwd-hash 与当前 master session 一致的
    未完 native thread
  当 以当前 master/session/goal 视角渲染监控
  那么 该 peer execution=unknown(reason=peer-originator-mismatch),
    不得凭该 thread 晋升 running —— thread fallback 必须先与 peer 自身
    身份文件绑定,而非只拒绝 lifetime 存在时的矛盾

场景: peer goal 文件与当前视角 goal 不一致不得 running(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_peer_goal_file_mismatch_not_running
  假设 同 master 同 cwd 的 peer 目录,originator 与当前 wire master 同源,
    但 goal 文件为 goal_99 而当前视角为 goal_01,无 lifetime.json,
    存在精确绑定当前 master session 的未完 native thread
  当 以 goal_01 视角渲染监控
  那么 该 peer execution=unknown(reason=peer-goal-mismatch),不得凭该
    thread 晋升 running

场景: 缺失身份文件时未完 thread 不得证明归属(信息不足≠归属证明)(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_missing_originator_thread_not_running
    过滤: olp_review_monitor_v3_missing_goal_thread_not_running
  假设 peer 目录与当前 profile/goal_01 视角下存在精确 cwd 绑定的未完
    native thread,但(反例1)有 goal 文件而缺 originator 文件,或
    (反例2)originator 正确而缺 goal 文件
  当 以明确 master session+goal_01 视角渲染监控
  那么 两反例均 execution=unknown(fail-closed,reason=
    peer-originator-unproven-missing / peer-goal-unproven-missing);
    身份不再只做"存在即冲突拒绝",thread 晋升 running 前必须对当前视角
    已知身份字段给出实际 native 匹配证据;无 lifetime 时 native thread
    不是可绕过身份的权威。当前视角身份字段未知时不强求对应文件
    (无 goal context 的通用视角沿用旧语义)

场景: 快照自报 outcome/outcome_source 不作终止权威(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_unverified_self_claim_outcome_not_terminal
  假设 runtime-evidence.json 只含 {slug, outcome: completed,
    outcome_source: unverified-self-claim},无 native result-N.md+turns.txt
  当 渲染监控
  那么 该 peer last-outcome state=unknown,自报值与来源仅作
    snapshot-outcome-untrusted:* notes 保留(可观测但不采信);
    不得信自报来源字符串而显示 completed。有 native 证据时四值终止
    分层(completed/errored/interrupted/rate_limited)维持不变,
    review-freeze 只认 completed 的准入语义在另一层不受影响

场景: 身份逐键一致的精确未完 native thread 仍识别 running(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_matching_originator_goal_thread_still_running
  假设 peer 目录 originator 文件与当前 wire master 同源(无 cwd 后缀的
    `octosfix:local:tui#coding`),goal 文件与当前 goal 一致,无 lifetime.json,
    存在 session_id 带 NUL+~cwd-hash 且与 master channel/cwd 一致的
    未完 native thread
  当 渲染监控
  那么 该 peer execution=running(active-thread)—— 收紧身份绑定不得
    误伤合法正向绑定(真实 native 形状参考 ../native-observation-shapes.json:
    thread session_id 含真实 NUL 字节,originator 文件无 cwd 后缀)

场景: runtime-evidence 快照身份防混(v3 组)(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_bare_snapshot_not_identity_authority
    过滤: olp_review_monitor_v3_foreign_snapshot_rejected
    过滤: olp_review_monitor_v3_complete_snapshot_wrong_goal_profile_unknown
    过滤: olp_review_monitor_v3_snapshot_identity_incomplete
  假设 runtime-evidence 快照分别为: 只有 slug+active_thread 的裸快照;
    foreign runtime/session 全字段;完整身份但 goal/profile 与当前视角
    不符;带身份键但逐键不完整
  当 渲染监控(当前 runtime/profile/goal 视角)
  那么 均不采信为 running/身份权威(unknown 或拒绝);复合身份=runtime 路径
    +session+goal+profile 逐键一致才绑定,active_thread 字符串本身不是
    身份,也不是 liveness 广播

场景: negative_events 归属过滤(v3 组)(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_negative_events_no_profile_no_fallback
    过滤: olp_review_monitor_v3_negative_events_profile_goal_filtered
  假设 事件文件分属当前 profile 与 foreign profile/其他 goal/其他 session
  当 读取负向事件
  那么 当前 profile 缺失时不得扫描 foreign profile 兜底;事件按当前
    profile+goal(+ originator session,事件携带时)过滤,他人 goal 的
    blocked 不得计入当前视角

场景: 终止绑定与四值分层(v3 组)(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_v3_terminal_binding_strict
    过滤: olp_review_monitor_v3_terminal_outcome_four_value_display
    过滤: olp_review_monitor_v3_pending_followup_keeps_previous_turn
  假设 native result-N.md+turns.txt 存在/缺失/伪造各形态
  当 计算终止态
  那么 只有 turns.txt 与 result-N 一致的 completed 才可信;completed/
    errored/interrupted/rate_limited 四值如实分层;伪造值(fabricated 等)
    与非终止态不折叠;pending 后续轮不覆盖已完成轮的 last-outcome

场景: 有效 lifetime 跨 goal 不得晋升(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_valid_lifetime_cross_goal_not_promoted
  假设 peer 目录 lifetime.json 形状完全有效(originator/master/registry/
    phase/result_digest 一致)但 goal 文件属于其他 review(goal_99),
    当前视角 goal_01(review-state.json 提供)
  当 渲染监控
  那么 跨 goal 的有效 lifetime 显示 unknown(fail-closed,不晋升
    running/idle);同 goal 正向对照 running/idle 保持 —— lifetime
    晋升必须同时通过 peer 自身身份文件归属,不只 lifetime 形状校验

场景: lifetime 严格校验(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_lifetime_strict_validation
  假设 peers 目录含 lifetime.json(逐字段校验:originator==master、
    phase/turn/generation 形状)与伪造形态
  当 解析 lifetime
  那么 Pending→queued、Running→running、Failed→failed 按 phase 映射;
    originator 不符/lifetime 记录 master 非当前 session → 不采信,
    不因存在而晋升

场景: 监控分离四区块且 runtime 身份完整(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_sections_and_runtime_identity
  假设 评审目录含 manifest 与事件序列
  当 渲染监控
  那么 输出含 lifecycle/current-turn/last-outcome/deliverables 四区块且
    runtime 身份为路径+session 复合标识而非裸 goal_01

场景: 监控 fallback 分层显示而非混写(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_fallback_cross_checks_result_and_thread
  假设 一个旧运行版本 runtime 目录,result-4.md 为最近终止轮 completed,
    runtime-evidence 显示 active_thread 非空(当前有新轮次在运行)
  当 渲染监控
  那么 last-outcome=completed(最近终止结果)、current-turn=running、
    accepted=false(分层不混写为 running/inconsistent),旧轮次产物不当作当前
    状态;禁止以 ui-protocol next_seq 与 result turn 数值比较判旧

场景: 旧运行版本无 lifetime 时 unknown(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_unknown_without_lifetime
  假设 legacy 运行版本(如 9c157101)无 lifetime.json 且 archive 缺当前 thread
  当 渲染监控
  那么 对应字段显示 unknown(不得推测为 completed/running),并用原生
    result-N.md+turns.txt 与 ui-protocol thread 完成态交叉核对

场景: inplace ACK 哈希变更可观测(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_observes_inplace_ack_edit
  假设 板文件已含 ACK 行且监控已建立基线
  当 该行内容被原地修改为 blocked
  那么 监控输出该变更事件(哈希比对,非仅行数)而不是静默

场景: helper 边界 — turns 重复后新 valid 轮不得恢复可信(critical)
  测试:
    包: octoscode
    过滤: olp_review_helper_turns_duplicate_then_valid_stays_untrusted
  假设 turns.txt 为 `1 completed/1 errored/2 completed`(同轮 1 冲突后
    跟新 valid 轮 2;纯结构校验数据 fixture,非产品执行证据)
  当 parse_turns_txt 解析
  那么 rows 必须为空(任意重复 → 全体不可信,后续 valid 行不得恢复)
    且 notes 含 turns-duplicate-row;正常多轮路径不受影响

场景: helper 边界 — BASE/HEAD 日志字节完全相同时双侧绑定(critical)
  测试:
    包: octoscode
    过滤: olp_review_helper_identical_slot_logs_both_bound
  假设 两份有效 FAILED 日志(含 qualified 测试失败锚)字节完全相同、
    SHA 相等,slot 其余字段(probe/pr_head/base/probe_sha256/exits)齐备
  当 _verify_slot_evidence 核验
  那么 返回有效 slot 且 base_log/head_log 双侧绑定 —— 两次真实独立
    执行输出相同字节是合法形态,不得因 hash 相等误拒;其余冻结门
    (receipt/HEAD/probe/BASE/selector 锚)不放宽

Rule: review-regression — 前轮八反例回归数据集

场景: state 形状校验全入口结构化(critical)
  测试:
    包: octoscode
    过滤: olp_review_malformed_state_all_entries_structured
  假设 合法 JSON 但结构损坏的 state(顶层非对象/frozen 缺 reviews/head/
    嵌套 reviews/challenges/history/latest/verdicts/cross 形状非法)
  当 任一 init/freeze/challenge/cross/status/classify 入口读取
  那么 一致结构化 JSON 错误(state-shape-invalid / classify-context-*),
    不出现 KeyError/TypeError traceback,不改写损坏状态;classify 共用
    状态形状门并映射为 classify-context-invalid

场景: status 只读锁与写事务协调(critical)
  测试:
    包: octoscode
    过滤: olp_review_status_read_only_lock_contract
  假设 review 由 init 创建,或查询路径不存在/空目录/已有 state 但缺锁
  当 status 查询,包括目录 555 + 锁 444 和 writer 持有排他锁
  那么 不创建目录或锁;缺目录/state 返回 not-initialized;已有锁以
    只读 fd 获取共享 flock,等待 writer 释放后才读 state;只读目录和锁
    可正常查询;缺锁返回 review-lock-unavailable 结构化错误且不退化
    无锁读取;所有路径保持 state 字节不变

场景: accepted live 记录全生命周期 fail-closed(critical)
  测试:
    包: octoscode
    过滤: olp_review_live_receipt_lifecycle_tamper_rejected
  假设 apply_live_verdict 产出的 accepted history/latest 记录(唯一
    writer,恒带 cargo receipt);篡改 receipt 字节/删除/篡改或删除
    stdout 工件/删 receipt 键/快照核心字段与 receipt 不一致(类型敏感,
    False≠0)/快照缺字段由 hash 验证过的 receipt 补齐
  当 verify_no_tamper 校验(status/cross/challenge/classify 共用)
  那么 篡改/删除/不一致/缺失 → live-receipt-tampered 拒;真实旧快照
    缺字段从完整 receipt 重建后通过;legacy 正例保持可验

场景: adapter --lib 入口全工件绑定与 tracked 干净门(critical)
  测试:
    包: octoscode
    过滤: olp_review_k3_cargo_adapter_lib_entry_full_artifacts
  假设 独立 tiny lib fixture(src/lib.rs 真实单测,git 提交后 tracked
    干净)经生产 adapter --lib 执行
  当 adapter 运行
  那么 observed=pass、test_target=--lib、selector_qualified 经
    cargo --list 全限定定位、源文件 SHA256 绑定、stdout/stderr 工件
    mkstemp 唯一;tracked 源修改(执行前后同门)拒绝且 git status 查询
    失败不得当作 clean;被测 fixture repo 须 tracked == HEAD

场景: adapter 同 selector 多轮工件不覆盖(critical)
  测试:
    包: octoscode
    过滤: olp_review_adapter_same_selector_two_runs_no_overwrite
  假设 同一 selector 于同一 artifact-dir 顺序两次真实执行
  当 adapter 落盘 stdout/stderr
  Then 每次独立唯一文件(mkstemp),旧
    receipt 的 hash 与文件仍可验证;tracked 源修改(执行前后同门)拒绝,
    untracked 探针/工件接受,git status 失败拒绝
  那么 两套工件都在场且 hash 与各自 receipt 一致

场景: 初审 claim verdict 白名单(critical)
  测试:
    包: octoscode
    过滤: olp_review_first_review_verdict_whitelist
  假设 初审报告 claims 块 verdict 非 {approve, request-changes,
    comment}(当前合约初审初始态词表;空/非 str/cross 词表 accept/
    refute/执行后状态均非法)
  当 freeze 解析初审
  那么 claims-verdict-invalid 结构化拒绝;合法词表通过,未污染 state
    的 freeze 全流程正常

场景: monitor closed 身份保留跨仓 parity(critical)
  测试:
    包: octoscode
    过滤: olp_review_monitor_closed_keeps_identity_on_trusted_lifetime
  假设 peer closed 标记 + 可信 lifetime(originator/goal 归属一致)
  当 渲染监控
  那么 execution 恒 closed,身份字段(task_id/generation/turn/
    master_session_id)保留;foreign goal → closed 且身份 null;
    malformed lifetime → 身份 null;无 lifetime/旧失败语义不变

场景: classify 受信来源门(伪造链拒绝)(critical)
  测试:
    包: octoscode
    过滤: olp_review_classify_forged_chain_and_provenance_gates
    过滤: olp_review_classify_pr_aggregation_production_cli
    过滤: olp_review_regression_dataset_classifies_four_prs
  假设 (a) 无 repo/无 review state/无真实执行的完全自洽伪造链
    (虚构 HEAD/BASE/receipt/slot/日志);(b) receipt 未在受信上下文
    注册(外部副本/篡改);(c) BASE/HEAD 双执行证明任一侧未注册或
    registered BASE 为 PASS/异 selector
  当 classify 聚合(显式 --review-dir 受信上下文,可重复)
  那么 一致性不等于来源: 伪造链/未注册/篡改/BASE 侧不匹配 →
    blocked/unassessed 或结构化错误,绝不 residual;真实注册的
    BASE FAIL + HEAD FAIL 同 probe 双执行 → residual/existing。
    受信注册表 = 各上下文 challenges[*].history 中 accepted live
    记录(receipt canonical 路径 + sha256 + live-evidence 目录包含),
    只由 --live-cargo 写入,classify 只读;历史未注册数据 →
    blocked/unassessed 降级,不改写为新鲜执行。

场景: pass selector 一致性门(critical)
  测试:
    包: octoscode
    过滤: olp_review_classify_pass_selector_consistency_gates
  假设 某 PR 一个 selector 为真实 existing fail(receipt 完整),另一
    selector 自报 pass 但 summary adapter_exit=7(或 cargo_exit=101/null
    与 pass 声明矛盾;receipt 本身合法)
  当 classify 聚合
  那么 坏 pass selector 归 harness 证据错误,该 PR 必须 blocked/
    unassessed —— 不得因另一 selector 有 existing 双执行证据而判
    residual;合法 pass(adapter 0/cargo 0)不落 harness 故障

场景: 八反例回归分类(critical)
  测试:
    包: octoscode
    过滤: olp_review_regression_dataset_classifies_four_prs
  假设 使用前轮真实 8 反例数据集( fixtures/review-evidence/ ),期望分类来自
    MANIFEST outer_recommendation 外层独立写入;
    PR #629 的 residual 判定绑定真实双执行证据: BASE(0a174d95)与 HEAD(9bcf4099)
    同 probe 独立执行均 FAIL(receipt ../outer-verification-slot.json,
    日志 ../outer-629-base.log / ../outer-629-head.log,复现源 ../residual-629-repro.rs)
  当 跑完整 freeze→challenge→cross 流程(正常小测试用真实最小 Cargo fixture
    经生产 adapter 同路径执行;真实 Store 八探针为外层 prepared-replay-slots
    单次集成重放绑定收据,不要求每个小测试递归编译全仓)
  那么 #627/#628/#630 判为 blocked(阻塞问题成立),#629 判为 residual 且其
    判词引用上述双执行 receipt/日志文件;分类引擎从 MANIFEST 外层字段与
    双执行对照派生,不含 PR 编号条件分支;每个判词绑定证据文件与反例名;
    harness 成功与产品测试失败状态分别记录
    (旧代码 101+8failed 是缺陷证据,不是回归通过)

场景: watch-board 正哨保持不变(critical)
  层级: 集成(与 olp-watch-board.sh 并行真实运行)
  测试:
    包: octoscode
    过滤: olp_review_monitor_keeps_watch_board_contract
  假设 监控与 olp-watch-board.sh 同时运行于同一板
  当 板追加新 ACK 行
  那么 watch-board 仍按其既有退出码/输出协议命中,监控额外报告 change 事件,
    双方互不干扰

场景: JSON 与人读双输出(critical)
  测试:
    包: octoscode
    过滤: olp_review_evidence_json_and_human_output
  假设 一份完整走完流程的评审目录
  当 分别以 --format json 与默认 human 运行两个入口
  那么 JSON 可被 json.loads 解析且含同判词集合,human 含四区块可读视图;
    错误输出亦为可解析 JSON


场景: frozen state 必需键全入口结构化校验(critical)
  测试:
    包: octoscode
    过滤: olp_review_frozen_missing_keys_structured_status_and_live_cargo
  假设 合法 freeze 后的 review-state.json 被注入缺键/坏类型
    (verdicts/challenges 缺失或非 dict;repo 缺失/空串/纯空格/非 str)
  当 status/cross 分别读取全部上述坏状态,或 challenge --live-cargo 读取
    repo 缺失/空串/纯空格/非 str
  那么 一律 state-shape-invalid 结构化 JSON 拒绝(非零退出),
    不再出现 KeyError: verdicts / KeyError: repo traceback

场景: latest/history accepted 必须 bool(critical)
  测试:
    包: octoscode
    过滤: olp_review_latest_accepted_must_be_bool
  假设 accepted 值被注入 1/0/"1"/[]/{}/显式 null(truthy 或 falsy
    非 bool);或 history 记录同类注入;或 latest/history 分别删除该键(缺省)
  当 状态被 cross/status 消费
  那么 非 bool 一律 state-shape-invalid;缺键=缺省豁免;
    accepted:false + imported:true 合法兼容仍可通过;
    全部读点以 is True 判真(truthiness 不再作为依据)

场景: 快照核心 int 0 与 bool False 类型敏感(critical)
  测试:
    包: octoscode
    过滤: olp_review_snapshot_core_false_vs_zero_rejected
  假设 真实 PASS selector 注册(exit_code=int 0,先断言基线为 int),
    快照 latest 或 history 的 exit_code 被改为 bool false
    (Python 宽松比较 0==False 为真的反例)
  当 verify_no_tamper 校验
  那么 live-receipt-tampered 类型敏感拒绝;恢复原件复绿

场景: latest/history 记录形状同门与 legacy 兼容(critical)
  测试:
    包: octoscode
    过滤: olp_review_latest_executed_shape_gates
  假设 latest.executed="bad" 或 executed.artifacts=[](非 dict);
    或所有 accepted 记录(latest 与全部 history)整段删 executed
    (legacy 形态,receipt 原件完整且 hash 一致)
  当 status 校验
  那么 形状注入 → state-shape-invalid;legacy 缺快照由 receipt 单侧
    重建允许,但删除/篡改真实 stdout 工件仍 live-receipt-tampered
    (工件门不可绕),legacy 状态恢复工件复绿

场景: adapter tracked 门三面与 git status fail-closed(critical)
  测试:
    包: octoscode
    过滤: olp_review_adapter_tracked_modified_before_and_after
  假设 fixture probe 测试运行中真实改写已 tracked 的 src/lib.rs
    (执行前 clean,本次 cargo test 运行后 dirty);untracked 探针
    文件;非 git 工作树(夹具外层固定建有真实父 Git 仓库)
  当 生产 adapter 执行(前置/末尾同门)或门函数直调
    (测试子进程以 GIT_CEILING_DIRECTORIES 隔离夹具边界,git/python
    调用带 deadline;TMPDIR 位于 Git 工作树内时同样适用)
  那么 同次执行后 tracked 修改 → tracked-source-modified(执行后
    phase,drift 标记证明测试确已运行);untracked 不触发;CLI 早拒
    非 git 工作树,不误认父仓库 HEAD;git status 查询失败经门函数真实 I/O 返回
    rc=1 + tracked-source-modified(查询失败),不得当作 clean


Rule: actual-model-evidence — 实际模型与行为证据同时满足才可验收

场景: 原生 ledger 的双模型与逐轮锚定(critical)
  测试:
    包: octoscode
    过滤: olp_review_models_require_runtime_evidence
  假设 两个不同 reviewer 各有初审与 cross,且行为条件已满足
  当 从完整原生 ledger 核对两轮 turn_started、turn_completed 和实际模型
  那么 仅 GLM 与 K3 各自族匹配才 model_verified=true;
    缺 peer/证据、旧轮、错模型、失败/中断、输出片段冒充终态、损坏序号、
    异 session、多 cwd 流、证据变更/删除均使 review_accepted=false

场景: 模型验收与真实 Cargo 执行组合(critical)
  测试:
    包: octoscode
    过滤: olp_review_model_gate_composes_with_live_cargo
  假设 冻结评审基于真实 Git fixture,生产 live Cargo adapter 实际运行精确测试
  当 两个 reviewer 的模型 fixture 与 native completed 报告均被 cross 核验收录
  那么 behavior_accepted/model_verified/review_accepted 均为 true;
    删除模型证据锚后行为条件仍为 true,最终验收必须 false
  注: 此自动测试的模型 ledger 是明确标记的合成 fixture,不是线上模型审查收据

场景: 模型日志切段保留与删除前段的恢复指引(critical)
  测试:
    包: octoscode
    过滤: olp_review_models_require_runtime_evidence
  假设 模型 fixture 的完整轮次分为两个 ledger 文件
  当 两段保留时核验，随后删除含初审的第一段再核验
  那么 两段完整时通过；前段缺失时拒绝将尾段首轮当成 turn 1，
    peer-model-unverified 指引新建短 reviewer peer 并重做初审与 cross

场景: 兼容 cross 收录明确警告且有效重提可恢复(critical)
  测试:
    包: octoscode
    过滤: olp_review_model_gate_composes_with_live_cargo
  假设 真实 Cargo 行为证据已经通过，模型 ledger 是明确标记的合成 fixture
  当 不带 --require-model-evidence 收录，再带该参数重新提交有效 cross
  那么 兼容收录返回可解析 JSON warnings 中的 model-evidence-not-verified，
    stderr 提示参数且最终验收仍 false；有效重提无此警告，双 reviewer 完整后验收 true
