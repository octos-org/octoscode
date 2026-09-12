# OLP Review Evidence — 行为证据互审流程

> 入口: `scripts/olp-review-evidence.py` · 监控: `scripts/olp-review-monitor.py`
> Spec: [specs/task-evo-review-evidence.spec.md](../specs/task-evo-review-evidence.spec.md)
> Plan: [docs/superpowers/plans/2026-09-09-review-evidence.md](superpowers/plans/2026-09-09-review-evidence.md)

## 流程总览

```
init ──▶ freeze ──▶ challenge ──▶ cross(×2) ──▶ status
          两份初审     行为证据       逐 claim 裁决    汇总判词
          (冻结+SHA)  (执行为本)     (采纳/反驳/待验证)
```

四个子命令按顺序推进;任一步失败输出可解析 JSON
`{"error": {"code": "...", "message": "..."}}` 且 exit != 0。

## init — 初始化评审目录

```bash
python3 scripts/olp-review-evidence.py init <review_dir> \
  --repo <repo> --base <base_sha> --head <head_sha> \
  --runtime <runtime_path> --session <session_id> --goal <goal_id>
```

记录 HEAD/base/runtime/session/goal 到 `review-state.json`(原子写入)。

## freeze — 冻结两份独立初审

```bash
python3 scripts/olp-review-evidence.py freeze <review_dir> \
  --glm-review glm.md --k3-review k3.md \
  --glm-slug <slug> --k3-slug <slug> \
  --native-root <runtime>/data/peers \
  --head <head_sha> \
  [--runtime-evidence runtime-evidence.json]
```

`--native-root` 为外部权威收据根;`--head` **必填**(锚定评审基线,
frontmatter HEAD 必须与之精确一致)。

**前置校验(fail-closed)**:

| 校验 | 错误码 |
|---|---|
| 两份初审文件都存在且可解析 frontmatter | `first-reviews-incomplete` |
| peer outcome 非 pending/errored(自报) | `peer-outcome-invalid` |
| runtime-evidence 该 slug 无 active_thread | `peer-outcome-invalid` |
| native result-N.md 非 errored(外部否决) | `peer-outcome-invalid` |
| native 收据 slug 与报告 slug 匹配 | `peer-authority-mismatch` |
| **有至少一种外部终止权威**(native 收据或 runtime-evidence 终止快照) | `peer-authority-missing` |
| 报告 turn 恰等 native 最新编号 N(= turns.txt 已结束轮次) | — |
| 报告 turn < N(旧轮初审) | `stale-turn` |
| 报告 turn > N(未来轮) | `turn-mismatch` |
| 报告 turn 非数字/缺失 | `peer-outcome-invalid` |
| 报告 HEAD == 评审 HEAD(必填 `--head` 锚定) | `head-mismatch` |

冻结记录每份初审的 SHA256/peer/turn/outcome;此后任何改写或**删除**
均触发 `first-review-tampered`(verify fail-closed)。

**初审 verdict 白名单(fail-closed)**: 初审报告 claims 块的 `verdict`
只允许 `{approve, request-changes, comment}`(合约初审初始态词表);
空/非 str/cross 词表(accept/refute)或执行后状态(flipped/
blocked-on-evidence 等)→ `claims-verdict-invalid` 拒绝。

**state 形状校验(全入口)**: init/freeze/challenge/cross/status/classify
读取合法 JSON 但结构损坏的 state(顶层非对象/frozen 缺 reviews/head/
verdicts/challenges/repo,或 repo 非非空字符串/嵌套
reviews/challenges/history/latest/verdicts/cross 形状非法)时一致返回
结构化 JSON 错误(`state-shape-invalid` / classify 专用码),对**已列举
的畸形状态形态**不再出现 KeyError traceback(其余错误类型不在此保证
范围)。latest 与 history 记录同门共用形状校验: `accepted` 在场必须
bool(显式 null 非法,缺键才缺省);`executed` 和其 `artifacts`
为非 null 值时必须是 dict,缺键或 null 延续旧版缺省语义。全部 accepted 读点(cross 门/汇总/refutation/
apply 前置)统一以 `is True` 判真,truthiness(如 1/"1")不再作为判真
依据。

