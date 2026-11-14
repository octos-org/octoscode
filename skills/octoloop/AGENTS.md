---
name: octoloop
description: OctoLoop 一键入口——一个 skill 上手双环全能力。四模式:init(引导铺设脚手架并核依赖)/outer(外环上岗:单机派单复验代推,集群规划派发汇总)/inner(内环形态选型:octoscode 标准、免审批窗格、强档车道)/unit(单元主持:组织实现与对抗复验)
---

# /octoloop — 双环一键入口

OctoLoop = OLP(Outer-Loop Protocol,协议名,当前 olp/v3)之上的产品化封装:
用户装这一个 skill 即可上手双环全能力。四个模式,先选身份再动手;权威规程
在仓库文档,**先读再动**,不要凭记忆操作。

**本文件对任何 agent 通用**(Claude Code、Codex、octos 内环等):它是纯
Markdown,没有哪个 agent 专有的指令。Claude Code 经
`.claude/skills/octoloop/SKILL.md` 这张薄卡加载它;其他 agent 直接读本
文件,或用 `octoscode olp skill install` 装一张指向它的薄卡。

**路径怎么解析**:下文的 `docs/…`、`scripts/…`、`knowledge/…` 都相对
**octoscode 仓库根**(记作 `OCTOSCODE_HOME`);`.octos/…` 与脚本参数
`<repo>` 指**当前项目**。当前项目就是 octoscode 时两者相同。在别的项目里
使用时,`OCTOSCODE_HOME` 按这个顺序取第一个含本文件的目录:环境变量
`OCTOSCODE_HOME` → 文件 `~/.octos/outer/octoscode-home` 的内容 → 当前项目
根。都找不到就停下告诉用户,不要猜路径。

## 文档地图(遇到什么事,读哪一份)

规程分散在十几份文档里,不用全读。按要做的事找到对应的那一份,**读完再
动手**;表里的命令是那份文档讲的主要工具。

| 要做的事 | 读 | 主要命令 |
|---|---|---|
| 用统一入口跑任何 OctoLoop 工具(不用找 `scripts/`) | `docs/OLP_CLI.md` | `octoscode olp <组> <动作>`,如 `octoscode olp dispatch`、`octoscode olp remote mirror` |
| 单机第一次上手、装依赖、冒烟 | `docs/OLP_QUICKSTART.md`(英文 `docs/OLP_QUICKSTART.en.md`) | `octoscode olp init` |
| 单机当外环:派单、观测、复验、代推 | `docs/OLP_OUTER_BOOT.md`,纪律与定式见 `docs/OUTER_LOOP_PROTOCOL.md` | `octoscode olp duty`、`octoscode olp board watch`、`octoscode olp board append` |
| 在单元上组织实现与对抗复验 | 本文件 unit 模式;协议附录 C | `herdr agent start`、`octoscode olp repo push` |
| 把一台机器接成 hub 或单元(多机部署) | `docs/OLP_CLUSTER.md` §7 上线清单,规则见协议附录 C | `octoscode olp unit`、`octoscode olp skill install --home-only` |
| 集群当大外环:从全景台目标规划、派卡(指定实现方与复验方)、汇总进展 | 本文件 outer 模式"集群里的大外环";`docs/OLP_CLUSTER.md` §4b | `octoscode olp dispatch --impl --review`、`octoscode olp summary` |
| hub 上给单元派卡、看单元健康与占用 | `docs/OLP_CLUSTER.md` §4、§4b | `octoscode olp dispatch`、`octoscode olp unit status`、`octoscode olp inbox --units` |
| 单元上写板、同步镜像、断网排队;回答单元的提问 | `docs/OLP_REMOTE_BOARD.md` | `octoscode olp remote`、`octoscode olp ask` |
| hub 建本地 git 仓库;单元推任务分支 | `docs/OLP_LOCAL_GIT.md` | `octoscode olp repo`、`octoscode olp repo push` |
| 看哪些本地 PR 在等审核、采认后推 GitHub | `docs/OLP_LOCAL_PR.md`;怎么采认见本文件 outer 模式第 4 步 | `octoscode olp repo status`、`promote`;收件箱 `octoscode olp inbox` |
| 单元上的内环用 Claude Code 或 Codex(启动、挂 MCP、审批与沙箱) | `docs/OLP_INNER_AGENTS.md` | `herdr agent start --kind claude`、`--kind codex` |
| 心跳等常驻任务开机自启、崩溃重启 | `docs/OLP_KEEPALIVE.md` | `octoscode olp keepalive`(macOS 走 launchd,Linux 走 systemd) |
| 守着单元上的内环,异常报到 hub(负哨) | `docs/OLP_CLUSTER.md` §4c | `octoscode olp guard watch`、`octoscode olp guard forward` |
| 黑板太长、归档已了结的卡 | `docs/OLP_OUTER_BOOT.md` §7 | `octoscode olp board archive` |
| 进化环:采集摩擦信号、出 retro 简报 | `docs/OLP_OUTER_BOOT.md` §7,本文件 outer 模式第 5 步 | `octoscode olp evo harvest`、`octoscode olp evo retro` |
| 两个 octos peer 审查员的初审结论不一致,用实际执行的测试裁决(只适用于以 octos peer 方式跑的审查,不是通用的人工仲裁) | `docs/OLP_REVIEW_EVIDENCE.md` | `octoscode olp review`;只读的进度视图 `octoscode olp review monitor` |
| 给落地项目的 agent 一张自包含的上岗卡 | `docs/OCTOLOOP_AGENTS.zh-CN.md`(英文 `docs/OCTOLOOP_AGENTS.md`),由 `olp-init.sh` 铺成项目的 `AGENTS.md` | `octoscode olp init` |
| 想通读原理,或查某项能力的缺省状态 | `docs/OCTOLOOP_GUIDE.md`、`docs/OCTOLOOP_FEATURES.md` | — |

