# OctoLoop 使用指南与机制说明

> 一本面向用户与贡献者的整书:**上篇·使用指南**(新用户半小时从零跑通
> 双环:安装、一键引导、三模式上手、内环形态与模型车道、故障速查)+
> **下篇·机制说明**(双环角色与信道矩阵、核心纪律 R1-R6、五项引擎
> 机制、startup --prompt 恰一次语义)。上篇面向"用起来",下篇面向
> "看懂它";两篇小节互相引用、不重叠。所有命令均为发现式,不假设
> 你的机器长什么样。权威细节以仓库内文档为准:上手
> `docs/OLP_QUICKSTART.md`;纪律 `docs/OUTER_LOOP_PROTOCOL.md`;能力
> 语义 `docs/OCTOLOOP_FEATURES.md`(32-r1 核准版)。

---

## 上篇 · 使用指南

## 1. OctoLoop 是什么

OctoLoop 是一套"双环"协作系统:**内环**由便宜模型(octoscode TUI +
octos serve,如 glm/kimi 档)在你的仓库里干活——读黑板、执行、commit;
**外环**由强模型 CLI agent(Claude Code / Codex)审查——读黑板、派整改单、
独立复验、采认后代为 push。内环便宜、可反复重跑,外环贵、只花在审查与
裁决上;推送权只在外环,每个 commit 都过两双眼睛。

双环的价值在于成本与安全的错位配对:内环模型便宜,失败了重跑不心疼,
适合机械性、可回滚的执行工作;外环模型贵但判断力强,只花在审查与裁决
这两个最值钱的环节上。内环永远不 push——推送权只在外环,每个 commit
都要过两双眼睛,这是整套系统的安全底线(纪律细节见机制篇(下))。

**命名**:OLP 是协议名(Outer-Loop Protocol,文档与 ACK 定式沿用);
**OctoLoop** 是产品名——用户视角的一键化封装(`.claude/skills/octoloop`
三模式入口)。能力全景见 `docs/OCTOLOOP_FEATURES.md`。

## 2. 安装与引导

### 2.1 环境依赖清单

| 依赖 | 必须? | 说明 |
|---|---|---|
| Linux / macOS | 是 | Windows 可跑 TUI;serve 的 bwrap 沙箱档是 Linux 特性 |
| octoscode 可执行 | 是 | `npm install -g @octos-org/octoscode`(octos server 首启自动下载到 `~/.octos/bin`) |
| 内环模型 API key | 是 | 便宜档,例:Moonshot(kimi)/ ZAI;onboarding 向导里粘贴 |
| 外环模型 CLI | 是 | Claude Code / Codex 任一,用你已有的订阅 |
| herdr 或 tmux | 推荐 | 外环程序化驱动内环窗格;herdr 来源 <https://github.com/hagency-org/herdr>(octoscode 窗格识别当前在 `feat/octoscode-agent` 分支构建),不装可降级 tmux send-keys |
| bwrap(bubblewrap) | Linux 自带居多 | 权限档 1-4 的文件系统沙箱(见 §3.1 免沙箱说明) |

不需要 Rust——源码构建才需要(Rust 1.85+)。

### 2.2 一键路径

```bash
# ① 安装 octoscode
npm install -g @octos-org/octoscode

# ② 在项目目录初始化 OLP(可重复执行,不覆盖已有文件)
cd your-project/
octoscode olp init .

# ③ 启动内环
octoscode --stdio-command 'octos serve --stdio --solo'
```

`octoscode olp init` 做四件事:依赖体检、生成 `.octos/loop.md` 与
`.octos/OUTER_LOOP_REVIEW.md`(黑板模板)、黑板加 `.gitignore`、打印启动
命令。两件事脚本刻意不代办:API key(向导里自己粘贴)、免沙箱授权(见 §3.1)。

初始化入口仍调用内置的 Bash 脚本,无需下载源码或查找脚本文件。
Windows 下的 Git Bash 选择及工具依赖见 [OLP 快速开始](OLP_QUICKSTART.md)。

## 3. 三模式上手