**旧 receipt 与日志验真(全生命周期)**: `verify_no_tamper` 对每条
accepted=True 的 live 记录无条件校验 receipt 路径+sha256(改写/删除/
缺键 → `live-receipt-tampered`);receipt_kind 必须
`cargo-test-execution`;快照核心字段与 receipt 类型敏感一致(bool
False 不得冒充 int 0,反之亦然);stdout/stderr 工件删除/改写/缺声明
全拒;legacy 快照(executed 整段缺失或内部缺字段)由 hash 验证过的
完整 receipt **补齐后再验**(不降低强度——工件篡改在 legacy 路径下
同样被拒)。status 以只读 fd 在已有 `.review-state.lock` 上获取共享
flock,与写入口的排他锁协调后读取一致快照,不创建目录或锁。目录或
state 不存在返回 `not-initialized`;锁缺失/不可读取/不可加锁返回
`review-lock-unavailable`,不退化无锁读取。由 init 创建的上下文即使
目录和锁只读仍可查询(不要求锁有写权限)。

**执行前后 tracked 干净门(生产 adapter)**: 被测 repo 的 tracked 源
必须等于 HEAD(staged/unstaged 修改 → `tracked-source-modified` 拒绝,
执行前后同门);untracked 探针/工件是合法测试注入面;git status 查询
失败不得当作 clean。同 selector 多轮执行的 stdout/stderr 工件用
mkstemp 唯一命名,互不覆盖,旧 receipt 的 hash 仍可验证。

## challenge — 行为证据(内容校验为门、执行为本)

**推荐路径 — `--live-cargo`(唯一 live 执行入口)**:

```bash
python3 scripts/olp-review-evidence.py challenge <review_dir> \
  --claim <claim_id> --live-cargo \
  --selector <精确测试名> --expect pass|fail \
  [--lib] [--test-target <集成测试名>] [--exec-repo <repo>]
  [--manifest <Cargo.toml>] [--cargo-target-dir <dir>] [--timeout 300]
```

由本入口实时执行**固定的生产 Cargo adapter**
(`scripts/olp-review-evidence-cargo.py`):repo/manifest/精确 selector/
HEAD/source 绑定与完整输出工件/退出码全部由 adapter 产生并写
`live-evidence/*.receipt.json`;本入口只转发参数、约束 deadline 并复核
自产 receipt(selector/HEAD 一致),不引入任意 shell 旁路。**外部传入
receipt/imported 日志不构成执行证明。**`--selector` 必须是 adapter
`--list` 唯一定位的精确测试名(zero-match 前置拒绝 `no-tests-matched`);
`--expect` 显式声明预期 pass/fail;真实执行但结果与预期相反
(`expectation-violated`)仍如实记录为行为证据。`--lib` 走 src/ 单元测试
路径(源文件 SHA256 绑定),集成测试省略。

**负向判定路径 — 显式 `--evidence` 日志(仅拒绝/分层,永不采信为执行)**:

```bash
python3 scripts/olp-review-evidence.py challenge <review_dir> \
  --evidence ev.log --claim <claim_id> \
  --test-name outer_review_627_x \
  --harness-root tests/ --harness-root src/ \
  --run-dir <repo> [--timeout 300] [--imported]
```

`--live-cargo` 与 `--imported` 互斥。`--exec-argv` 任意执行体一律
`executor-not-trusted`(**永不执行**;唯一 live 执行入口是 `--live-cargo`
经固定生产 adapter)。**判定顺序**:

1. **imported 分层**(先于一切内容判定): 外层导入未独立执行的日志 →
   `not-replayed`,不自动 accepted。
2. **内容门**: 证据含结构锚(测试名行 + panicked at + FAILED 汇总,
   或通过形态 `test result: ok.`);纯文档/字符串断言 →
   `evidence-not-behavioral`,claim 置 `unverified`。
3. **测试名解析**: `--test-name` 必须能在 `--harness-root` 源码中解析到
   `fn <name>(` 定义。
