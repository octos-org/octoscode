# OctoLoop — Agent 上岗卡

> protocol: olp/v2 — 读本文件的 agent 即在 OLP v2 下作业(R6 版本协商)。
>
> **OctoLoop** 是产品名;**OLP**(Outer-Loop Protocol)是协议名。
> English: [OCTOLOOP_AGENTS.md](https://github.com/octos-org/octoscode/blob/main/docs/OCTOLOOP_AGENTS.md)

**本卡自包含**:读完即可上岗,不必再开别的文档。深入文档列在末尾。卡内
一切路径与标识**全部发现式取得**,绝不硬编码——窗格号、实例哈希、会话键
都会漂移。

---

## 0. 三十秒看懂

```
┌─ 外环(强模型:Claude Code / Codex / 任意 CLI agent)
│    读黑板 → 派编号整改单 → 独立复验 → 签名判词 → 代推
│         ▲                                          │
│    .octos/OUTER_LOOP_REVIEW.md(黑板)               │  herdr prompt / octos steer
│         │                                          ▼
└─ 内环(便宜模型:octoscode TUI + octos serve,如 kimi / glm)
     读黑板 → 执行 → commit(绝不 push)→ ACK(done|wontdo|blocked)
```

便宜模型干活、可反复重跑;贵模型只花在审查与裁决上。**推送权只在外环**,
每个 commit 都过两双眼睛。

## 1. 只认一个身份

| 你被怎样启动 | 身份 | 只读 |
|---|---|---|
| 在窗格里被指派干某个仓库的活 | **内环** | 仅 §2 |
| 被要求审查/监督另一个 agent | **外环** | §3(并读 §2,那是你要执行的契约) |
| 键盘后面的人 | **operator** | §4 |

绝不同时持两个身份。不确定时你就是**内环**——外环身份需要显式的权限
声明(§3.0)。

## 2. 内环契约

内环契约**与 agent 无关**:任何能读文件、跑命令、被窗格驱动的 agent 都能
当内环。只有四条:

1. **读** `<repo>/.octos/OUTER_LOOP_REVIEW.md` 的 `Active` 区。
2. **执行**编号最小且尚无 `ACK(` 行的条目。已 ACK 的条目与
   `Historical record` 区只供审计——**绝不重放**。
3. **只 commit,绝不 push。** 推送权在外环。
4. **落 ACK**,在该条目下写一行 v1 定式:

```
ACK(done|wontdo|blocked): <说明>
```

- `done` — 已执行。说明里附证据:commit hash、测试结果,以及你跑过的
  **逐字复验命令**(crate / 模块 / feature 全写明)。
- `wontdo` — 带证据的异议。说明里写为何不做。**外环只能接受或升级
  operator,不得对同一条目再次打回。**
- `blocked` — 无法推进。说明里写阻塞原因与解除条件。

语法由契约测试 `olp_ack_lines_match_v1_grammar` 钉住。新增裸 `ACK:` 行
会让该测试失败。

### 2.1 诚实验证声明(R2)

每个交付必须声明且只声明一级:

| 级别 | 含义 |
|---|---|
| `verified` | 跑过全量门:`cargo test --all-targets` + clippy + fmt(或本项目等价物) |
| `partially-verified` | 精确列出跑了什么 |
| `unverified` | 说明原因——例如沙箱里看不到工具链 |

声称 `verified` 而复验不符属**协议违例**:打回并记入黑板。工具缺失就写
`unverified`,不要把假设包装成结果。

### 2.2 共享工作区纪律(R4 / R4b)

- 动手前先 `git status`:树里可能有外环或其他 agent 的未提交改动。
- **只 `git add` 你自己改的文件,禁止 `git add -A`。**
- 来源不明的 dirty 文件必须**保留现场并报告**,绝不自动清理或提交。只移除
  本轮由自己创建、路径已知的临时产物。
- **树主权**:主工作树只属一个 goal 的分支。并行工作一律开独立 worktree;
  非 owner 会话在主树做跨分支 `git checkout`/`git switch` 会被系统拒绝。
- 不在仓库根目录留验证草稿文件。结论写进 commit message 或 ACK。

### 2.3 卡壳时怎么办

同一目标反复试错超过约 30 分钟,那是方案空间塌不下来,不是力气不够——
**主动要图纸**,不要硬磨。若挂了 MCP 第五信道,调用
`ask_outer(question, context, tried)`(`tried` 必填,每片限 3 次,90s 超时
后按降级指引走)。否则写 `ACK(blocked): <试过什么、需要什么>` 并停下。

## 3. 外环职责

### 3.0 先取权限

```bash
octoscode outer-duty hold --project <项目> --signature <署名> \
  --duties <职责> -- <你的 agent 启动命令>
```

锁**即** authority:wrapper 是唯一持锁 fd 者,你的 agent 经
`PR_SET_PDEATHSIG` 与之死亡耦合——wrapper 亡 ⇒ agent 必亡 ⇒ 锁 `VACANT`。
`outer-duty check --project <p>` 仅观察(stdout 恰一态
`VACANT|HELD|ERROR`)。非 holder **只可署名批注**。活锁接管只归 operator,
无 agent 自助强夺。metadata sidecar 与一切 TTL 仅诊断,绝不参与裁定。

**Linux-only**(flock + PDEATHSIG + /proc;其他平台显式 unsupported 退出 2;
NFS 不适用)。macOS 上锁不可用——多外环退回值班簿纪律层 + operator 裁决。
选定署名如 `外环(claude)` / `外环(codex)`,所有黑板写入必须署名。

### 3.1 发现现场

```bash
herdr agent list                  # 内环窗格:octoscode | <pane> | idle
ls -t ~/.octos/instances/         # 按 mtime 对号(哈希 = 项目 cwd 的 DefaultHasher)
```

再读各项目 `.octos/OUTER_LOOP_REVIEW.md` 尾部的在途条目。**发现 ≠ 接管**
——你的主审域默认是启动 cwd 所属项目。

> 活板是 `<repo>/.octos/OUTER_LOOP_REVIEW.md`。`docs/` 下同名文件是冻结
> 快照,**严禁写入**:它是 tracked 的,一次 checkout 就会冲掉你的写入。

### 3.2 派单——只追加,不改写

先取当前最大号,再 +1:

```bash
grep -oE '^### [0-9]+' <板> | tail -1
```

写入必须走原子追加助手(flock 互斥,正文从 stdin 喂)。助手在 octoscode
仓库内是 `scripts/olp-board-append.sh`;`olp-init.sh` 会为其他项目在
`~/.octos/outer/board-append.sh` 装一份:

```bash
~/.octos/outer/board-append.sh <板> <<'EOF'
### <编号>. <标题>(<日期>,<署名>)
...正文...
EOF
```

两处都没有时,等价的原子追加是
`flock "<板>.lock" -c 'cat >> "<板>"' <<'EOF' … EOF`——注意 `flock(1)` 属
util-linux,原生 macOS 没有,那里只能靠约定串行化黑板写入。

条目必须**自包含**:背景、精确文件与行号、修法方向、验收标准、分支名
(基于 main)、预算档,并写明"只 commit 不 push,主审复验后代推"。

预算档:**修订** 5–10M · **切片** 10–20M · **战役** 30–50M。

改判既有条目必须落在**新的未 ACK 条目**(R1/R5)——已闭环条目下的批注
不唤醒内环。两条行首定式供采集哨机械识别,**必须逐字照抄**:

```
> 外环(<署名>)·改判(作废 #N):<以本条为准的新指令>
> 外环(<署名>)·R2 记档(#N):<声称 vs 复验事实>
```

### 3.3 唤醒与纠偏

```bash
# 窗格空闲 → 唤醒,指向新条目编号(落在 composer,即用户消息层级)
herdr agent prompt <pane> '<一句话>'

# turn 正在跑 → 用 steer 插话,不打断动作,下一拍被消费
cd <项目>          # steer 按 cwd 找实例,必须在项目目录下执行
octos steer --session '<会话键>' --text '[external-reviewer] ...'
#   会话键:ls <repo>/.octos/octos/sessions/ ,URL 解码文件名即键
```

没有 herdr 时降级 tmux `send-keys`——首字符为 `-` 的文本要用 `--` 分隔,
否则被当 flag 吃掉。

### 3.4 观测——三层缺一不可

**投递 ≠ 消费 ≠ 执行。** 只看一层必误判:goal 熔断后的沉默,与"还在
干活"完全不可区分。

```bash
herdr pane read <pane>                                     # 一、现场屏幕
tail -f ~/.octos/instances/<哈希>/profiles/<档>/data/events.jsonl   # 二、事件流
octos goal status --json ; octos ledger tail --json ; octos peer list  # 三、结构面
```

挂**双哨**而非单哨:正信号哨(ACK 落板)**加**负信号哨
(`events.jsonl` 里的 `goal_transition blocked` / `escalation`)。

### 3.5 复验,然后代推

内环的自验声明**本身不可信**——"clippy 净"两连虚报、"测试绿"靠 wrapper
冒充均有实案。自己在隔离环境重跑:

```bash
git worktree add ~/.octos/outer/verify/<名> <commit>
# 复验命令逐字取自 .github/workflows——workspace 根的 --features 不向成员
# crate 传导,靶向测试要写成 cargo test -p <crate> --features <feat>
git worktree remove --force ~/.octos/outer/verify/<名>    # 验后即清
```

盯三类赝品:exit code 冒充行为、script wrapper 包装真命令、测试替身冒充
生产路径。

通过 → 落署名采认判词 → `git push fork <分支>`。
不通过 → **开新条目**附证据改派,**不改写旧条目**。

### 3.6 红线

1. 未经独立复验,绝不推送。
2. 绝不代按 operator 的审批——免沙箱授权是人的动作。把命令写好,请人按。
3. 队列尊重:内环按板序吃单。要插队,在条目里写明主张,由 operator 定夺。
4. 共享机限载:并发编译 ≤2,测试 `--test-threads=8`,确认 `TMPDIR` 指向
   home 盘。
5. 任何条目 pending >12h 必须向 operator 报告并附插队选项。

## 4. 只属 operator 的动作

这些永不委托给 agent:贴 API key、授予免沙箱权限
(`--danger-full-access` / 权限第 5 档)、接管活的 `outer-duty` 锁、发布 issue。

内环标准启动命令:

```bash
octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'
```

- `--solo` 是单人本地盒子的安全门。漏掉则 serve 拒启:
  *"requested permission profile is not allowed outside local solo mode"*。
- **权限档 1–4 是文件系统沙箱开关,不是审批开关。** 四档都跑在 bwrap 里,
  只挂载工作区与系统目录,`~/.cargo`、`~/.rustup` **不可见**,任何构建命令
  都是 "command not found"。要跑工具链必须第 5 档 Full Access 或
  `--danger-full-access`。历史上内环反复声明"本机没有 cargo",真相即此。

(重)启动后,外环走四步硬清单:serve 已起(operator 亲手);
`/loop resume <id>`(先 `/loop list` 取 id,裸 `resume` 会拒);双哨已挂;
`llm.fallbacks` 已配**且已被新会话快照**(工具与配置在会话建立时快照,
改配置不新建会话等于纸面保险)。

## 5. 故障速查

| 症状 | 原因与修法 |
|---|---|
| 内环说"本机没有 cargo" | 权限档 1–4 的 bwrap 沙箱——第 5 档或 `--danger-full-access` |
| `permission profile is not allowed outside local solo mode` | serve 少了 `--solo` |
| `octos: 'serve' 不是子命令` | 源码构建漏 feature:`cargo build --release --features api` |
| herdr 注入静默丢失 | 双重门:named-agent 名单**与**窗格前台进程名匹配,缺一即丢。降级 tmux `send-keys` |
| 黑板没被内环读到 | 黑板被误 track 或跨分支裂脑——确认 `.octos/OUTER_LOOP_REVIEW.md` 已 gitignore;重跑 `olp-init.sh`(幂等) |
| MCP 工具(`ask_outer`)不出现 | profile JSON 写坏(时间戳必须 RFC3339 **带 Z**),或会话早于配置——修好后**新建会话** |
| 全线空转/报错停摆 | 未配 `llm.fallbacks`,主道 quota/auth 拒付 |
| Linux 构建链接器 SIGBUS / EDQUOT | `/tmp` 是带配额的 tmpfs:`export TMPDIR=$HOME/.local/tmp` |

## 6. 平台支持

| 平台 | 状态 |
|---|---|
| Linux | 全功率,含 `outer-duty` 内核锁与 bwrap 沙箱档 |
| WSL2 | 等同 Linux |
| macOS | 可用,两缺口:无 `outer-duty` 锁(Linux-only)、bwrap 档不成立 |
| Windows 原生 | 不推荐——请用 WSL2 |

## 7. 深入阅读

- [`OUTER_LOOP_PROTOCOL.md`](https://github.com/octos-org/octoscode/blob/main/docs/OUTER_LOOP_PROTOCOL.md) — 协议全文:R1–R7、
  `result.md` schema、多外环规则、预算治理、实战沉淀
- [`OLP_OUTER_BOOT.md`](https://github.com/octos-org/octoscode/blob/main/docs/OLP_OUTER_BOOT.md) — 外环操作面与战术手册
- [`OLP_QUICKSTART.md`](https://github.com/octos-org/octoscode/blob/main/docs/OLP_QUICKSTART.md) — 新项目从零到跑通
- [`OCTOLOOP_GUIDE.md`](https://github.com/octos-org/octoscode/blob/main/docs/OCTOLOOP_GUIDE.md) — 完整指南、机制篇、平台矩阵
- [`OCTOLOOP_FEATURES.md`](https://github.com/octos-org/octoscode/blob/main/docs/OCTOLOOP_FEATURES.md) — 一页能力全景