OctoLoop 的一键入口是 `.claude/skills/octoloop` 这张 skill 卡(即本仓库
自带的 `/octoloop` 命令)。三个模式,**先选身份再动手**;权威规程在仓库
文档,先读再动,不要凭记忆操作。

### 3.1 模式 init — 铺脚手架(首次/新机器)

引导运行 `octoscode olp init .`,完成后逐项核对 `docs/OLP_QUICKSTART.md`
§1 依赖清单;缺口按 §6 故障速查处理,再跑 QUICKSTART §5 冒烟验证(两分钟:
发个 hello、黑板首条 ACK 掉、`herdr agent list` 显示 octoscode 窗格)。

**权限档必读**:权限档 1-4 都运行在 bwrap 文件系统沙箱里,`~/.cargo`、
`~/.rustup` 对 agent 不可见,构建命令会 "command not found"。要让内环跑
cargo/npm,选第 **5 档 Full Access** 或启动带:

```bash
octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'
```

`--solo` 是单人本地安全门,漏掉会报 "not allowed outside local solo mode"。
免沙箱是安全决策,请亲手做,不要让 agent 代按。

### 3.2 模式 outer — 外环上岗三步

1. **读规程**:`docs/OLP_OUTER_BOOT.md`(操作面)+ `docs/OUTER_LOOP_PROTOCOL.md`
   (ACK 定式/多外环规则/预算档)。
2. **发现现场**:`herdr agent list` / `ls -t ~/.octos/instances/`,读各项目
   `.octos/OUTER_LOOP_REVIEW.md` 尾部在途条目。
3. **接管职责**:署名落板(经 `scripts/olp-board-append.sh`,flock 原子)
   → 立编号条目唤醒内环 → 内环 ACK 后隔离 worktree 独立复验 → 采认代推
   (内环永不 push)。

### 3.3 模式 inner — 内环形态

内环契约 agent 无关,按任务形态选型,见下节。

## 4. 内环三形态选型

skill 卡的模式 inner 要求按任务形态选内环形态;三档各有定位:

| 形态 | 适用场景 | 关键点 |
|---|---|---|
| **octoscode 标准** | 仓库内编码主路径 | octos serve stdio 挂载,全工具面 + MCP 第五信道(ask_outer/report_blocked) |
| **claude / codex 免审批窗格** | 快轨修订、外环同级复审 | herdr 窗格隔离,绕内环审批链;分支纪律照旧 |
| **强档车道** | 大型战役/多 peer 并行 | profile 多模型 lane(见 §5),sub_providers 供 pipeline 按节点选档 |

任何形态都要遵守:黑板 ACK 定式、工作区共存与树主权、诚实验证声明
(verified/partially/unverified)——规则见机制篇(下)。

## 5. goal 模式与 peer 并行上手

内环三形态(§4)解决"单个 agent 怎么跑",本节解决"一个目标怎么拆给
多个 peer 并行跑"——goal 是跨回合的目标外壳,peer 是受派干活的车道
工人。状态机与生命周期原理见机制篇(下)。

### 5.1 开一个 goal

两条入口,效果一致:

- **TUI 会话内**:`/goal <目标描述>`——goal 绑定当前会话,goal keeper
  跨回合推进,直到完成或中止。
- **master 侧脚本**:`goal_create`,建立 goal 对象(objective 必填,
  可选 token 预算;状态从 active 起)。

经验:goal 数值保持**每会话一个在途**——历史会话残留的未收口 goal
会以 blocked 之姿拦截新 goal_create(见 §7 坑位),开新目标前先收旧。

### 5.2 派 peer:peer_handoff 与 model 车道

`peer_handoff` 把一份自包含任务书交给一个 peer,关键字段:

- **`name`**:peer 名,必须唯一(同名会踩车)。
- **`brief`(任务书)**:**自包含**——peer 看不到你的会话上下文,
  仓库路径、分支、验收标准、素材权威源都要写进任务书本身。
- **`model`**:车道参数,值是 sub_provider key(如 `zai`/`kimi`),
  让不同 peer 跑不同模型车道。**未配置的 key 回退主道并出 model_note
  警告**——看到警告先查 `~/.octos/profiles/<id>.json` 的
  sub_providers(配法见 §6)。车道解析语义细节见机制篇(下)。
