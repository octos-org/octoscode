# OLP Quickstart — 双环协作从零到跑通

面向**第三方用户**:你有一个代码仓库,想让"便宜模型内环干活 + 强模型外环
审查"这套双环(OLP,[Outer-Loop Protocol](OUTER_LOOP_PROTOCOL.md))在自己
机器上转起来。本文只讲最短路径;协议细节、纪律与实战教训见协议文档。

## 0. 这是什么(30 秒)

```
┌─ 外环(强模型:Claude Code / Codex / 任意 CLI agent)
│    读黑板 → 派整改单 → 独立复验 → 签名判词 → 代推
│         ▲                                  │
│    .octos/OUTER_LOOP_REVIEW.md(黑板)      │ steer / prompt
│         │                                  ▼
└─ 内环(octoscode TUI + octos serve,跑便宜模型如 kimi)
     读黑板 → 执行 → commit(不 push)→ ACK(done|wontdo|blocked)
```

内环模型便宜、可反复重跑;外环模型贵、只花在审查与裁决上。推送权只在
外环,每个 commit 都过两双眼睛。

**命名落地**:OLP = 协议名(Outer-Loop Protocol,文档/ACK 定式沿用);
**OctoLoop** = 产品名——用户视角的一键化封装(`.claude/skills/octoloop`
三模式入口:init/outer/inner;能力全景见 `docs/OCTOLOOP_FEATURES.md`)。

## 1. 环境依赖清单

| 依赖 | 必须? | 说明 |
|---|---|---|
| Linux / macOS | 是 | Windows 可跑 TUI;serve 的沙箱档位(bwrap)是 Linux 特性 |
| 网络 | 首次必须 | 首启自动下载 octos server 到 `~/.octos/bin`;离线见 README 故障表 |
| Node **或** Homebrew **或** shell installer | 三选一 | 只为装二进制,**不需要 Rust**(源码构建才需 Rust 1.85+) |
| 内环模型 API key | 是 | 便宜档,例:Moonshot(kimi)。onboarding 向导里粘贴 |
| 外环 agent CLI | 是 | Claude Code / Codex 任一,用你已有的订阅 |
| herdr(终端工作区管理器) | 否,推荐 | 外环用 `herdr agent prompt` 程序化驱动内环窗格;不装可用 tmux send-keys 降级 |
| bwrap(bubblewrap) | Linux 自带居多 | 权限档 1-4 的文件系统沙箱;见下文 0b 语义 |

## 2. 一键路径

```bash
# ① 装 TUI(server 首启自动拉起,无后台常驻服务)
npm install -g @octos-org/octoscode

# ② 在你的项目目录铺 OLP 脚手架(幂等,绝不覆盖已有文件)
cd your-project/
curl -fsSL https://raw.githubusercontent.com/octos-org/octoscode/main/scripts/olp-init.sh | bash
#   (或 clone 本仓库后运行 scripts/olp-init.sh)

# ③ 启动内环
octoscode --stdio-command 'octos serve --stdio --solo'
```

`olp-init.sh` 做四件事:依赖体检(缺什么、怎么装,一屏说清)、生成
`.octos/loop.md`(内环维护循环)与 `.octos/OUTER_LOOP_REVIEW.md`(黑板
模板)、把黑板加进 `.gitignore`(分支无关,防跨分支裂脑)、打印启动命令。

**一键的诚实边界**——两件事脚本刻意不代办,因为它们是操作者的显式决策:

1. **API key**:首次进 TUI 的 onboarding 向导里自己粘贴(三个字段,五分钟);
2. **免沙箱授权**:见下一节。

### 0b. 权限档语义(第一次必读,少走两天弯路)

权限档 **1-4(Default/Read Only/Workspace Write/Never Ask)都运行在 bwrap
文件系统沙箱里**——只挂载工作区与系统目录,`~/.cargo`、`~/.rustup` 对
agent **不可见**,任何构建命令都是 "command not found"。要让内环跑
cargo/npm 等工具链,必须:

- 权限菜单选第 **5 档 Full Access**(免沙箱),或
- serve 启动时带 `--danger-full-access`(等价 `OCTOS_DANGER_FULL_ACCESS=1`),
  标准命令:

```bash
octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'
```