4. **执行终拒(核心)**: 结构锚齐全但无本入口真实执行的 receipt →
   `evidence-not-executed`(外部日志/自造 argv 均不可作为执行证明;
   `/usr/bin/false` 等无关命令同拒)。执行证明只能来自 `--live-cargo`
   自产 receipt(selector/HEAD 绑定)。

**判别式**(executed 后):

- exit≠0 + FAILED 汇总(PR HEAD 真实复现) → `blocked-on-evidence`
- exit=0 + `test result: ok.`(真实通过) → `approve`
- 仅遗留路径可达 → `flipped`(PR 级聚合 residual)

## cross — 交叉互审

**实际模型验收**: `cross --require-model-evidence` 从绑定 runtime 的
`ui-protocol/<hex>/ledger-*.log` 读取原生 JSONL，核对初审和交叉审查各自
的实际模型。hex 目录编码完整 peer session 与可选 NUL cwd scope；行内
session 必须与该 peer 一致。模型来自 `token_cost_update.token_cost.model`，
GLM/K3 lane 分别接受 `glm`/`glm-*` 与 `k3`/`k3-*`。

报告 turn=N 映射到完整 ledger 中第 N 个 `turn_started`，必须有唯一
`turn_completed` 终态；`response`/`stream_end` 是输出片段，不能证明完成。
缺失或损坏的事件、序号缺口、多 cwd 流、失败/中断、模型错配均拒绝。
收录时保存初审及 cross 的 session stream、具体 turn ID、轮次、模型与
相关原始事件摘要；status 重新读取原始记录，逐项匹配这些锚点。

`review_accepted` 现在始终要求原有行为证据门和双模型证据门同时通过。
`behavior_accepted` 单独展示原有行为证据条件，`model_verified` 展示两个
不同 reviewer 的初审与各自最新 cross 是否均核验通过。旧格式或不带模型证据的
cross 仍可收录用于审计，但不能通过最终验收；删除模型锚点也不会退回
旧的通过条件。需重新提交带 `--require-model-evidence` 的有效 cross。
兼容收录会在 JSON `warnings` 数组返回 `model-evidence-not-verified`，
并在 stderr 提示补带参数；stdout JSON 保持可解析。带参数并核验通过的
收录返回空 `warnings` 数组。收录成功不等于最终验收通过。
K3 配额错误等失败记录只能记为审查未完成。

这是对受信本地 runtime 产物的一致性核验，并非提供商的密码学身份认证。
ledger 不完整或归档被移走会降级为未验证，不猜测缺失轮次。已有 live Cargo
入口继续独立核对源码、测试命中数、实际退出码与日志摘要；模型一致不能替代
测试执行证据。

**reviewer 会话生命周期**：使用专用于本轮评审的短 peer 会话，初审与 cross
必须在该 peer 的完整日志仍保留时验收。当前 octos 默认每段约 10 MiB、最多
保留 5 段；仅切段不影响核验，删除最早一段后则可能失去全局轮次映射。
不要复用长时间运行的编码 peer 充当 reviewer。若前段已经丢失，在新的评审
上下文新建 reviewer peers，重做初审、冻结、挑战和 cross；不得重编号剩余
日志、沿用旧 cross 锚点，或把旧报告复制成新模型证据。长期会话支持需要
另外实现可靠持久的报告轮次与 runtime turn ID 映射，当前不接受连续尾段代替它。

```bash
python3 scripts/olp-review-evidence.py cross <review_dir> \
  --cross-report cross.md --cross-slug <slug> \
  --native-root <runtime>/data/peers --expect-claims X,Y,Z \
  --require-model-evidence [--allow-operator-refute]
```

前置: frozen + challenge 已接纳。cross 报告须含结构化 `cross_claims`
块(`cross_claims: [{id, verdict, evidence, ...}]`),逐 claim 覆盖
(`missing-claim-coverage`;无块 → 覆盖校验拒绝);errored/pending 拒绝;
同样要求外部权威收据。**文字+行号反驳不得覆盖已复现失败** —— 结构化
refute 仅在已验证新行为证据或显式 `--allow-operator-refute` 人工裁决
下生效,否则 `challenge-refuted` 回边只认新重放行为证据。