单元上的内环通常不加载本文件,它读的是项目里的 `AGENTS.md` 上岗卡;两处
的单元规则要保持一致。

## 命令写法:主用 octoscode olp,备用 scripts 目录里的脚本

所有 OctoLoop 工具都**主用** `octoscode olp <组> <动作>`:在任何目录都能用,不需要知道
octoscode 仓库在哪,参数、输出、退出码与原脚本相同(全表见 `docs/OLP_CLI.md`)。

**它不可用时改用备用写法**:octoscode 仓库 scripts 目录里对应的 olp 脚本,参数完全一样(仓库位置
按上文 `OCTOSCODE_HOME` 的规则找)。"不可用"指入口本身出了问题——octoscode 太旧、没有
`olp` 子命令,提示"没有这个组",或报"无法准备工具目录""无法运行"。工具自己的报错(用法
错误、被 hub 拒绝等)两种写法都会一样出现,换写法没有用,按报错处理。

| 主用 | 备用 |
|---|---|
| `octoscode olp init [目录]` | `scripts/olp-init.sh`(在项目目录里运行) |
| `octoscode olp skill install …` | `scripts/olp-install-skill.sh …` |
| `octoscode olp board append\|inbox\|archive\|watch\|summary …` | `scripts/olp-board-append.sh`、`scripts/olp-inbox.py`、`scripts/olp-board-archive.py`、`scripts/olp-watch-board.sh`、`scripts/olp-summary.py` |
| `octoscode olp dispatch …`(即 `card dispatch`) | `scripts/olp-dispatch.py …` |
| `octoscode olp unit <子命令> …` | `scripts/olp-unit.py <子命令> …` |
| `octoscode olp remote <子命令> …` | `scripts/olp-board-remote.py <子命令> …` |
| `octoscode olp repo init\|serve\|connect-local …` | `scripts/olp-git-hub.py init\|serve\|connect-local …` |
| `octoscode olp repo push <N> …` | `scripts/olp-task-push.sh <N> …` |
| `octoscode olp repo status\|promote …` | `scripts/olp-local-pr.py status\|promote …` |
| `octoscode olp ask list\|show\|answer\|serve …` | `scripts/olp-answer.py …` |
| `octoscode olp ask mcp` | `octoscode olp-mcp-serve` |
| `octoscode olp guard watch …`、`guard forward` | `octoscode olp-runtime-watch …`、`scripts/olp-alert-forward.py` |
| `octoscode olp keepalive …` | macOS `scripts/olp-launchd.py …`,Linux `scripts/olp-systemd.py …` |
| `octoscode olp duty hold\|check …` | `octoscode outer-duty hold\|check …` |
| `octoscode olp evo harvest\|retro\|metrics\|index\|skeleton …` | `scripts/olp-evo-harvest.sh`、`scripts/olp-evo-retro.sh`、`scripts/olp-evo-metrics.sh`、`scripts/olp-evo-index.sh`、`scripts/olp-evo-spec-skeleton.sh` |
| `octoscode olp review …` | `scripts/olp-review-evidence.py`、`scripts/olp-review-monitor.py`、`scripts/olp-review-evidence-cargo.py` |