- **worktree 围栏**:多 goal 并行时各 peer 在独立 worktree 干活,
  防止两个目标在同一棵树撞分支——这是 R4b 的自动落地(见机制篇)。

### 5.3 观察:三只眼睛

- **TUI agents 栏**:`Alt+P` 打开,`Ctrl+L` peek——实时看各 peer
  状态与车道。
- **CLI**:`octos peer list` 列 peer(含 model 车道);`octos goal
  status --goal <id>` 看 goal 状态与 token 消耗。
- **事件流**:`tail -f .../data/events.jsonl` 可见 peer_staged
  (含 model_lane)与 goal_transition 事件(见 QUICKSTART)。

### 5.4 收口正解与坑位

goal 达成后**必须显式收口**,否则变僵尸拦截后续 goal:

- **会话内 `/goal stop`**——一步到位的正解(goal 转 complete)。
- **离线 CLI** `octos goal archive/reopen`:**仅在 serve 不在跑时
  可靠**。坑位:serve 存活时,其 live cache 稍后持久化会把离线归档
  **反盖**回去(账本时间线实证:archived 被 blocked 覆盖)——此时
  一律走会话内 /goal stop 或在线 RPC,别用离线命令。

### 5.5 实战:本书自己就是样例

- **#33 双车道成书**:master 恰派两 peer,`model=zai`(guide-part1-zai)
  与 `model=kimi`(guide-part2-kimi)各写一篇,汇稿成
  docs/OCTOLOOP_GUIDE.md——peer_handoff 显式车道参数的端到端实证。
- **#34 goal_07 端到端**:goal_create 建立 goal_07 → 单 commit 完成
  修订 → 收口 complete,token 消耗与零残留可查——goal 通道从建立到
  收口的全链样例。


## 6. 多模型 lane 配置

主对话车道配在 `~/.octos/profiles/<id>.json` 的 `config.llm`(发现:
`ls ~/.octos/profiles/`),逐字段:

- **`primary`**:常驻主道 `{provider, model}`——战役主力。
- **`fallbacks[]`**:断供自动降级序列,quota/auth 拒付时逐道切;
  **顺序即优先级**。
- 修改后**新建会话**生效(工具与配置在会话建立时快照);profile JSON
  时间戳字段必须 RFC3339 带 Z。

实例(zai-coding glm-5.3 主道,k3(moonshot-coding)兜底、deepseek 应急):

```json
{
  "llm": {
    "primary": { "provider": "zai-coding", "model": "glm-5.3" },
    "fallbacks": [
      { "provider": "moonshot-coding", "model": "k3" },
      { "provider": "deepseek",        "model": "deepseek-chat" }
    ]
  }
}
```

**断供降级体感**:断供发生时对话不停——状态栏闪一次降级提示,响应继续;
恢复后主道自动回归,无需重启。

## 7. 常见故障速查

从 QUICKSTART §6 扩充,按"看到什么 → 为什么 → 怎么办"排列:

| 症状 | 原因 | 处置 |
|---|---|---|
| `octos: 'serve' 不是子命令` | 源码构建漏了 feature | `cargo build --release --features api`(发布二进制无此问题) |
| 内环说"本机没有 cargo" | 权限档 1-4 的 bwrap 沙箱挡了工具链 | 第 5 档或 `--danger-full-access`(配 `--solo`,见 §3.1) |
| serve 拒启:"permission profile not allowed outside local solo mode" | 启动少了 `--solo` | 命令补上 `--solo` |
| 工具面缺 MCP 工具(ask_outer 等不出现) | profile JSON 写坏(时间戳缺 Z 等)或会话早于配置快照 | 修 profile(`ls ~/.octos/profiles/` 定位),**新建会话**再验 |
| 黑板没被内环读到 | 黑板被误 track/跨分支裂脑 | 确认 `.octos/OUTER_LOOP_REVIEW.md` 在 `.gitignore`(olp-init 已做);重跑 init 幂等补 |
| 断供空转/全线报错停摆 | 未配 fallbacks,主道 quota/auth 拒付 | 按 §5 配 `fallbacks[]`,新会话生效 |
| herdr 注入静默丢失 | 双重门:named-agent 名单 + 窗格前台进程名匹配 | 缺一即丢;降级 tmux `send-keys`(`-` 开头文本用 `--` 分隔) |
| 首启下载 server 失败 | 离线/代理 | 手装 `npm i -g @octos-org/octos`;`OCTOSCODE_NO_AUTO_INSTALL=1` 关自动装 |
| Linux 构建大项目链接器 SIGBUS / EDQUOT | `/tmp` tmpfs 带配额 | `export TMPDIR=~/.local/tmp`(建目录后写进 shell profile) |