## status — 汇总

```bash
python3 scripts/olp-review-evidence.py status <review_dir>
```

- 无执行证据的两模型一致 approve → `pending-behavioral-evidence`
- PR 级聚合 = f(claim × severity × introduced_vs_existing),
  期望分类来自外层 MANIFEST `outer_recommendation`(禁 PR 编号硬编码)

## 监控入口 — Herdr pane 可读

```bash
python3 scripts/olp-review-monitor.py <review_dir> \
  [--runtime-dir <runtime>] [--profile <profile>] \
  [--board <ack-board.md>] [--session <wire-session>] \
  [--format human|json]
```

四区块: lifecycle / current-turn / last-outcome / deliverables。核心
身份语义(fail-closed,详见 spec `Rule: review-monitor`):

- **runtime 身份是复合标识**(runtime 路径+session+goal),裸 `goal_01`
  不是身份;`--profile` 缺省时不扫描 foreign profile 兜底。
- **thread 晋升 running 需正向身份证明**: 未完 native thread
  (v/session_id/thread_id/next_seq/completed=false,无 active 键)要晋升
  peer 为 running,peer 目录 `originator`(wire master 同源)与 `goal`
  文件必须对当前视角已知字段**实际匹配**——缺失即 unknown
  (`peer-*-unproven-missing`),信息不足 ≠ 归属证明;与当前视角矛盾的
  一律 unknown。身份逐键一致的合法绑定仍 running(正向对照)。
- **终止态只信 native 证据**: result-N.md+turns.txt 交叉一致的
  completed/errored/interrupted/rate_limited 四值如实分层;快照自报
  outcome/outcome_source 仅作 `snapshot-outcome-untrusted:*` notes,
  不构成终止权威。lifetime.json 逐字段严格校验(originator==master 等)。
- **negative_events 归属过滤**: 按当前 profile+goal(+ 事件携带的
  originator session)过滤;当前 profile 缺失不扫 foreign。
- ACK 缓存写自身 `monitor-state.json`(原子),与 `olp-watch-board.sh`
  契约互不干扰;监控绝不改写 review-state.json。

## classify — PR 级通用聚合(spec L55-73/L439-455)

**受信来源门(Blocker1)**: `--review-dir <dir>`(可重复,必填才有来源):
每个上下文须 frozen、reviews 形状合法、真实 repo HEAD == state.head、
verify_no_tamper 通过(同一 flock 临界区读取);注册表 = 其 challenges[*].
history 中 accepted live 记录(receipt canonical 路径 + sha256 +
live-evidence 目录包含),只由 `--live-cargo` 写入,classify 只读。
`--review-dir` **可重复**: BASE/HEAD(及多 PR)各自一个受信上下文,
每上下文保留自己的真实 repo/HEAD(state-HEAD 门不弱化)。caller-selected
context 是**受信边界**(调用方选择信任哪些评审目录),不是密码学签名——
能整套改写受信目录的行动者在边界之外(物理/仓库安全与 freeze 防篡改
层职责)。
per-HEAD receipt 须命中注册(防外部副本/篡改);BASE/HEAD 双执行证明
**两侧**各自绑定注册执行(同 qualified selector + test_target sha +
observed=fail + 真 int 非零 exit + stdout sha == slot 对应 log sha)。
一致性 ≠ 来源: 完全自洽伪造链 → blocked/unassessed;历史未注册数据 →
blocked/unassessed 降级。`clean` 当前为保留态(无受支持的对照证明路径
时不可达,不放宽)。PASS 记录要求 adapter_exit 与 cargo_exit 均为
**真 int 0**(bool 是 int 子类,显式拒绝)。


```bash
python3 scripts/olp-review-evidence.py classify \
  --manifest <MANIFEST.json> --replay-summary <replay-summary.json> \
  --review-dir <ctx1> [--review-dir <ctx2> ...] \
  [--slot <slot.json> --slot-log-dir <双日志目录>]
```

