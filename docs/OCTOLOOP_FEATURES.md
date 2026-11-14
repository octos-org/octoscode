# OctoLoop 功能清单(一页全景)

> OctoLoop = OLP(Outer-Loop Protocol,协议)的产品化封装。本页列出
> 双环系统近期落地的全部能力:是什么 + 缺省状态 + 用户怎么看到效果。
> 深入规程:OUTER_LOOP_PROTOCOL.md(纪律)/ OLP_OUTER_BOOT.md(外环
> 操作面)/ OLP_QUICKSTART.md(上手)。

## 稳性与自治(引擎侧)

- **断供自动降级(fallback 车道)**:provider 断供(quota/auth 拒付)
  时引擎按 profile `llm.fallbacks` 自动换道,不再全线空转。缺省:
  未配 fallbacks 则单道;配了则逐道自动切换。体感:断供期对话继续
  响应而非报错停摆,恢复后主道自动回归。
- **孤儿 peer 恢复态(Parked)**:serve **重启**导致 client 绑定丢失的
  peer_handoff 孤儿转为 Parked 可恢复态(其余孤儿仍真 Failed)。
  缺省:开。体感:重启后 agents 栏出现 parked·orphaned across
  restart 而非 failed。
- **malformed 自纠**:模型自己产出的畸形 tool_call 参数,诊断作为
  tool result 喂回模型自纠,限每 turn 3 次,耗尽才终止(stream 层
  不可重试语义不变)。缺省:开。体感:畸形参数不直接终结 turn,
  模型拿到纠错反馈重发。
- **预算 checkpoint**:50 轮迭代耗尽且工作树脏时,自动 wip commit +
  阶段版 result(有 .result-owner 时写 result.checkpoint.md,不覆盖
  peer 终稿),goal 转 budget_limited 独立状态。缺省:开。体感:
  超时任务的工作不再全丢,可从 checkpoint 续。
- **turn-continuation 钩子**:活 goal 的 turn 之间零延迟自动续拍
  (引擎特性)。缺省:开。体感:goal 推进不再等外环唤醒节拍。
  (备注:master-sentry 是外环侧的兜底哨兵,非引擎机制。)

## 结果与审计(交付侧)

- **result.md 单写者契约**:peer 交付只经 result.md frontmatter
  (slug/outcome/updated_unix/turn)写回,杜绝双写竞争。缺省:开。
  体感:goal ledger 的 findings 干净、可审计。
- **文件变更回执 + scope**:每轮交付列变更文件与影响面,复验按图
  索骥。缺省:master 复验必查。体感:R2 复验命令可直接对着回执跑。
- **写策略三档 + 逃生口**:workspace 只读/受写/host 三档 + 显式逃生
  口,越权写被拒。缺省:workspace。体感:误写仓库外文件直接报错。
- **黑板收件箱**:`scripts/olp-inbox.py <板>` 从黑板算出每张卡等谁——
  收件箱(等外环决定)、进行中(等内环 ACK)、搁置、已了结;状态不存进
  黑板。配套三个判词定式:采认、打回、搁置。缺省状态:手动运行。用户
  怎么看到:外环每轮先看收件箱,不必从头扫板;没有采认行的卡留在收件箱。
- **写板分配编号与归档**:`olp-board-append.sh` 在锁内给 `### {N}.` 分配
  编号,`--key` 让重发不重复;`olp-board-archive.py` 把已闭环条目移到
  归档文件并留编号桩。缺省状态:旧写法(自己写编号)仍然接受。用户怎么
  看到:多个写入方不再撞号;长板变短而 `#N` 引用仍可查。
- **多机部署(hub + 单元)**:一台 hub 放黑板与本地 git 仓库、跑持锁的
  大外环,多台单元各跑一个内环。单元登记与心跳、一步派卡、单元只能推
  带复验记录的任务分支、远程写板与离线队列、提问往返、运行时守护与
  负哨、推 GitHub 前的采认闸门。单元与 hub 可以是 macOS 或 Linux:
  常驻任务的保活在 macOS 上用 `olp-launchd.py`,在 Linux 上用
  `olp-systemd.py`(systemd 用户单元,需要 linger)。缺省状态:不配置就
  不生效,单机行为不变。用户怎么看到:见 `docs/OLP_CLUSTER.md`(§7 是
  单元上线清单);协议附录 C 是规则。
- **进化环(阶段 0–3)**:三源采集哨(`olp-evo-harvest.sh` 从活板
  ACK、events.jsonl、MCP 审计板增量识别内环摩擦,identity 去重、
  字节游标、崩溃一致)落症状卡,retro 脚本(`olp-evo-retro.sh`)按
  归一化文本分组出候选简报与记录草稿,供持锁外环归档为 FLAW 记录。
  缺省状态:手动运行。用户怎么看到:`.octos/EVOLUTION.md` 与 retro
  简报(状态目录 `retro/` 下)。外环私有工作纸,不写入 OLP 信道矩阵,
  不升协议版本。阶段 2/3 增补:共享 `olp-evo-lib.py`、合成回放基线
  `fixtures/evolution/replay/`、窗口化诊断 `olp-evo-metrics.sh`(含停摆与
  伪 verified 行,非 KPI)、监视器 `--harvest` 常驻采集、FLAW 直出契约骨架
  `olp-evo-spec-skeleton.sh`、索引 `olp-evo-index.sh`;octos 侧新增
  `fallback_switch`/`malformed_exhausted` 事件(PR #2257)。
  多机部署:采集与 retro 只在 hub 上运行——配置了 hub 主机名
  (`OLP_HUB_HOST`,或 `~/.octos/outer/hub-host` 文件)后,其他机器上
  的非 dry-run 调用在任何写入前以退出码 3 拒绝;未配置时为单机,行为不变。

## 通信与信道

- **纯 Rust MCP 第五信道**(#31):`octoscode olp-mcp-serve` 子命令,
  turn 内同步问外环(ask_outer/report_blocked 两工具、50s 超时降级、
  每进程限 3 次 + tried 必填、board 审计、取答归档)。缺省:profile
  mcp_servers 挂载即开。体感:内环遇分歧 50s 内拿到外环人工作答,
  超时得降级指引不卡 turn。
- **startup --prompt**(#30):Omarchy 默认 Agent 唤起即用;引导完成
  后恰好一次自动 turn/start(派发前重连补发/派发后不重发)。
  缺省:CLI 旗标。体感:`octoscode --prompt "任务"` 启动即开工,
  TUI 全程可交互。

## 命名

- **OLP** = 协议名(Outer-Loop Protocol,文档与 ACK 定式沿用)。
- **OctoLoop** = 产品名(用户视角的一键化封装;skill 入口与本页)。
