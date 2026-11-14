# OctoLoop 统一命令面

> 2026-11-14 定稿。第一步(本版):`octoscode olp …` 调用打包进二进制的现有脚本,
> 参数、输出、退出码不变;之后逐批把实现换成 Rust,命令面不再变。参考了 #634
> (`octoscode olp init/watch/board-append/evo …`)。

## 1. 规则

1. **一个入口**:`octoscode olp <组> <动作> [参数…]`。不再要求用户或 agent 知道
   `scripts/` 在哪。
2. **按对象分组**,不按机器分组。同一个对象在 hub 和单元两侧的动作放在同一组里,帮助
   信息写明在哪一侧运行。
3. **参数、输出、退出码与现有脚本一致**。迁移只换入口,不换语义;现有契约测试改为调用新
   命令后断言不变。
4. **组名和动作名是英文小写单词**,与黑板、协议里已有的词对应(board、unit、card…)。
5. **不暴露库文件**:`olp-preflight.sh`、`olp-hub-guard.sh`、`olp-evo-lib.py`、
   `olp-cli-lib.py` 是被别的工具加载的,没有命令。`olp-cross-check-diagnostic.sh` 是 Windows 构建诊断,与 OLP
   无关,不纳入。`olp-mcp-server.py` 是已归档的原型(实现已在 `octoscode olp-mcp-serve`)。
6. **旧入口保留一段时间**:`scripts/olp-*` 照常可用;`octoscode outer-duty`、
   `octoscode olp-mcp-serve`、`octoscode olp-runtime-watch` 作为新命令的别名保留。

## 2. 命令一览

### 2.1 起步

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp init [目录]` | `scripts/olp-init.sh` | 任意 |
| `olp skill install [--to F \| --print \| --home-only] [--force]` | `scripts/olp-install-skill.sh` | 任意 |
| `olp doctor [--repo R]… [--work W]… [--remote hub] [--json]` | `scripts/olp-doctor.py` | 任意 |

### 2.2 黑板(board)——一块黑板文件上的操作

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp board append [--key K] <板>` | `olp-board-append.sh` | 黑板所在机器 |
| `olp board inbox <板> [--units] [--json] …` | `olp-inbox.py` | hub |
| `olp board archive <板> [--keep N] [--dry-run]` | `olp-board-archive.py` | hub |
| `olp board watch <板> <token> [--harvest <repo>] …` | `olp-watch-board.sh` | hub |
| `olp board summary --board <板> [--repo R] [--json]` | `olp-summary.py` | hub |
| `olp id alloc …` | `olp-id-alloc.sh` | 任意 |

### 2.3 卡(card)——派发

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp card dispatch <单元> --board <板> --title … [--impl A] [--review B,C] [--wake 窗格] …` | `olp-dispatch.py` | hub |

### 2.4 单元(unit)——登记、心跳、健康

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp unit register\|set-state\|status\|check\|where …` | `olp-unit.py` 同名子命令 | hub |
| `olp unit receive [--unit U]` | `olp-unit.py receive` | hub(ssh 强制命令) |
| `olp unit heartbeat --unit U [--require …]` | `olp-unit.py heartbeat` | 单元 |

### 2.5 远程写板(remote)——单元写 hub 的黑板

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp remote append\|ack\|flush\|queue\|mirror …` | `olp-board-remote.py` 同名子命令 | 单元 |
| `olp remote serve --unit U --board <板>` | `olp-board-remote.py serve` | hub(ssh 强制命令) |

### 2.6 本地仓库与本地 PR(repo)

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp repo init <裸仓库> --board <板>` | `olp-git-hub.py init` | hub |
| `olp repo serve --unit U --repo R` | `olp-git-hub.py serve` | hub(ssh 强制命令) |
| `olp repo connect-local --unit U --repo R` | `olp-git-hub.py connect-local` | 单元(单机部署) |
| `olp repo push <N> --verified-by S [--force]` | `olp-task-push.sh` | 单元 |
| `olp repo status --repo R --board <板>` | `olp-local-pr.py status` | hub |
| `olp repo promote <N> --repo R --board <板>` | `olp-local-pr.py promote` | hub |
| (内部)`olp repo hook …`、`olp repo local-pack …` | `olp-git-hub.py hook / local-pack` | 由 git 调用 |

### 2.7 提问(ask)

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp ask list\|show\|answer <单元> [id]` | `olp-answer.py` 同名子命令 | hub |
| `olp ask serve` | `olp-answer.py serve` | 单元(ssh 强制命令) |
| `olp ask mcp` | `octoscode olp-mcp-serve` | 单元(MCP 服务) |

### 2.8 守护(guard)——单元上的负哨

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp guard watch …` | `octoscode olp-runtime-watch` | 单元 |
| `olp guard forward [--include K] [--only K]` | `olp-alert-forward.py` | 单元 |