分类意图来自外层(MANIFEST `outer_recommendation`);工具只做通用聚合
派生,**零 PR 编号分支**:

- **per-selector 产品裁决(逐条 receipt 校验)**: 每条失败记录必须
  receipt 文件在场且 summary↔receipt 一致(selector/HEAD before+after/
  observed/exit_code);**缺失/外来伪装/不一致 → harness 证据错误,
  该 PR 一律 blocked/unassessed,不得继承 existing/residual**。
  harness 状态(adapter_exit)与产品失败分列——`adapter_exit=0/
  cargo_exit=101/observed=fail` 是"harness 成功+产品测试失败"(缺陷
  证据),adapter 非零是 harness 故障,均不得混淆。
- **introduced_vs_existing**: 需 BASE/HEAD 同 probe 双执行对照(slot
  sha256 逐字节核验 + pr_head 匹配 + **slot probe 源文件声明的测试
  函数名与失败 selector 精确同名**)。**当前生产仅支持"双 FAIL 形态
  同构 → existing"一种对照模式**;`introduced`(HEAD FAIL + BASE
  PASS)是冻结词表的保留语义,**当前无对应双执行证据模式支撑、不会
  产出**——缺该模式证据时不判 introduced,一律 `unassessed`。
  **缺双执行证据或仅部分失败 selector 被覆盖 → `unassessed`/整 PR
  blocked,禁按 exit_code 相等或任一 head 匹配把 existing 推广到整
  PR;缺双执行证据禁标 clean**。
- **PR 级**: `clean`(全 pass 且无 unassessed)/`residual`(失败全
  existing,判词绑定 slot+双日志)/`blocked`(反例成立且无双执行对照,
  或 harness 收据缺失/hash 不符)。
- claim 级 `flipped` = 遗留/既有路径(legacy/existing),与
  `introduced_vs_existing` 是两个维度;真实执行复现失败 → claim 级
  `blocked-on-evidence`。

## 判词状态机(claim 级两层词表)

```
approve ──challenge(fail)──▶ flipped ──cross反驳──▶ challenge-refuted
   │                            │
   │ 无证据                     │ executed FAIL @HEAD
   ▼                            ▼
pending-behavioral-evidence   blocked-on-evidence
   │ 收口仍缺
   ▼
unverified          not-replayed(imported 未独立复验)
```

## 测试真实性边界

- `tests/olp_review_evidence.rs`(63 enabled / 1 ignored):
  完整冻结基线的嵌套坏形状矩阵覆盖 init/freeze/challenge/cross/status/classify;
  status 探针覆盖缺路径、空目录、真实只读权限、缺锁和 writer/reader 等待。
  frozen 坏状态覆盖 status/cross 与 live-cargo 的 repo 消费入口,包含纯空格
  repo;latest/history 分别验证 accepted 缺键的兼容语义。非 Git 负向夹具
  固定嵌在真实父 Git 仓库下,测试子进程设置 `GIT_CEILING_DIRECTORIES`
  防止隐藏 `.git` 后误认父仓库;新增 git/python 探针使用 deadline。
  该隔离仅用于测试夹具,生产 adapter 仍允许 Git 工作树内的 Cargo 子项目。
  子进程真实调用生产入口;外层反例回归先 RED 后修(证据 `.octos/red-proof/`)。
- `olp_review_k3_full_happy_path_accepted` 替代旧假日志 approve 路线
  (python 假 cargo 日志违反合约 3,已 REMOVED)。
- `olp_review_real_store_harness_executes_historical_negative_probes`:
  临时 clone 固定 PR 合成树 + `include!` 外层诊断反例源码,真实 cargo
  编译执行 8 探针(约 30s,独立昂贵验收,不在每个单测重复)。
- Python 打印 cargo 样式文本的伪造 argv 无法成为行为 verified。
- 顺序如实记录: 首批 18 测试与脚本同轮实现(先落盘后补测试);
  外层反例与新场景按先 RED 后修。