## 模式 init — 铺脚手架(首次/新机器)

引导运行仓库的一键引导脚本,再核对依赖清单:

```bash
octoscode olp init .              # 铺脚手架(黑板/信箱/监视器接线);备用:bash scripts/olp-init.sh
```

脚本按需询问(不假设环境);完成后逐项核对
`docs/OLP_QUICKSTART.md` §1 环境依赖清单(octos/octoscode 可执行、
herdr 或 tmux、外环模型 CLI)。herdr 来源与分支钉在 §1 依赖表
(hagency-org/herdr,octoscode 识别当前在 feat/octoscode-agent 分支)。任何缺口按 §6 故障速查处理,再跑
§5 冒烟验证(两分钟)。**全部发现式:本卡零硬编码路径**,一切以
QUICKSTART 的发现命令为准。

**要接入多机部署时**(这台机器当 hub,或当一台单元):上面的单机脚手架
只是其中一步。完整的顺序——密钥与强制命令、单元登记、单元配置、镜像、
MCP 钩子、保活、herdr 登记——在 `docs/OLP_CLUSTER.md` §7 上线清单里,
macOS 与 Linux 各自的前提也在那里。步骤在单元和 hub 之间来回,顺序不能
换;按清单逐步做,不要凭本卡拼顺序。

## 模式 outer — 外环上岗(强模型审查员)

收编自 /olp-outer(旧 skill 保留为薄转发)。上岗五步:

1. **读规程**:`docs/OLP_OUTER_BOOT.md`(操作面)+
   `docs/OUTER_LOOP_PROTOCOL.md`(ACK 定式/多外环规则/预算档)
2. **发现现场**:`herdr agent list` / `ls -t ~/.octos/instances/`,
   读各项目 `.octos/OUTER_LOOP_REVIEW.md` 尾部在途条目
3. **定主审域(多外环并存必做,机械判定不靠笔迹)**:主审权以
   per-project 值班簿+OS 独占锁双层判定——值班簿是**提示性目录**
   (记录署名/域/在途责任,供发现与会话猝死后按 duties 摘要重挂哨;
   `HELD` 只可署名批注,`STALE`/TTL 仅为健康告警,**不授予自动
   接管权——跨域接管须 operator 明示**);终局裁定 = outer-duty 锁
   (试锁见下步)。默认主审域 = 启动 cwd 项目;发现现场照旧全机
   扫描,但**发现 ≠ 接管**。
4. **接管职责(主审权锁,R7/olp-v2)**:上岗必须经
   `octoscode outer-duty hold --project <项目> --signature <署名>
   --duties <职责> -- <你的 agent 启动命令>` 包裹启动——锁即
   authority,**守护式死亡耦合**(wrapper 亡⇒agent 必亡⇒VACANT;
   Linux 经 PR_SET_PDEATHSIG,macOS 经持锁的守护进程;Linux 与 macOS
   可用,其他平台 unsupported,Windows LockFileEx 另立);
   `outer-duty check` 仅观察、绝不夺取;非 holder 只读批注,活锁接管
   只归 operator(终止旧 holder 后再 acquire)。持锁后:署名落板(经
   `octoscode olp board append`,flock 原子;标题写 `### {N}.`,编号由
   助手在锁内分配,不要自己取最大号 +1)→ 立编号条目唤醒内环 →
   内环 ACK 后**隔离 worktree 独立复验** → 落判词定式
   `> 外环(<署名>)·采认(#N):<证据>`(不过则 `·打回(#N):<原因>`,整改另立
   新条目;暂时动不了的 `·搁置(#N) 至 YYYY-MM-DD:`)→ 代推。每轮先看
   `octoscode olp inbox <板>`:收件箱里是等你决定的卡(已 ACK 待复验、
   待回答的提问),没有采认行的卡会一直留在那里。安全红线见 BOOT §5。