`--solo` 是单人本地盒子的安全门:宽松 permission profile 只在显式 solo
opt-in 下允许(漏掉的症状是 "requested permission profile is not allowed
outside local solo mode")。免沙箱属于安全决策,请操作者亲手做,不要让任何
agent 代按。

## 3. 内环车道配置(可选,省钱关键)

octos 的 `sub_providers` 把模型分车道,机械活走便宜档(开箱模板与选道
矩阵见协议文档附录 B):

```toml
[sub_providers.cheap]
model = "kimi/kimi-k2-turbo"
description = "机械性、低风险、可回滚:测试诊断、日志分类、ACK 检索、格式化搬运。"

[sub_providers.strong]
model = "anthropic/claude-opus"
description = "长链推理与跨文件判断:审查定级、分歧裁决、多步调试。"
```

**主对话车道**(profile `~/.octos/profiles/<id>.json` 的 `config.llm`,
与 sub_providers 独立——后者只喂 pipeline 节点):

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

- `primary`:常驻主道(如 zai-coding 的 glm-5.3——战役实测主力)。
- `fallbacks[]`:断供自动降级序列(quota/auth 拒付逐道切,k3 兜底、
  deepseek 应急);顺序即优先级(例:k3(moonshot-coding)兜底、deepseek 应急)。
- 修改后**新建会话**生效(工具与配置在会话建立时快照);profile JSON
  时间戳字段必须 RFC3339 带 Z。
- **回执体感**:断供发生时对话不停——状态栏闪一次降级提示,响应继续;
  恢复后主道自动回归,无需重启。

## 4. 外环最小接入(任何 CLI agent 三步上岗)

把下面这段话交给你的外环 agent(Claude Code / Codex 均可)即完成接入:

> 你是本项目的外环审查员。①读 `.octos/OUTER_LOOP_REVIEW.md` 与
> `docs/OUTER_LOOP_PROTOCOL.md`;②给内环派活 = 在黑板**追加**带日期编号的
> 条目(只追加、不改写);③内环完成后会在条目下写
> `ACK(done|wontdo|blocked): <说明>` —— done 必须独立复验(建在隔离
> git worktree 里跑测试,不碰内环工作树),wontdo 只能接受或升级操作者,
> 不得重复打回;④采认后由你 push(内环永不 push);⑤多外环时批注署名,
> 分歧升级操作者裁决。

驱动与观测信道(全部可脚本化,外环可自主轮转):

```bash
# 下行:把一条外环意见实时塞进内环正在跑的 session(独立 user 消息层级)
octos steer --session '<session-key>' --text '[external-reviewer] ...'

# 上行:结构化观测
octos goal status --json      # goal 状态机
octos ledger tail --json      # 交付账本
tail -f .../data/events.jsonl # steer_consumed / escalation / goal_transition 等事件
```

herdr 用户另有驾驶舱注入:`herdr agent list` 看窗格,
`herdr agent prompt <pane> '<text>'` 即用户消息级下发。

## 5. 冒烟验证(两分钟)

1. TUI 里发个 hello,确认主档模型回话;
2. 黑板首条(olp-init 生成的"黑板启用")让内环 ACK 掉——读写闭环即通;
3. 有 herdr 的话:`herdr agent list` 应显示 `octoscode | <pane> | idle`。

## 6. 故障速查

| 症状 | 原因与修法 |
|---|---|
| `octos: 'serve' 不是子命令 | 源码构建漏了 feature:`cargo build --release --features api`(发布二进制无此问题) |
| 内环说"本机没有 cargo" | 权限档 1-4 的 bwrap 沙箱,见 0b 节——第 5 档或 `--danger-full-access` |
| "permission profile is not allowed outside local solo mode" | serve 少了 `--solo` |
| 首启下载 server 失败 | 离线/代理:手装 `npm i -g @octos-org/octos`;`OCTOSCODE_NO_AUTO_INSTALL=1` 关自动装 |
| Linux 上构建大项目时链接器 SIGBUS / EDQUOT | `/tmp` 是 tmpfs 且可能带配额,rust-lld 临时文件很大:`export TMPDIR=$HOME/.local/tmp`(建目录后写进 shell profile) |
| herdr 注入静默丢失 | 双重门:named-agent 名单 + 窗格前台进程名匹配,缺一即丢;降级 tmux `send-keys`(首字符 `-` 的文本用 `--` 分隔) |

## 7. 深入阅读

- [OUTER_LOOP_PROTOCOL.md](OUTER_LOOP_PROTOCOL.md) — 协议全文:ACK 语法、
  result.md schema、多外环规则、预算治理、实战教训全集
- README「Quickstart (solo onboarding)」— 单环(不带外环)的逐屏引导
