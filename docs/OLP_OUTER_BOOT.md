# OLP 外环上岗卡(Outer Boot Card)

任何强模型 agent(Claude Code / Codex / 其他 CLI agent)把本文读完即可以
**外环**身份接入双环系统:派单、唤醒、观测、复验、代推。本卡只含操作面;
角色语义与完整纪律见 [OUTER_LOOP_PROTOCOL.md](OUTER_LOOP_PROTOCOL.md)。

> 原则:环境细节(窗格号、实例哈希、会话键)会漂移——本卡教**发现方法**,
> 不硬编码任何具体值。

## 0. 身份与署名

- 选定署名:`外环(<你的名字>)`,如 `外环(codex)`。所有黑板写入必须署名。
- 多外环并存:每条目**单一主审**;他人条目只可署名批注(陈述意见,不打回
  不改写);分歧升级 operator,`wontdo` 只能接受或上报。

## 1. 黑板(权威账本)

- 每个仓库一块:`<repo>/.octos/OUTER_LOOP_REVIEW.md`。`docs/` 下同名文件
  是冻结快照,**严禁写入**(tracked、随分支变化,写了会被 checkout 冲掉)。
- **写入必须走原子追加助手**(flock 互斥 + 自写登记):
  ```bash
  scripts/olp-board-append.sh <板路径>       # 正文从 stdin 喂(仓库发行版;本机自建版亦可)
  ```
- 编号:先 `grep -oE '^### [0-9]+' <板> | tail -1` 取当前最大号,+1 使用。
- 条目自包含:背景、精确文件/行号、修法方向、验收标准、分支名(基于
  main)、预算档(修订 5-10M / 切片 10-20M / 战役 30-50M),并写明
  "只 commit 不 push,主审复验后代推"。

## 2. 唤醒与纠偏(下行)

```bash
herdr agent list                      # 发现内环窗格(octoscode | <pane> | 状态)
herdr agent prompt <pane> '<一句话>'   # 空闲时唤醒:指向黑板新条目编号
```
master 正在跑 turn 时,用 steer 插话(不打断动作,下一拍被消费):
```bash
cd <目标仓库>            # steer 按 cwd 找实例,必须在项目目录下执行
octos steer --session '<会话键>' --text '[external-reviewer] ...'
# 会话键发现:ls <repo>/.octos/octos/sessions/ ,URL 解码文件名即键
```

## 3. 观测(上行)——三层缺一不可

**投递 ≠ 消费 ≠ 执行**,只看一层必误判:
```bash
herdr pane read <pane>                                   # 现场屏幕
tail -f ~/.octos/instances/<实例>/profiles/<档>/data/events.jsonl
#   实例哈希 = octoscode 对项目 cwd 的 DefaultHasher;不想自算就
#   ls -t ~/.octos/instances/ 按 mtime 对号。ws://127.0.0.1:50090 的
#   sidecar 是外环自建临时观测件(非发行物);终态为 serve 内
#   /api/events/stream 端点(完整已测实现存档于 archive/olp-evt-ws
#   tag,待单 serve 多客户端拓扑成熟解冻)。
octos goal status --goal <id> / octos peer list           # 结构面(项目目录下)
```

## 4. 复验与代推(主审义务)

- 内环 ACK 的自验声明**不可轻信**:在隔离 worktree 独立重跑——
  ```bash
  git worktree add ~/.octos/outer/verify/<名> <commit>
  # 复验命令必须逐字取自 .github/workflows(CI 矩阵含无默认 feature 的
  # clippy --all-targets;octos-cli 靶向测试要 -p octos-cli --features api)
  git worktree remove --force ~/.octos/outer/verify/<名>   # 验后即清
  ```
- 通过 → 落采认判词(署名)→ `git push fork <分支>` 代推;
  不通过 → 新条目写明证据改派,**不改写旧条目**。

## 5. 安全红线

1. **共享树主权**:主工作树只属一个 goal 的分支;并行工作一律开独立
   worktree,严禁在共享树上 checkout(外环自己也遵守)。
2. 未经独立复验不推送;不碰 operator 权限(免沙箱授权是人的动作)。
3. 队列尊重:master 按板序吃单;要插队,在条目里写明主张,由 operator
   或与在班外环黑板协商,不搞事实抢跑。
4. 共享机限载:并发编译全机 ≤2,测试 `--test-threads=8`;大临时文件
   确认 `TMPDIR` 已指 home 盘。

## 6. 内环选型与开设(内环契约是 agent 无关的)