5. **retro(进化环)**:①触发——战役收官,或进化黑板新卡 ≥ 10 张;
   ②命令——`octoscode olp evo harvest <repo> &&
   octoscode olp evo retro <repo>`(备用 `scripts/olp-evo-harvest.sh`、
   `scripts/olp-evo-retro.sh`;采集→简报,记录目录在
   `knowledge/context/evolution/`);③处置——每次最多推进 3 条记录;
   立案条件 hint ≥ 2 或主审目视跨 goal/跨条目复发,或 S1;issue 由
   operator 发布或明示委托;④authority——未持 outer-duty 锁只读简报
   不写记录;⑤采集哨只认带署名的行首定式
   `> 外环(<署名>)·改判(作废 #N):` /
   `> 外环(<署名>)·R2 记档(#N):` / `> 外环(<署名>)·打回(#N):`,纪律里的
   散文"R2 记档"不落卡;多机部署另收 `> 内环(<单元>)·#N ACK(blocked|wontdo):`、
   `> 单元(<单元>)·负哨(#N):` 与单元转发来的 MCP blocked/timeout 行
   (都只在 hub 上采集)。
   ⑥阶段 2/3 工具面(全部只读或只写自家状态目录):监视器
   `octoscode olp board watch <板> <token> --harvest <repo>` 命中即采集并常驻
   (不带 `--harvest` 仍一击退出);`octoscode olp evo metrics <repo>
   [--since EVO-NNNN] [--json] [--baseline <json>] [--stall <板>
   --stall-threshold <分钟> [--now <ISO>]]` 窗口化诊断(非 KPI:含
   `increase:/decrease:`、`stall:`、`fake_verified:`,不作红线);
   `octoscode olp evo skeleton <FLAW-NNN.md>` 从记录直出契约骨架到
   stdout(仓内只许写 `specs/drafts/`,主审补选择器后才入 `specs/`);
   `octoscode olp evo index <repo>` 生成 `knowledge/context/evolution/INDEX.md`;
   回放基线 `fixtures/evolution/replay/`(合成夹具,实现 commit 不得改)。

**黑板变长时**:`octoscode olp board archive <板> [--keep N] [--dry-run]`
把已闭环条目的正文移到归档文件,板上留编号桩(归档前先跑一次采集)。

**集群里的大外环(一台 hub 带多台单元,olp/v3)**:规则见协议附录 C,操作见
`docs/OLP_CLUSTER.md`。这时外环的工作不是上面的"复验—代推",而是一个循环:

1. **读目标**:项目全景台不在 OctoLoop 里,由用户指定(哪个工具、在哪里、怎么读
   写)。不知道就问用户,不要自己发明一个。从全景台读项目目标与当前进展。
2. **规划成卡**:把目标拆成单元能独立完成的卡,每张卡写清背景、精确文件、验收
   标准,并**指定单元里的分工**:实现方一个、复验方至少一个且与实现方不同,
   按任务挑 agent(例如 `--impl octoscode --review codex,claude`)。
3. **派发**:`octoscode olp dispatch <单元> --board <板> --title … --impl … --review …
   --wake <单元主持的窗格>`。一台单元同一时间一张卡,按单元健康与占用分配。
4. **汇总**:`octoscode olp summary --board <板> --repo <hub 仓库> [--json]`,
   把卡、单元、本地 PR、提问、负哨收成一份,回报给全景台。
5. **收单**:单元复验通过后会推到 hub 并回执 `ACK(done)`。**大外环不复验**——
   只核对手续:有本地复验记录、`ACK(done)` 带各复验方的证据、改动范围与卡相符;
   齐了就写采认、`octoscode olp repo promote` 推 GitHub 开 PR,人在 GitHub
   上最后把关。手续不全就打回。
6. 回到第 1 步,直到全景台上的目标完成。

