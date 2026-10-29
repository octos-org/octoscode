---
name: octoloop
description: OctoLoop 一键入口——一个 skill 上手双环全能力。三模式:init(引导铺设脚手架并核依赖)/outer(外环上岗:派单、观测、复验、代推)/inner(内环形态选型:octoscode 标准、免审批窗格、强档车道)
---

# /octoloop — 双环一键入口

OctoLoop = OLP(Outer-Loop Protocol,协议名)之上的产品化封装:用户装
这一个 skill 即可上手双环全能力。三个模式,先选身份再动手;权威规程
在仓库文档,**先读再动**,不要凭记忆操作。

## 模式 init — 铺脚手架(首次/新机器)

引导运行仓库的一键引导脚本,再核对依赖清单:

```bash
bash scripts/olp-init.sh          # 铺脚手架(黑板/信箱/监视器接线)
```

脚本按需询问(不假设环境);完成后逐项核对
`docs/OLP_QUICKSTART.md` §1 环境依赖清单(octos/octoscode 可执行、
herdr 或 tmux、外环模型 CLI)。herdr 来源与分支钉在 §1 依赖表
(hagency-org/herdr,octoscode 识别当前在 feat/octoscode-agent 分支)。任何缺口按 §6 故障速查处理,再跑
§5 冒烟验证(两分钟)。**全部发现式:本卡零硬编码路径**,一切以
QUICKSTART 的发现命令为准。

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
   authority,**守护式死亡耦合**(wrapper 唯一持 fd;agent 经
   PR_SET_PDEATHSIG 与 wrapper 同死,wrapper 亡⇒agent 必亡⇒VACANT;
   Linux-only,非 Linux unsupported,Windows LockFileEx 另立);
   `outer-duty check` 仅观察、绝不夺取;非 holder 只读批注,活锁接管
   只归 operator(终止旧 holder 后再 acquire)。持锁后:署名落板(经
   `scripts/olp-board-append.sh`,flock 原子)→ 立编号条目唤醒内环 →
   内环 ACK 后**隔离 worktree 独立复验** → 采认代推。安全红线见
   BOOT §5。
5. **retro(进化环)**:①触发——战役收官,或进化黑板新卡 ≥ 10 张;
   ②命令——`scripts/olp-evo-harvest.sh <repo> &&
   scripts/olp-evo-retro.sh <repo>`(采集→简报,记录目录在
   `knowledge/context/evolution/`);③处置——每次最多推进 3 条记录;
   立案条件 hint ≥ 2 或主审目视跨 goal/跨条目复发,或 S1;issue 由
   operator 发布或明示委托;④authority——未持 outer-duty 锁只读简报
   不写记录;⑤采集哨只认带署名的行首定式
   `> 外环(<署名>)·改判(作废 #N):` /
   `> 外环(<署名>)·R2 记档(#N):`,纪律里的散文"R2 记档"不落卡。
   ⑥阶段 2/3 工具面(全部只读或只写自家状态目录):监视器
   `scripts/olp-watch-board.sh <板> <token> --harvest <repo>` 命中即采集并常驻
   (不带 `--harvest` 仍一击退出);`scripts/olp-evo-metrics.sh <repo>
   [--since EVO-NNNN] [--json] [--baseline <json>] [--stall <板>
   --stall-threshold <分钟> [--now <ISO>]]` 窗口化诊断(非 KPI:含
   `increase:/decrease:`、`stall:`、`fake_verified:`,不作红线);
   `scripts/olp-evo-spec-skeleton.sh <FLAW-NNN.md>` 从记录直出契约骨架到
   stdout(仓内只许写 `specs/drafts/`,主审补选择器后才入 `specs/`);
   `scripts/olp-evo-index.sh <repo>` 生成 `knowledge/context/evolution/INDEX.md`;
   回放基线 `fixtures/evolution/replay/`(合成夹具,实现 commit 不得改)。

## 模式 inner — 内环形态选型(执行侧)

内环契约 agent 无关(BOOT §6);按任务形态选:

| 形态 | 适用 | 关键点 |
|---|---|---|
| **octoscode 标准** | 仓库内编码主路径 | octos serve stdio 挂载,全工具面 + MCP 第五信道(ask_outer/report_blocked) |
| **claude / codex 免审批窗格** | 快轨修订、外环同级复审 | herdr 窗格隔离,绕内环审批链;分支纪律照旧 |
| **强档车道** | 大型战役/多 peer 并行 | profile `config.llm.primary`/`fallbacks` 多模型 lane(QUICKSTART §3),sub_providers 供 pipeline 按节点选档 |

任何形态都要:黑板 ACK 定式、R4/R4b 工作区共存与树主权、
R2 诚实验证声明(verified/partially/unverified)。

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
   等人。两类永远留给 operator 亲手:免沙箱启动、agent 修改自己的
   权限配置(自我提权,harness 会拦且应该拦)。
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