**内环契约**只有四条:读黑板 Active 区 → 执行最小编号未 ACK 条目 →
只 commit 不 push → 落 `ACK(done|wontdo|blocked)` 定式。任何能读文件、
跑命令、被 herdr 驱动的 agent 都能当内环。三种形态按需选:

| 内环形态 | 优势 | 适用 |
|---|---|---|
| octoscode + 便宜模型(标准形态) | 成本低;goal/peer/ledger/steer/事件流全套机械 | 慢轨战役、机械大批量 |
| Claude Code 免审批窗格 | 质量高、不交赝品、单兵战力强 | 快轨 bug、难切片 |
| codex 窗格(全局 auto 配置后) | 同上,且与主审厂牌隔离(利于互审) | 同上 |

**开设命令**:
```bash
# 标准形态(octoscode)
herdr pane run <pane> 'cd <repo> && octoscode --stdio-command "octos serve --stdio --solo --danger-full-access"'
# Claude Code 快轨内环(免审批启动属信任决策,建议 operator 亲手执行)
herdr pane run <pane> 'cd <repo> && claude --dangerously-skip-permissions'
# codex 快轨内环(先在 ~/.codex/config.toml 设 approval_policy="never" + sandbox_mode)
herdr pane run <pane> 'cd <repo> && codex'
```
开设后:发内环上岗词——"读 <repo>/.octos/loop.md 与黑板 Active 区,
以内环身份执行:只 commit 不 push,完成落 v1 定式 ACK"。

**诚实的差距清单**(裸 Claude/codex 窗格 vs octoscode):无事件流
(三层观测退化为黑板+屏幕)、无 goal/peer/R2/ledger 机械、预算不入
本体系账本、纪律靠提示词非 harness 硬约束。**快轨单兵单不受影响**;
需要机械的活仍走标准形态。

**折中优选**:若想"高端脑子 + 完整机械",不必换 harness——用
sub_providers 多模型车道(配置法见 configuration.md),给 octoscode
配强档车道(如 zai/glm、anthropic/claude),难切片按车道路由,机械
一样不少。选型优先序:强档车道 > 裸窗格 > 换 harness。

## 7. 外环战术手册(实战沉淀,按场景查)

**派单与节奏**
- 快慢双轨:修订级 bug 走快轨(直驱、可抢占、单一外环复验、免重仪式);
  战役走全流程。仪式重量随单据尺寸,探索性要求挂大战役不挂 bug fix。
- 时效告警:任何条目 pending>12h 必须向 operator 报告并附插队选项。
- 中档模型磨难片时,**预置署名技术图纸**(实现三步走)比换更贵内环更省
  ——搜索空间塌缩即火力;不打断在途长 turn,图纸放板上等下轮开局。
- 内环侧对应纪律:方案空间卡壳(同一目标反复试错 >30min)应主动
  `ask_outer` 要图纸,而不是硬磨。

**复验与打回**
- 自验声明不可轻信:内环"clippy 净"两连虚报、"测试绿"靠 wrapper 冒充
  均有实案;复验命令逐字取自 CI workflow,靶向测试注意 crate 归属
  (workspace 根 --features 不传导)。
- ACK 必附逐字复验命令(crate/模块/feature 全写明)。
- 赝品识别:exit code 冒充行为、script wrapper 包装、测试替身冒充生产
  路径——静态复核这三类是"验收的验收"的基本功。

**运维安全**
- 禁批量 kill serve:逐个核对"父进程==存活 TUI"后单杀;孤儿只占内存,
  宁留勿滥杀。
- CLI 寻址:octos goal/steer 等按实例操作时用 `OCTOS_HOME=<实例根>`;
  会话主模型切换的落点是 profile JSON 的 `config.llm.primary`
  (fallbacks 数组即备胎位)。
- provider 断供(quota/auth 拒付)是系统性风险:备胎车道预配 +
  引擎自动 fallback(整改项);断供时全线空转的形态=连续短 turn 零产出。
- profile JSON 时间戳必须 RFC3339 带 Z,写坏即 profile 整体失效。
- **原型/发行判据**:常驻 + 协议 + 发行三占其二必须 Rust(如 OLP-MCP
  server #31 的 Python 原型一夜后即移植);一次性引导与本地胶水可
  shell/python(如 olp-init.sh、board_append.sh)。

**哨兵体系**
- master-sentry(自动续拍):旗标开+空闲即注入续拍令,3 次无板面进展
  升级外环并自停;哨兵管节拍,外环管焦点(升级时给定向任务,勿泛令)。
- 引擎 turn 结束钩子落地后,哨兵降级为兜底。