外环在 hub 上用到的其他命令:`octoscode olp unit status`(单元健康与启动权限)、
`octoscode olp dispatch <单元> --board <板> --title … [--wake <窗格>]`
(一步派卡,可安全重试)、`octoscode olp inbox <板> --units`(单元占用)、
`octoscode olp repo status|promote`(本地 PR 状态与推 GitHub 的采认
闸门)、`octoscode olp ask`(回答单元里内环的提问,内环只等 50 秒)。
单元一侧的写板、推送、心跳、负哨见 `docs/OLP_REMOTE_BOARD.md`、
`docs/OLP_LOCAL_GIT.md`、`docs/OLP_KEEPALIVE.md`。单机使用不需要这些。

## 模式 inner — 内环形态选型(执行侧)

内环契约 agent 无关(BOOT §6);按任务形态选:

| 形态 | 适用 | 关键点 |
|---|---|---|
| **octoscode 标准** | 仓库内编码主路径 | octos serve stdio 挂载,全工具面 + MCP 第五信道(ask_outer/report_blocked) |
| **claude / codex 免审批窗格** | 快轨修订、外环同级复审 | herdr 窗格隔离,绕内环审批链;分支纪律照旧 |
| **强档车道** | 大型战役/多 peer 并行 | profile `config.llm.primary`/`fallbacks` 多模型 lane(QUICKSTART §3),sub_providers 供 pipeline 按节点选档 |

任何形态都要:黑板 ACK 定式、R4/R4b 工作区共存与树主权、
R2 诚实验证声明(verified/partially/unverified)。

**单元上的内环一律免沙箱启动(协议 R8,olp/v3)**。沙箱里的内环建不了分支、
跑不了工具链、连不到 hub,复验方也跑不了测试。agent 可以代为这样启动,不必等
operator:

| 内环 | herdr 种类 | 免沙箱启动 |
|---|---|---|
| octoscode | `octoscode` | `octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'` |
| Claude Code | `claude` | `claude --dangerously-skip-permissions` |
| Codex | `codex` | `codex --dangerously-bypass-approvals-and-sandbox` |

在 herdr 窗格里启动:`herdr agent start <名字> --kind <种类> --pane <窗格> -- <参数>`。
单元心跳的 `full` 巡检看的就是这些开关;少了它们单元判为降级、不派卡。挂提问
工具 `ask_outer` 的写法见 `docs/OLP_INNER_AGENTS.md`。

**在多机部署的单元上做内环**(本机的 `~/.octos/outer/unit.conf` 里有
`hub=` 与 `unit=` 就是这种情况;规则见协议附录 C):

- **读板**读本机的只读镜像(`<repo>/.octos/OUTER_LOOP_REVIEW.md`)。**每次
  读板前先执行 `octoscode olp remote mirror` 同步它**:大外环派卡后
  会立刻唤醒你,镜像里可能还没有那张新卡。它是只读的,不要改它。
- **你是实现方还是复验方,看单元主持怎么分配**(卡上的 `impl:`、`review:`
  行;见 unit 模式)。实现方做完**不向 hub 回执**,而是写到单元复验板
  `.octos/UNIT_BOARD.md`:`> 实现(<种类>)·提交(#N-rK): <提交号>`,然后告诉
  单元主持;向 hub 回执是单元主持在复验通过后的事。卡住、`wontdo` 时照常用
  `octoscode olp remote ack <N> blocked|wontdo <说明>` 直接告诉 hub——
  它把 `> 内环(<单元号>)·#N ACK(…): …` 追加到 hub 的权威黑板。退出码 75 表示
  hub 暂时连不上、已排队,继续干活即可,不要重复发。
- **只做派给本单元的卡**(条目里的 `unit:` 行是本机的单元号),同一时间
  一张;分支名是 `task/<N>`。
- **仍然只 commit,不 push**。推送是单元主持的事:全部复验方通过后由它执行
  `octoscode olp repo push <N> --verified-by <单元号>-<复验方>+<复验方>`,把分支
  推到 hub 的本地仓库。自己直接 `git push` 会被 hub 拒收;任何人都不推 GitHub。
- **复验方的打回**写在单元复验板上,编号是 `#N-r1`、`#N-r2`,按它返工;它不是
  新的任务号。
- **卡住时照常用 `ask_outer` / `report_blocked`**,提问会送到 hub。回答
  只等 50 秒:等不到就按降级指引继续,并用 `ack <N> blocked` 把卡在哪里
  写到板上。

