# Outer-Loop Protocol (OLP) — v0 草案

> 让任意外部模型(Claude Code / Codex / 脚本化 agent)以标准方式**计划、监控、
> 审查、指导** octos 的长程 goal 执行。本协议规范化的是已在实战中验证过的信道,
> 不发明新机制;L1/L2 是短期补齐路线。
>
> `protocol: olp/v0`

## 角色

| 角色 | 职责 | 模型档位 |
|---|---|---|
| **operator**(人) | 宏观指令(`/goal`)、终审、审批 | — |
| **runtime**(octos serve + master/peers) | 长程执行:goal keeper 推进、peer 并行干活 | 苦力档(k3 / cheap lanes) |
| **outer agent**(本协议的对象) | 计划、事件驱动监控、交付审查、指导、基建维护 | 强档(Fable / GPT) |

## L0 信道矩阵(现状即可用,全部已实证)

### 下行:outer → runtime

| 信道 | 载体 | 时效 | 用途 |
|---|---|---|---|
| 会话常驻约束 | `AGENTS.md`(octos prompt_layer 自动注入每个 session) | session boot | 纪律、协议本身的引导 |
| 任务级指导 | `docs/OUTER_LOOP_REVIEW.md` 的 `Active` 区(带日期条目 + `ACK:` 行) | master 每轮读 | 审查意见、整改要求;历史区不可执行 |
| 既成事实 | 原子 git commit | 立即 | 代修、基建修复 |
| 事件提示 / **门铃** | inbox `<session-hash>.notes` | 下一 turn,阅后即焚 | 仅事件通知与**黑板指针**("第 N 条已更新,去读并执行");不承载指令内容本身 |
| **TUI 注入** | `herdr agent prompt <pane> '<text>'`(需 herdr ≥ Ti-Agent-OS fork `fc414dd8`,含 octoscode manifest)或 tmux `send-keys` | ~8s 内开 turn | 唯一实证"读**且执行**"的即时下行通道——文本落在 composer 即用户消息层级,等价 operator 亲手输入;steer API(L2)落地前的事实标准 |

### 上行:runtime → outer

| 信道 | 载体 | 特性 |
|---|---|---|
| 事件流 | serve 日志 `peer-goal:*` / escalation / `transitioned goal` 行 | outer 侧 tail+filter,事件驱动零轮询 |
| 交付物 | `peers/<slug>/result.md`(frontmatter: `slug/outcome/updated_unix/turn`) | 每轮交付 |
| 权威账本 | `goal-ledgers/<goal_id>` | durable,重启幸存 |
| 求助 | escalation(park 于 approval/question) | 分级升级,见 R3 |
| 代码 | git log / diff | 审查对象 |

## 协议语义(核心规则)

- **R1 — ACK 义务**:`docs/OUTER_LOOP_REVIEW.md` 的 `Active` 区中每条意见,runtime 侧执行后
  必须在条目下补 `ACK: <做了什么 / 为何不做>`。无 ACK 视为未读,outer 有权打回交付。
- **R2 — 诚实验证声明**:runtime 侧每个交付必须声明验证级别之一:
  `verified`(跑过 `cargo test --all-targets` + clippy + fmt)/
  `partially-verified`(列出跑了什么)/ `unverified`(说明原因,如无工具链)。
  声称 verified 但复验不符,视为协议违例,outer 打回并记入黑板。
- **R3 — 升级分级**:escalation 三级——runtime 自决(重试/换法)→ outer 裁决
  (技术取舍、批不批一个方案)→ operator 裁决(权限审批、范围变更、对外动作)。
  outer 不得代替 operator 按下审批;operator 缺席时 escalation 保持 park。
- **R4 — 工作区共存**:同一工作区多写者(master/peers/outer)各自只
  `git add` 自己改的文件,禁止 `git add -A`;改动即原子 commit,不留长时间
  未提交状态。来源不明的 dirty 文件必须保留并报告,不得自动清理或提交。
- **R5 — 指导幂等**:outer 的意见带日期与唯一编号,只在 `Active` 区可执行;
  ACK 后移入历史区且永不重放。重复投递以 ACK 为去重依据。
- **R6 — 版本协商**:本文件头部 `protocol: olp/vN`;`AGENTS.md` 引用同版本。
  信道语义变更必须升版本。

## 接入清单(一个新的 outer agent 需要知道的全部)

0. 启动命令注意:单人本地盒子要给 serve 带 **`--solo`**
   (`octos serve --stdio --solo`,或 `OCTOS_SOLO_LOGIN=1`)。宽松
   permission profile(yolo/approvals-never/network-allow)是安全基石门,
   仅在显式 solo opt-in 下可设——漏掉它的症状是
   "requested permission profile is not allowed outside local solo mode"。