### 7.1 平台支持矩阵

以协议 R7 平台边界、`outer_duty` 模块 doc 与 #38 风险注为权威源:

| 平台 | 状态 | 说明 |
|---|---|---|
| Linux | 全功率 | 全部能力含 outer-duty 内核锁、bwrap 沙箱档 |
| WSL2 | 等同 Linux | prctl//proc/bwrap/bash/herdr 全在,实战结论直接适用 |
| macOS | 可用,两缺口 | ①outer-duty 锁不可用(Linux-only,多外环退回值班簿纪律层+operator 裁决);②bwrap 档为 Linux 特性,权限档 1-4 沙箱语义不成立;herdr 未验证,降级 tmux(`--` 分隔坑见 BOOT) |
| Windows 原生 | 不推荐 | octoscode 构建未完整验证(CI windows job backlog 在案);bash 脚本/herdr/tmux 缺失,外环驱动观测通道瘫;活 serve 的 goal archive/reopen 一律 fail closed;请用 WSL2 |

注:①outer-duty 锁的权威语义见协议 R7(守护式死亡耦合,Linux-only,
非 Linux unsupported 退出 2,Windows LockFileEx 另立条目);②值班簿
在任何平台都只是提示性目录,不构成所有权;③平台能力变化时先改
协议/模块 doc,本矩阵随后同步——本表不发明新承诺。

---

*下篇《机制篇》覆盖:黑板协议与 ACK 定式、R2/R4 纪律、预算治理、
MCP 第五信道、断供降级引擎等原理。*

---

## 下篇 · 机制说明

## 1. 双环角色与信道矩阵

### 1.1 三个角色

OctoLoop 把一个长程任务拆给两圈 agent 加一个人,三者模型档位刻意错开:

| 角色 | 职责 | 模型档位 |
|---|---|---|
| **operator**(人) | 宏观指令、终审、审批(推送、授权、范围变更) | — |
| **内环 runtime**(octos serve + master/peers) | 长程执行:goal keeper 推进、peer 并行干活、按黑板整改 | 便宜档(如 kimi/k3 车道) |
| **外环 outer agent**(Claude Code / Codex 等) | 计划、事件驱动监控、交付审查、指导、基建维护 | 强档 |

设计动机:内环模型便宜、可反复重跑,出错成本低;外环模型贵,只花在
"判断"上——每个 commit 过两双眼睛,推送权只在外环。双环之间的契约
(ACK、验证声明、升级路径)由 OLP 协议文档钉死,与具体模型无关:换
一个外环 CLI 或换一批内环车道,协议不变。

### 1.2 信道矩阵

双环不共享内存,一切协作走**可审计的持久信道**。下行(外环 → 内环)
以黑板为主、注入为辅;上行(内环 → 外环)有六条信道:

| # | 信道 | 载体 | 用途与特性 |
|---|---|---|---|
| 1 | 事件流 | serve 日志 `peer-goal:*` / escalation / `transitioned goal` 行 | 外环 tail+filter,事件驱动零轮询 |
| 2 | 交付物 | `peers/<slug>/result.md`(frontmatter schema) | 每轮交付的权威回执,单写者契约 |
| 3 | 权威账本 | `goal-ledgers/<goal_id>` | durable 审计与读面,重启幸存(状态裁定拓扑见机制篇 §3.5) |
| 4 | 求助 | escalation(park 于 approval/question) | 分级升级,见 R3 |
| 5 | **主动问询(MCP)** | `octoscode olp-mcp-serve` 子命令 | 内环 turn 内**同步**问外环,见下(绰号"第五信道"沿用历史序数:上行前四信道+MCP=五;"代码"信道系后加入,现排第 6 行) |
| 6 | 代码 | git log / diff | 审查对象,原子 commit 即既成事实 |