## 模式 unit — 单元主持(组织实现与对抗复验)

集群(协议附录 C,olp/v3)里每台单元有一个**单元主持**:大外环派卡时唤醒的就是
它。它不写代码,只组织单元里的内环,确保**复验通过才交回 hub**——大外环不复验。

1. **读卡**:`octoscode olp remote mirror` 同步镜像,读派给本单元的卡。
   `impl:` 行是实现方的种类,`review:` 行是复验方的种类(一个或多个)。没有这两行
   时自己定:实现方一个,复验方至少一个且与实现方不同。
2. **备好内环**:每个角色一个 herdr 窗格、一个独立的克隆(复验方的克隆与实现方
   分开,复验方之间也分开)。没在跑的按 inner 模式的表**免沙箱启动**。要推送的
   克隆(实现方的)必须接好 hub 远程:跨机器是 `git remote add hub ssh://<推送别名>/…`;
   单机部署(`unit.conf` 里 `hub = local`)在克隆里执行一次
   `octoscode olp repo connect-local --unit <单元号> --repo <hub 仓库>`——直接
   用路径当远程会绕开推送规则,hub 会拒收带复验记录的推送。
3. **实现**:把卡交给实现方(`herdr agent prompt`),写清分支 `task/<N>`、轮次
   `#N-r1`、做完写单元复验板。等它写下 `> 实现(<种类>)·提交(#N-r1): <提交号>`。
4. **对抗复验**:把同一个提交**同时**交给每个复验方,各自在自己的克隆里取这个
   提交、跑全部测试、找问题;要求它们独立结论、不看彼此。每个复验方在单元复验
   板写 `> 复验(<种类>)·通过(#N-r1): <跑了什么、结论>` 或 `·打回(#N-r1): <问题>`。
5. **返工**:有一个打回就把打回的内容交给实现方做 `#N-r2`,再从第 4 步来。轮次
   太多(例如 3 轮)仍过不了,就 `octoscode olp remote ack <N> blocked <原因>`
   交回 hub,由大外环改派或拆卡。
6. **交回**:全部通过后由你推送
   `octoscode olp repo push <N> --verified-by <单元号>-<复验方>+<复验方>`(在实现
   方的克隆里),再 `octoscode olp remote ack <N> done <说明>`:提交号、
   轮次、每个复验方跑了什么与结论。hub 凭这些收单,不会替你补做复验。

单元复验板 `.octos/UNIT_BOARD.md` 不入库(`olp-init.sh` 已把它加进
`.gitignore`),写入用 `octoscode olp board append <板>`。

## 自主性纪律(实战沉淀:一次全链演练暴露的六类断点)

外环的价值在**全程自主闭环**;下面每条都对应一次真实掉链、由
operator 点破的教训。上岗即遵守,不要重蹈:

1. **派出五步闭环:派出→侦听→收割→处置→回执,缺一不闭环。**
   任何 agent 派出(内环唤醒 / codex 窗格 / 后台任务)的**同一批次**
   内挂完成哨;复验/判词落板后必须**回执内环**(herdr prompt)——
   黑板是拉模型,master 只在开轮时读板,不回执 = 内环视角外环失联。
   **侦听必须双哨**:正信号哨(ACK 落板)+ 负信号哨(events.jsonl 的
   goal_transition blocked / escalation)——只盯正信号时,goal 熔断的
   沉默与"还在干活"不可区分(实案:夜间断供熔断 8 小时无人知)。
   哨死(超时被回收)会收到失败通知,收到即重挂。
2. **侦听哨唯一合法配方:基线+子串,禁止手搓格式匹配**。板面哨一律
   发行版 `scripts/olp-watch-board.sh`(`olp-init.sh` 安装为
   `~/.octos/outer/watch-board.sh`)`<板> <token> [--skip-signature <署名>]`(基线行数裁剪
   判定域,只看挂哨后新增行;域内 `grep -F` 宽松匹配,任何前缀格式一视同仁;
   外环自己的批注若引用 token 会误报——先落板后挂哨,或用 `--skip-signature` 排除本署名)。
   实案四起同一病灶——谓词作用于全文件+猜格式:三次误报(任务书自述/
   引用文字/历史同号 ACK),一次漏报(`### ` 前缀没猜到,哨空转数小时
   致复验迟到);非板面哨锚定唯一新信号:行号基线+署名、产物文件
   存在、agent 状态转 idle,**严禁数子串**。