0b. **权限档 = 文件系统沙箱开关,不是审批开关**(2026-08-24 实战定论):
   /permissions 菜单 1-4 档(Default/Read Only/Workspace Write/Never Ask)
   都运行在 bwrap 沙箱里——只挂载工作区与系统目录,`~/.cargo`/`~/.rustup`
   **不可见**,任何构建命令都是 "command not found",历史上 peer 反复声明
   "本机无工具链" 的真相即此。要跑构建必须第 5 档 **Full Access**(免沙箱),
   或 serve 带 **`--danger-full-access`**(默认所有未显式选择的 session 为
   Full Access;solo 门控;`OCTOS_DANGER_FULL_ACCESS=1` 等价)。注意:
   给 agent 授免沙箱权限属 operator 亲手动作——外环自己的 harness 也会
   拦截代按,不要尝试绕过。标准启动命令:
   `octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'`。
   重启 checklist:①上述命令启动(operator)②`/loop resume`(外环可代)。

1. 数据根:`~/.octos/instances/<cwd-hash>/profiles/<profile>/data`
   (cwd-hash = 项目目录的 DefaultHasher 十六进制;L1 将提供查询命令代替自算)。
2. 挂事件监听:tail serve 日志,过滤 `peer-goal:|escalation|transitioned goal|ERROR`。
   实测三个坑:(a) 管道每级都要行缓冲,且用**绝对路径** `/usr/bin/grep
   --line-buffered`——shell alias(如 ugrep)会让 `--line-buffered` 静默失效,
   事件卡在缓冲区里,监控"活着但瞎了";(b) 日志按进程启动日期滚动,跨天要
   同时 tail 两个日期文件;(c) 观测分三层——**投递**(notes 文件被清空)≠
   **消费**(turn prompt 读到)≠ **执行**(交付/ACK 落地),只看一层必误判,
   turn 心跳以日志里 `^` 锚定的时间戳行判定,勿取续行。
3. 读本文件 + `docs/OUTER_LOOP_REVIEW.md` 的 `Active` 区了解当前指导;
   `Historical record` 仅用于审计。
4. 审查交付:`peers/*/result.md` → git diff → 独立复验(R2)。
5. 写指导:黑板追加条目;紧急基建问题直接原子 commit(R4)。
6. (可选,推荐)驾驶舱注入通道:herdr 里跑 octoscode 后,
   `herdr agent list` 应显示 `octoscode | <pane> | idle`,
   `herdr agent prompt <pane> '<text>'` 即用户消息层级下发。herdr 的注入
   有双重门:named-agent 名单 + pane 前台进程名匹配,二者缺一注入被静默丢弃
   ——所以需要 fork 补丁把 octoscode 编入 Agent 枚举与 manifest,本地
   index.toml 覆盖**不能**新增 agent。降级方案:tmux `send-keys`(注意
   首字符为 `-` 的文本会被当 flag 吃掉,用 `--` 分隔)。

## 路线

- **L1(小改动,高收益)**:`octos goal status --json` / `octos peer list --json`
  只读观测命令;`result.md` frontmatter schema 固化;per-turn hook 挂机制化验证
  (peer 完成 → 自动 test → 结果写 ledger);sub_providers 苦力车道配置模板。
- **L2**:带 ACK 的 `session/steer` 旁路 API(WS/HTTP),替代"黑板等 master 读"
  的被动时效;事件订阅端点替代日志 tail。

### 心跳自治:maintenance loop × 黑板(实测定型)

自治回路的基座是 octos 自带的 maintenance `/loop`:server 定时给 master
创造 turn,bare `/loop` 的 prompt 从仓库的 **`.octos/loop.md`** 解析——
外环拥有心跳指令的定义权。本仓库的 loop.md 约定:每次醒来只执行黑板
`Active` 区中编号最小的未 ACK 条目并原地 ACK;没有 active 工作时不得
清理或提交来源不明的工作区内容。operator 一次性设置
`/loop every 30m` 后,外环写黑板即等于下发任务,无需任何人工转发。
两个实测教训已吸收:(1) inbox notes 只是"投信"而非"按铃"——外环写
notes 不会创造 turn;(2) system prompt 附加段的指令会被读而不被执行,
行动指令的可靠载体是黑板条目 + loop.md 的执行契约(steer 落地后升级为
用户消息层级注入)。steer(REQ-OLP-CTRL)因此从必需品降级为低延迟
优化:心跳保底自治,steer 把响应从"下个心跳"缩到"立即"。

### 推/拉间隙与门铃模式(实测补充)

黑板是拉模型:runtime 侧"每轮任务开始"才读。当 goal 已 complete、master idle
时,**没有任何自动 turn 会去读新的黑板条目**。v0 的 workaround 是"门铃":
outer 往 inbox notes 写一条只含指针的通知(内容留在黑板),再由 operator 说
任意一句话触发下一个 turn——门铃随 prompt 注入,master 循指针读黑板执行。
L2 的 steer API 将消除"需要 operator 说一句话"这最后一步。

## v0 实验记录(2026-08-23,非持续认证)

一次双环协作实验(外环 Claude Code/Fable 5,内环 octoscode + 苦力模型,
黑板十条评审项)于 2026-08-23 完成记录。下表是当时的信道/流程样本,
不是对未合并 PR、当前 main 或具体 runtime 实现的持续认证:

| 条款/机制 | 实证样本 |
|---|---|
| AGENTS.md 注入 + R6 版本 | olp/v0 金丝雀握手:新 session 首轮即复述协议头 |
| R1 ACK 义务 | 十条评审项全部带 ACK 闭环,含整改与拒绝两类 |
| 异议路径(ACK: wontdo) | #9:内环以证据拒绝重派指令,外环复核后接受——内环是对的 |
| R2 诚实验证 + 分工 | #8:peer 声明沙箱无工具链(unverified),外环曾代跑测试;后续实现缺陷见 review 的更正记录 |
| R3 升级分级 | #10:新依赖(signal-hook)曾走升级请求;旧 suspend 实现后来被审查判错,见 review 的更正记录 |
| 心跳自治 | maintenance `/loop` 30m + loop.md 执行契约,黑板新条目无人工转发即被执行 |
| 门铃模式 | inbox 指针 + 黑板内容分离,两次实测"读而不执行"事故后定型 |
| TUI 注入 | herdr fork 补丁 e2e:`agent prompt` → ~8s 内 turn 启动 |

结论:v0 实验说明黑板、ACK 和注入信道可以协作;它不证明当时各运行时改动
正确。具体行为只能由 main 上的代码、绑定 commit 的测试和独立复验确认。
主要摩擦点(观测靠 tail 日志、寻址靠自算 hash、steer 靠 TUI 注入)仍是
L1/L2 路线的动机,对应 REQ-OLP-{OBS,CTRL,HEADLESS}。

## 实战沉淀·第二辑(2026-08-24 性能战役,10.5s→0.21s 全程)

一次跨两仓库的真实性能战役(octoscode #578 + octos #2114),外环三次
带证据改判、内环两次带证据抗命、一次外环拦截倒退修复。教训按角色归档:

### 外环派发纪律

- **迭代预算是任务切分的硬约束**:内环单 turn 有迭代上限(实测 50),
  超限即静默收工(改动留工作区、无 commit 无 ACK,外观像"卡住")。
  任务书必须按 turn 预算切片(如 commit A/B 分步),重活拆成连续小条目。
- **指导粒度决定成败**:给"设计要求"内环会拿模糊方向瞎撞(丢唤醒假说、
  writer 重写两次弯路);给"行号级定位 + 证据链 + 照图施工"一次到位。
  定位性工程(读代码、置探针、断因果)是外环的活,不要外包给苦力模型。
- **外环诊断用独立 git worktree**:与内环共享工作区做探针实验必然纠缠
  (实测:探针与修复混编译断、diff-preview 竞态 SIGBUS)。R4 的外环版:
  诊断改动永不落在内环正在写的 worktree。

### 验收纪律(R2 增补)

- **测试全绿 ≠ 真机正确,外环真机验收是终审**。两个实证反例:快照机制
  单测全过但对存量大账本零收益(cadence 永远不触发);writer 超时重驱动
  内存 duplex 测试全过但真管道字节流损坏(write_all 被 drop 丢进度偏移)。
  凡涉 IO/并发,test double 必须用真 OS 原语(真管道、真文件),内存
  替身只配当冒烟。
- **恒定时长 = 固定成本或超时**的启发式两次立功:耗时不随数据量变 →
  别在数据路径找,去找 timeout/池 keep_alive/固定循环。
- **探针三重校验**:源文件 grep → 二进制 strings → 运行时输出,三关都过
  才算探针在场;缺一关就会像"writer 无输出"那样误导整轮推理。
- 观测两坑:tracing subscriber 可能是线程作用域——裸 OS 线程(如专职
  writer 线程)的 tracing 输出会静默丢失,改直写文件;serve 子进程的
  stderr 被 octoscode 收进 ring buffer,eprintln 探针不可见。

### git 运维

- **"同内容异历史"的收编**:cherry-pick 拆 PR 上游合并后,本地 main 与
  origin/main 是同样改动、不同哈希——此时 pull/rebase = 每文件冲突地狱。
  正解:存档分支(`archive/<date>`)→ `reset --hard origin/main` → 唯一
  上游没有的内容(黑板叙事)单 commit 补回。
- **octos 构建永远带 `--features api`**:漏掉则 serve 子命令消失,
  octoscode 启动失败,症状与代码 bug 无法区分(累计踩坑三次)。

### 外环 harness 自身边界

外环(Claude Code)的权限分类器会拦截:给 agent 授免沙箱权限、批量搬移
serve 数据目录等动作。这类动作按 operator-tier 处理:摆好现场(菜单开到
目标项、命令写好)交 operator 一键完成,不要尝试绕过——R3 由"外环不应
代按"升级为"外环技术上也无法代按",这是特性不是缺陷。

## 已知局限(v0)

- inbox notes 阅后即焚,曾实测吞掉过指导——故 R 系列规则不依赖它。
- 日志文件按进程启动日期滚动而非自然日;tail -F 需同时跟前后两天的文件。
- session-hash 使用 Rust DefaultHasher,跨 Rust 版本不保证稳定(上游注释已声明);
  outer 不得将其用于持久寻址。