前四条与第 6 行代码信道是"外环拉取"模型:内环留下痕迹,外环循事件来读。第 5 行的主动问询信道(绰号"第五信道")
(#31 落地,纯 Rust 实现,无 Python 依赖)方向相反——**内环主动推**:

- **挂载方式**:profile 的 `config.mcp_servers` 指向 octoscode 可执行
  文件,`args` 为 `["olp-mcp-serve"]`;挂载后内环模型 turn 内可原生
  调用两个 MCP 工具(接线步骤见 OUTER_LOOP_PROTOCOL.md 接入清单第 7 项;
  就地两行:profile `config.mcp_servers` 里 `"command"` 指向 octoscode
  可执行文件、`"args": ["olp-mcp-serve"]`,新会话生效)。
- **`ask_outer(question, context, tried)`**:turn 内同步发问。信箱目录
  `~/.octos/outer/mcp/` 下 `questions/` → `answers/` → `consumed/`(取答
  后归档)。**90s 超时降级**:超时返回降级指引而非卡死 turn。
- **`report_blocked(reason, needs)`**:直接把阻塞直报外环看板,不经
  信箱往返。
- **两条防滥用闸**:每进程限 3 次问询;`tried` 参数必填(先说清自己
  试过什么)——防止内环把思考外包给强模型。
- **审计**:全程记入 `OUTER_LOOP_MCP.md`,署名 `MCP(ask_outer)`。

体感:内环遇分歧 90 秒内拿到外环(人工或外环 agent)的答复;超时则
拿到"自行降级处理"的指引,turn 不被阻塞。

## 2. 核心纪律 R1–R6

协议文档的 R1–R6 是双环协作的"交通法"。每条纪律都对应一类真实事故,
由契约测试或 ledger 审计钉住,不是建议。

### R1 — ACK 义务

黑板 `Active` 区的每条意见,内环执行后必须在条目下补一行 ACK;**无
ACK 视为未读**,外环有权打回交付。v1 起 ACK 用定式语法(契约测试
`olp_ack_lines_match_v1_grammar` 钉住):

```
ACK(done|wontdo|blocked): <说明>
```

- `done` — 已执行,说明里写做了什么与证据(commit hash / 测试结果)。
- `wontdo` — 带证据的异议。**分歧规则:对 wontdo,外环只能"接受"或
  "升级 operator 裁决",不得对同一条目再次打回**——防止无限打回循环。
- `blocked` — 被阻塞,写阻塞原因与解除条件。

v1 语法只约束 2026-08-24 起新增的 ACK 行,历史行由豁免清单覆盖,
不重写。

### R2 — 诚实验证声明

每个交付必须声明验证级别之一:

- `verified` — 跑过 `cargo test --all-targets` + clippy + fmt;
- `partially-verified` — 列出实际跑了什么;
- `unverified` — 说明原因(如无工具链)。

声称 verified 但复验不符,视为**协议违例**,外环打回并记入黑板。实战
增补:测试全绿 ≠ 真机正确——凡涉 IO/并发,外环终审要求真 OS 原语
复验(真管道、真文件),内存替身只配当冒烟。

### R3 — 升级三级

escalation 分三级,各有明确的裁决人:

1. **runtime 自决** — 重试、换法,master 就地解决;
2. **outer 裁决** — 技术取舍、批不批一个方案;
3. **operator 裁决** — 权限审批、范围变更、对外动作。

边界:外环不得代替 operator 按下审批;operator 缺席时 escalation 保持
park 状态,不强行推进。外环还要主动审计 master 的中途自决(ledger 的
decisions/escalations 表)——master 不会为"自认为解决了"的事再上报。

### R4 — 工作区共存

同一工作区多写者(master/peers/外环)并存,规则三条:

- 各自只 `git add` **自己改的文件**,禁止 `git add -A`;
- 改动即原子 commit,不留长时间未提交状态;
- 来源不明的 dirty 文件**保留并报告**,不得自动清理或提交。

### R4b — 树主权与自动围栏

R4 管"同树多写者",R4b 管"多 goal 撞同一棵树",是系统默认机制而非
外环手工活(octos #20-20c 移交,作为 R4 子条款,不升协议版本):

- **自动围栏**:`peer_handoff` 未显式指定 worktree 时,撞车谓词
  (active goal > 1 / peer 目标分支 ≠ 主树当前分支 / 主树有未围栏在途
  peer)命中即自动开围栏——worktree clone,分支 `peer/<slug>`。单 goal
  单分支零开销,不回归;显式 `worktree=false` 仍可覆盖,但谓词命中时
  记 model_note 警告。
- **树主权持久化**:第一个在主树落非默认分支的 goal 记为主树 owner,
  持久化进 goal-ledger,重启幸存。此后任何不属 owner goal 的会话在主树
  执行跨分支 `git checkout`/`git switch` 一律**拒绝**并提示"开围栏",
  不静默切换。
- **放行面**:fenced peer 在自己 clone 内 checkout 放行;只读 git 操作
  与 pathspec restore 不拦。

效果:防撞从"外环 steer 盯着"降级为补位手段——系统默认不撞,外环
只在谓词未覆盖的边界人工补位。

### R5 — 指导幂等

外环的意见带日期与唯一编号,只在黑板 `Active` 区可执行;ACK 后移入
历史区且**永不重放**。重复投递以 ACK 为去重依据——同一编号再投递,
已有 ACK 即跳过,不会二次执行。

### R6 — 版本协商

协议文档头部声明 `protocol: olp/vN`,`AGENTS.md` 引用同版本;**信道
语义变更必须升版本**。新 session 首轮复述协议头(金丝雀;系 v0 实验期的可选诊断手段,非 v1 规范义务),
版本不符立即可见,不会静默按旧语义协作。

## 3. goal 状态机与 peer 生命周期
goal 与 peer 是内环长程执行的两个主对象:goal 是任务容器,peer 是被
派出去干活的并行 worker。本节面向贡献者说清两者的状态机、派工时的
车道解析、交付回执契约,以及 goal 状态的存储拓扑(运维排障必读)。
上手操作面(开 goal、派 peer、观察、收口)见指南篇(上),本节不重复。

### 3.1 goal 状态机

goal 有五个状态,语义各别:

| 状态 | 含义 |
|---|---|
| `active` | 活 goal:goal keeper 正在跨 turn 推进,ledger 持续记账 |
| `blocked` | 被阻塞:通常是 escalation 按 R3 分级 park 待裁决(外环或 operator 缺席时保持此态,不强行推进) |
| `paused` | 暂停:operator 显式挂起,账本与现场保留 |
| `complete` | 目标达成并收口,账本保留 |
| `budget_limited` | 预算受限的**独立中间态**——不是失败,是"有名字的、可恢复的现场"(由机制④转入,见下) |

`archived` 是 operator-only 终态(不在上述可设置枚举内,#25 设计):
归档把 goal 移出在途视野,账本仍在、可 reopen 回 active——离线归档
的可靠性坑见 §3.5。

状态转移由 goal keeper 驱动:它是 master 侧跨 turn 的推进者——每个
turn 结束时记账、判断进展、决定下一步,与单个 turn 的生命周期解耦。
keeper 的节拍由两处引擎机制托底,本节只指路不展开:

- turn 之间**零延迟自动续拍**由 turn-continuation 钩子完成
  (见 §4 五项机制⑤)——keeper 不等外环心跳即可推进;
- 预算耗尽时的现场钉存(wip commit + 阶段版 result)与
  `budget_limited` 转态由预算 checkpoint 机制完成
  (见 §4 五项机制④)——`blocked` 与 `budget_limited`
  刻意分立:前者等裁决,后者等续算,处置路径不同。

### 3.2 peer 生命周期

peer 的"状态"分**三层**,先分层再谈映射,勿混用:

1. **handoff 阶段**:`staged` = 派工单落档(brief、车道、围栏决策随单
   持久化)——这是**入库动作**的完成,不是运行态。
2. **后台任务状态机**(引擎侧,运行态真相):
   `running | completed | failed | interrupted | parked`。
   `parked` = serve 重启造成的 peer_handoff 孤儿转入的可恢复态,工作
   现场保留、可续跑(语义与判别标准见 §4 五项机制②,不复述)。
3. **`octos peer list` 目录视图**(CLI 侧展示层):
   `running | done | closed`。

"landed"不是状态,是**动作**:交付被 master 采认、并入主线——peer 的
产出只有经采认才算落地。

| 引擎状态 | CLI 视图 | 含义 |
|---|---|---|
| staged(落档) | running | 待开工(即刻转 running) |
| running | running | 在跑,事件流持续产出 |
| completed | done | 本轮交付完成 |
| failed / interrupted | closed | 真失败/被中断(与重启孤儿刻意区分) |
| parked | closed | 重启孤儿,可恢复续跑 |

观察入口(操作见指南篇(上)):TUI agents 栏、`octos peer list`、
事件流。

### 3.3 peer_handoff 的 model 车道解析语义

`peer_handoff` 的 `model` 参数是 sub_providers 车道 key(模板与选道
标准见协议附录 B),解析按三步:

1. **key 校验**:查 profile 的 sub_providers,命中则按该车道挂载
   模型;
2. **落档**:命中的车道记录**随 brief 一起持久化**进派工单——
   peer 重启恢复(parked 续跑)后仍在原车道,车道不是一次性提示;
3. **未命中 key**:**回退主道**(primary)并记一条 `model_note`
   警告——派发不被拒绝,但车道漂移在板面可见,便于事后审计
   "实际跑在哪条道"。

实战验证见黑板 #33:zai / kimi 双车道各一 peer 全程在线,事件流
`model_lane` 字段可证。

### 3.4 result.md 单写者契约与 frontmatter v1 六字段

peer 的每轮交付只经 `peers/<slug>/result.md` 写回:**单写者契约**——
该文件的权威写者只有 peer 本人,杜绝双写竞争,goal ledger 的
findings 因此干净可审计。其 YAML frontmatter v1 恰为六字段:

`slug` · `outcome` · `updated_unix` · `turn` · `verified` · `protocol`

字段语义、类型与取值域以协议文档附录 A 为**唯一事实源**,本节只列
名单、不复述——消费侧约定(未知字段必须忽略,forward compatibility)
同见附录 A。

### 3.5 goal 状态存储拓扑(运维视角)

goal 状态有三个面,各自的权威角色不同,混淆它们是排障迷路的根源:

| 面 | 角色 |
|---|---|
| supervisor 事件流 | **唯一重启恢复源**:serve 启动只从这里重放、水合内存态 |
| 运行中 serve 内存 `state.goals` | **会落盘的 live cache**:运行期一切 goal 判断与推进直接读写它,并随 `persist_goal_state` 写回事件流 |
| per-goal SQLite ledger(`goal-ledgers/<goal_id>`) | **读面**:`octos goal status` 等查询直接读它,不参与状态裁定 |

**竞争坑(必读)**:serve 存活时,离线 CLI 的归档/重开
(`octos goal archive/reopen`)只向 supervisor 事件流 append,**不能
原子更新或失效存活进程的 live cache**——存活 serve 随后把内存旧态
落盘,会以更新事件**反盖**离线归档(黑板 #34 外环(codex)对抗复审
批注实证,账本行反盖行可见)。因此:

- **离线归档/重开仅在 serve 不在跑时可靠**;
- serve 存活时的收口正解是会话内操作(如 `/goal stop`),操作面见
  指南篇(上)。

> 注:此前"会话记录水合复活"的归因表述**已作废**——源码级结论
> (黑板 #34 codex 批注,主审已采认)是"离线 operator transition 与
> live writer 缺少串行化/同步",与本节拓扑表一致;上游修法方向为
> "serve 存活走在线 RPC / epoch-CAS 防反盖"。

## 4. 五项引擎机制

以下五项是 serve/引擎侧的自治能力(语义与 `docs/OCTOLOOP_FEATURES.md`
32-r1 核准版逐条一致),缺省逐项:**①fallback 需配置 `llm.fallbacks` 才
逐道切换——未配即单道,断供即停(配法见指南篇 §6);②③④⑤ 无需配置、
默认生效**。

### ① 断供自动降级(fallback 车道)

provider 断供(quota 耗尽 / auth 拒付)时,引擎按 profile 的
`llm.fallbacks` 列表**逐道自动切换**,不再全线空转。未配 fallbacks 则
单道(断供即停);配了则逐道切换,恢复后主道自动回归。体感:断供期
对话继续响应而非报错停摆。对双环的意义:内环车道断供不再等于 goal
停摆,降级链路本身也是可观测的实验数据。

### ② 孤儿 peer 恢复态(Parked)

peer 的状态机里,"serve 重启导致 client 绑定丢失"与"真失败"被区分开:
重启造成的 `peer_handoff` 孤儿转为 **Parked 可恢复态**,其余孤儿仍是
真 Failed。体感:serve 重启后 agents 栏出现 `parked · orphaned across
restart` 而非一片 `failed`——工作现场保留,可恢复续跑,不会因为一次
进程重启把并行中的 peer 全部判死。

### ③ malformed 自纠

模型自己产出的**畸形 tool_call 参数**(JSON 残缺、字段名错)不再直接
终结 turn:引擎把诊断作为 tool result 喂回模型,让它自己纠偏重发。上限
**每 turn 3 次**,耗尽才终止;stream 层的不可重试语义不变(网络/流级
错误不重试,这条只治模型级畸形)。体感:畸形参数不直接终结 turn,
模型拿到纠错反馈重发——便宜模型常见的格式抖动被引擎吸收掉大半。

### ④ 预算 checkpoint

单 turn 迭代预算(50 轮)耗尽**且工作树脏**时,引擎自动做两件事:
`wip` commit 把现场钉进 git,再写一份阶段版 result——有 `.result-owner`
在场时写 `result.checkpoint.md`,**不覆盖 peer 终稿**;goal 转入独立的
`budget_limited` 状态(不是 failed)。体感:超时任务的工作不再全丢,
可从 checkpoint 续——预算耗尽从"静默烂在工作区"变成"有名字的、可
恢复的中间态"。

### ⑤ turn-continuation 钩子

活 goal 的 turn 之间**零延迟自动续拍**(引擎特性):上一个 turn 结束、
goal 仍活着,下一个 turn 立刻接上,不等外环唤醒节拍。体感:goal 推进
不再等外环心跳。注意区分:**master-sentry 是外环侧的兜底哨兵**(旗标
开 + 空闲即注入续拍令,3 次无板面进展升级外环并自停),不是引擎机制
——引擎钩子落地后,哨兵降级为兜底。

## 5. `startup --prompt`:恰一次语义

`octoscode --prompt "任务"` 让启动即开工(#30),其语义可以一句话说清:
**引导(onboarding)完成后,恰好自动发一次 turn/start**。

- **恰一次**:派发前若连接中断,重连后会补发那一次;一旦派发成功,
  之后无论发生什么**不重发**——不会重复开工,也不会丢单。
- **引导优先**:首次启动的 onboarding 向导未完成时,任务不抢跑;向导
  收口后才派发。
- **全程可交互**:自动派发的是第一个 turn,之后 TUI 照常交互,任务与
  对话同轨。

对双环的意义:operator 的"唤起即用"零成本——一行命令把任务送进内环,
TUI 留在前台供随时 steer,与心跳自治、黑板指导叠加构成完整的下发路径。

---

> 本篇完。协议全文见 `docs/OUTER_LOOP_PROTOCOL.md`;能力缺省状态与
> 体感速览见 `docs/OCTOLOOP_FEATURES.md`;安装上手见指南篇(上)。