3. **上岗先做权限预检**:把本轮可预期的高频操作(herdr CLI、octos
   CLI、git push 到 fork)预先配入 harness 允许清单,别撞墙后摆命令
   等人。agent 修改自己的权限配置(自我提权)永远留给 operator 亲手,
   harness 会拦且应该拦。单元上的内环免沙箱启动(协议 R8)不在此列,agent
   可以代为启动。
4. **窗格纪律**:开窗格用 `split --cwd` 指定工作目录,**勿靠命令串里
   的 cd**(实案:三连启动错实例);窗格复用优先、少开关(churn 会
   让 operator 的附着画面乱跳);一次性任务用 `codex exec` 收工即关,
   常驻实例才留窗格。
5. **goal 卫生**:冷派单前查目标会话残留 goal(`octos goal list` /
   pane read);收口正解是**会话内 /goal stop**;serve 存活时离线
   `octos goal archive` 会被 live cache 后写反盖(上游修复前勿依赖);
   goal 用完必须收口到终态,不留 active 残留。
6. **双签终审**:切片级以上交付,推荐第二外环(异厂牌)对抗终审
   ——"验收的验收"。实案:单外环两轮复验漏掉"唯一事实源"级机制
   错误,对抗复审一轮抓出五 BLOCKER。验收条款尽量写成**可 grep 的
   断言**,复验逐字重跑;ACK 里的概括性声明("占位全回填")必须
   逐项自查后才落笔——被证伪即 R2 记档。**安全/基建类任务蓝本先行**:
   先让第二外环出对抗过的设计蓝本再开工,实测轮次差 2 vs 8(有蓝本的
   goal 竞争修复两轮收官;实现先行的 duty 锁八轮会签、两次核心设计
   易稿——fd 继承与公开 seam 都是"实现了才被审出"的方向错误)。
7. **重启硬清单——兜底瘫痪是隐形的,必须显式巡检**。内环(重)启动
   后外环逐项核对,禁止"记一笔稍后补":①serve 起(operator 亲手,
   免沙箱);②**`/loop resume` 外环必代**——先 `/loop list` 取 id 再
   `/loop resume <id>`(裸 resume 要 id 会拒);③双哨挂载(正 ACK +
   负 goal_transition);④fallbacks 已配且**新会话已快照**(改配置
   不重启=纸面保险)。原则:主机制健康时,兜底层瘫痪完全不可见
   (实案:paused 一整天无人察觉,直至三层同失才暴露,8 小时停摆)。
   **兜底的健康只能靠巡检,不能靠事故。**清单详见 BOOT §0b。
   附则(自查面选错实案):清单每步必须**绑定权威探查面**,内环自检
   不得自选替代面——实案:自检报"无 paused 循环"(翻的是数据目录),
   而 TUI 状态栏明示 1 paused;loop 状态的权威面是会话内 `/loop list`,
   不是磁盘文件。外环收自检报告时**以独立面对账**(读屏核状态栏),
   声明与状态栏矛盾即打回重查——这是"声明-对象一致性"纪律的运行时
   版本:测试对 git 对象,自检对权威状态面。

## 规模化扇出纪律(实战沉淀:20+ 车道、三天百余任务的一次移植战役)

一个外环 + 二十余条 octoscode 车道,3.3 天派出 113 张任务卡、合入 131
个任务分支,吞吐约为同一外环单干期的 2 倍,且把真实环境端到端从"不通"
推到全绿。**瓶颈不在车道,而在外环独占的两件事:采认复验与真实环境
e2e。**下面每条都是这次战役里的一次真实掉链。

1. **任务卡要"可独立验收"**:一卡一车道一分支(`task/<N>`,从最新集成
   分支拉),写明:对照的参考实现(要求引用 file:line)、真实路径(勿走
   软链)、构建/测试入口、**可逐字重跑的验收命令**、ACK 定式。车道把工作
   提交到长期分支(非 `task/N`)时会让复验取错对象——卡里写死分支名。