### 2.9 保活(keepalive)——按平台自动选 launchd 或 systemd

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp keepalive doctor\|heartbeat\|install\|uninstall\|list\|render …` | macOS:`olp-launchd.py`;Linux:`olp-systemd.py`(同名子命令) | 单元(hub 也可) |

### 2.10 主审权锁(duty)

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp duty hold\|check …` | `octoscode outer-duty` | hub |

### 2.11 进化环(evo)

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp evo harvest <repo> [--dry-run]` | `olp-evo-harvest.sh` | hub |
| `olp evo retro <repo> [--dry-run]` | `olp-evo-retro.sh` | hub |
| `olp evo metrics <repo> …` | `olp-evo-metrics.sh` | hub |
| `olp evo index <repo>` | `olp-evo-index.sh` | hub |
| `olp evo skeleton <FLAW-NNN.md> [--out F]` | `olp-evo-spec-skeleton.sh` | hub |

### 2.12 行为证据互审(review)

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp review init\|freeze\|challenge\|cross\|status\|classify …` | `olp-review-evidence.py` | 任意 |
| `olp review cargo …` | `olp-review-evidence-cargo.py` | 任意 |
| `olp review monitor …` | `olp-review-monitor.py` | 任意 |

## 3. 写进系统里的命令

git 钩子、单元工作仓库的 receivepack/uploadpack、保活任务、ssh 强制命令里写着一条
以后才执行的命令。它们一律写 `<octoscode 绝对路径> olp …`:升级 octoscode 后自动
用新版本,不必重装。写绝对路径是因为这些地方的 PATH 很短(sshd、git 钩子、launchd)。

- `olp repo init` 写钩子 `exec <octoscode> olp repo hook <名字>`;`olp repo
  connect-local` 写 `<octoscode> olp repo local-pack …`;`olp keepalive heartbeat`
  写 `<octoscode> olp unit heartbeat … | …`。PATH 上的 `octoscode` 与正在运行的是同一个
  文件时写 PATH 上的名字(包管理器的链接比它背后带版本号的文件稳定)。
- 直接运行 `scripts/olp-*` 时照旧写仓库里脚本的路径,它随 `git pull` 更新。
  `olp keepalive heartbeat --scripts <目录>` 明确要求用该目录的脚本。
- **ssh 强制命令**由人写进 `authorized_keys`,写成
  `command="/path/bin/octoscode olp unit receive --unit m07",no-pty,no-port-forwarding …`
  (`command -v octoscode` 查路径)。
- **派卡唤醒**里写 `octoscode olp remote mirror`,备用仓库里的脚本。
- **`olp doctor`** 检查以上各处:调用 octoscode 绝对路径或仓库脚本为 ok;固定在某一版本
  的工具目录(旧版本这样写)或按 PATH 找 octoscode 为 warn;路径已不存在、钩子缺失为
  fail(退出码 1)。warn 与 fail 都附修复命令。

## 4. 已定的选择

- 按对象分组;最常用的三个另有顶层别名:`olp dispatch` = `olp card dispatch`,
  `olp inbox` = `olp board inbox`,`olp summary` = `olp board summary`。
- 单元写 hub 黑板的命令独立成 `remote` 组,不和 hub 本机的 `board append` 混在一起。
- 保活只有一个 `keepalive` 组:macOS 走 launchd,Linux 走 systemd,其他平台报错。
- `review` 纳入命令面;它的实现最后一批才换成 Rust。

## 5. 怎么运行(第一步)

二进制里打包了全部 `scripts/olp-*`、两张上岗卡和 skill 正文。第一次运行时把它们解到
`~/.octos/outer/olp-bundle/<内容哈希>/`(环境变量 `OCTOSCODE_OLP_BUNDLE` 可改根目录),
之后复用;同一版本的二进制总是同一个目录,不同版本互不覆盖。脚本之间的相对关系(同目录
的写板助手、hub 守卫、`../docs/` 上岗卡、`../skills/` 正文)和在仓库里一样。

- 脚本拿到环境变量 `OCTOSCODE_BIN`(正在运行的 octoscode),据此写 §3 的命令。
- `olp-bundle/current` 链接指向最近一次运行的版本的目录;`olp skill install` 把它记进
  `~/.octos/outer/octoscode-home`,skill 读规程正文时总是读到最新版。
- `.py` 用 `python3` 运行(`OCTOSCODE_PYTHON` 可改),`.sh` 用 `bash`(`OCTOSCODE_BASH`
  可改)。Unix 上直接替换为脚本进程,标准输入输出、信号、退出码原样传递。
- `olp guard watch`、`olp ask mcp`、`olp duty …` 本来就是 Rust 实现,直接调用,不经
  脚本。
- `octoscode olp`、`octoscode olp help`、`octoscode olp <组>` 列出命令;
  `octoscode olp <组> <动作> --help` 显示该工具自己的帮助。