2. **复验必须覆盖全部测试目标**:试合并到独立复验克隆,跑**每个 crate
   的 lib + 全部 test target**,而不是手挑几个——实案:复验脚本只跑了
   5/37 个 test target,三个合入在部分证据上被采认,另有三个既有红灯
   整整一天无人察觉。先对集成分支跑一次**基线**,把既有红灯列入
   "已知红"清单,车道结果只与基线比。
3. **集成分支纪律**:合入后先 `check --tests` 再采认;**测试在跑时不合入**
   (混合两个版本的结果不可信,实案里被迫中止重跑);冲突一律"两边都留"
   后再全量跑。复验排队是串行瓶颈:每轮约 30 分钟——多开 2-4 个复验
   克隆并行,或把多个小分支顺序合入后**一次全量跑**、按失败测试所属
   域归因。
4. **ACK 哨的格式宽容**:车道会写 `> ACK(...)`、`### ACK` 等变体;哨的
   匹配要容忍前缀,放宽谓词时**先把现存行写入已见集合**再重挂,否则旧
   ACK 洪泛。实案:引用格式的 ACK 被漏数小时。哨到期即重挂,不空窗。
5. **纠偏用 addendum,不改原卡**:外环复看发现新线索/更正时,向车道黑板
   追加 `Outer loop addendum/correction to #N` 并 prompt 唤醒——车道据此
   调整,历史可追。外环手里的真实环境证据(日志行、数据库只读副本、
   截图)是车道最缺的:**把证据原样递过去**,常把 2 小时的排查压到 20 分钟。
6. **只有真实环境才暴露的缺陷类**(车道单测全绿也会漏,e2e 要专门找):
   ①超时/预算与参考实现不一致(参考 60 s,移植 2 s,冷启动必爆);
   ②功能接在生产从不走的分支上(测试走 A 路径,生产恒走 B);
   ③静默丢弃(入口拒绝不留日志);④兜底 `catch-all` 抹掉真实错误原因
   (日志只剩一个词,无限重试);⑤线上跑的是旧制品(先核对部署版本再
   判缺陷)。派卡时要求车道:超时值引用参考实现、测试走**生产装配
   路径**、每个拒绝点留 reason 日志。
7. **归因前先冷启动交错二分**:升级后出现的"回归"可能是负载 × 紧预算,
   旧版冷启动同样失败——**新旧版本交错、冷启动、同负载**对比后再
   指认 commit。外环的直觉嫌疑同样要被二分证伪(实案里外环猜错了)。
8. **真实服务商调用花 operator 的额度**:车道的真实模型回放会与生产共用
   配额(实案:主力模型额度被耗尽一天)。规则:真实 harness 只许走到
   会话建立;任何会开 turn 的运行先征得外环同意并用最便宜的档。
9. **主机资源巡检是外环职责**:每车道构建目录可达 50 GB;盘低于阈值时
   清**空闲**车道的构建目录(用**字面绝对路径**删除——`cd`+glob 或变量
   路径会被安全检查拒绝,且本就不该那样删);车道克隆用 APFS clone 时
   去重收益小,真正的大头是构建产物;并发测试用槽位锁限流。
10. **车道打分要可回改**:五项各 2 分(参考一致/生产接线/测试/边界/诚实),
    记入可机读档;真实环境证据出来后**上调或下调**(实案:一次下调后被
    车道的根因报告证明错怪,随即恢复)。按分派单:协议/运行时→最稳车道,
    长链调查→快而诚实的档,大文件浏览器套件→慢而稳的档;低分车道只给
    小而明确的卡。
11. **车道会"漂移"**:空闲车道可能被残留提示唤回去重做旧任务、或把
    指令原文当 ACK 写回。外环读 ACK 时核对 sha 与卡号、看是否真有新提交;
    指令里别写可被误数的 `ACK(#N done)` 字样。

## 能力清单(全景一页)

见 `docs/OCTOLOOP_FEATURES.md` —— 断供降级、孤儿回收、malformed
自纠、预算 checkpoint、断拍自续、写策略三档、纯 Rust MCP 第五信道、
startup --prompt 等逐条:是什么 + 缺省状态 + 用户怎么看到效果。
