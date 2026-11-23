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

## 1b. 给 agent 的四条约定

命令清单由 CLI 自己提供,文档只维护这四条:

- 不确定用哪条命令时,执行 `octoscode olp commands --json` 查当前版本的命令目录。按
  `summary` 选命令,用 `route` 给出的正式写法。
- 已经知道命令时,执行 `octoscode olp <组> <动作> --help` 看用法和示例;要用程序取字段
  时执行 `octoscode olp <组> <动作> --help --json`。
- 参数取值取决于当前环境时,先用对应的查询命令拿到可选值,例如
  `octoscode olp unit status --json`。
- 查帮助不会执行命令。要执行就写 `octoscode olp <组> <动作> [参数]`;单独传 `--json`
  时该选项由命令自己处理,命令仍会执行。

目录里每条命令的字段:

| 字段 | 含义 |
|---|---|
| `route` | 正式写法,如 `octoscode olp card dispatch` |
| `summary` | 一句话说明 |
| `subcommands` | `true`:下一个词是工具自己的子命令,用它的 `--help` 查 |
| `runs_on` | 在哪一侧运行:`hub`、`unit`、`any`;`internal` 是 git 或 ssh 强制命令调用的,不手工执行 |
| `writes` | 是否改动数据;`false` 的命令可以放心试 |
| `needs_lease` | 是否需要主审权租约(没有时退出码 6) |
| `implementation` | `script`(打包的脚本)或 `rust` |
| `help` | 查这条命令帮助的写法 |

`writes`、`needs_lease` 为 `null` 表示这里没有记录(取决于子命令,或尚未核实),以该命令
自己的 `--help` 为准,不要当成 `false`。参数的用法提示(`args`)和可选值的查询命令
(`values_from`)随命令迁到 Rust 逐条补上,现在还没有。

事件日志(JSON Lines,`specs/task-olp-event-log.spec.md`):`octoscode olp board import <板.md>
--to <目录>` 把 Markdown 黑板一次性导入成事件日志;`board inbox <路径>` 的路径是目录时读
事件日志,结论与原板相同。日志不手工编辑,写入走命令(`specs/task-olp-card-writes.spec.md`):
`card open`(立卡,分配卡号)、`card ack`(内环回执)、`card accept` / `return` / `void` /
`snooze`(外环判词),都带 `--log <目录>`;`board render <目录>` 把日志输出成只读的 Markdown。
单元经 ssh 强制命令 `remote serve-log --unit <单元号> --log <目录>` 访问日志
(`specs/task-olp-unit-serve.spec.md`):只能对派给自己的卡回执(`ack`)、报进度节点
(`progress`),只能读到自己的卡的事件(`events`)。单元上的命令(`specs/task-olp-unit-client.spec.md`):
`unit.conf` 里写了 `store = log` 时,`remote ack`、`remote mirror` 走这条通道,另有
`remote progress <N> claimed | review <轮次> | rework <轮次>`;没写时一切照旧。
派卡与唤醒(`specs/task-olp-delivery-wake.spec.md`):`card open --unit U --pane <窗格>` 写进日志
就是送达;单元用 `remote inbox [<窗格>]` 看还没领取的卡(`body` 是卡的全文,`next` 是接下来的
领取与回执命令),用 `remote progress <N> claimed` 领取。`remote watch <窗格>` 是单元上的值守(不是 agent):有新卡且 agent 空闲时往窗格发一句
固定短句"有新派卡,请执行:octoscode olp remote inbox <窗格>,再按输出里的 next 领取、办理、回执",
不带卡的内容。发话后 agent 开始工作才算提醒过;没接住的隔 `--retry` 秒再发一次,连续两次没接住就
不再追发。窗格被对话框或审批挡住(herdr 报 `blocked`)时不往里打字;持续接不了卡就向 hub
上报 `progress <N> undelivered`,外环的 `olp inbox` 会把这张卡列出来并写明"送达失败"。

迁到 Rust 的命令(`src/olp/`):`board inbox`。迁过去的命令不再经过打包的脚本;
`OCTOSCODE_OLP_LEGACY=1` 让它改跑原脚本,保留一个版本作退路。

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
| `olp board summary --board <板或日志目录> [--repo R] [--json]` | Rust(`src/olp/summary.rs`),原 `olp-summary.py` | hub |
| `olp id alloc …` | `olp-id-alloc.sh` | 任意 |
| `olp board stall <日志目录> [--quiet 秒] [--json]` | Rust(`src/olp/stall.rs`) | hub |

`board stall`(`specs/task-olp-stall.spec.md`)回答"是不是所有人都在等":有卡未了结、占着卡的
单元上没有内环在工作、日志 `--quiet` 秒没有新事件,三条都成立才算。有人在工作就不算,做得久
不是问题。静默时逐张卡给出原因、起始事件和谁能推动;同一段沉默最多报三次。

### 2.3 卡(card)——派发

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp card dispatch <单元> --board <板> --title … [--impl A] [--review B,C] [--wake 窗格] …` | `olp-dispatch.py` | hub |
| `olp card dispatch <单元> --log <目录> --title … [--impl A] [--review B,C] [--avoid 文本]… [--verify-of M] [--wake 窗格] …` | Rust(`src/olp/dispatch.rs`) | hub |

事件日志上的派卡(`specs/task-olp-dispatch-log.spec.md`)在日志锁内依次查:单元健康、卡上的复验方
这台单元跑不跑得了(按心跳里的复验 CLI 与在跑的内环;`--allow-reviewers` 放行)、回测卡不派给
原作者、单元是否还占着卡,然后写入。`--avoid` 写明这张卡不许碰什么,单元在收件箱里看得到。

### 2.4 单元(unit)——登记、心跳、健康

| 新命令 | 现在的入口 | 在哪运行 |
|---|---|---|
| `olp unit register\|set-state\|status\|check\|where …` | Rust(`src/olp/unit.rs`),原 `olp-unit.py` 同名子命令 | hub |
| `olp unit receive [--unit U]` | Rust,原 `olp-unit.py receive` | hub(ssh 强制命令) |
| `olp unit heartbeat --unit U [--require …]` | Rust,原 `olp-unit.py heartbeat` | 单元 |

登记文件与心跳文件的格式不变,hub 换二进制不用重新登记。`OCTOSCODE_OLP_LEGACY=1` 改跑原脚本。
还没迁移的脚本(派卡、汇总、提问、体检、保活)仍直接调用 `olp-unit.py`,所以脚本暂时保留。

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
| `olp ask list\|show\|answer <单元> [id]` | Rust(`src/olp/answer.rs`),原 `olp-answer.py` 同名子命令 | hub |
| `olp ask serve` | Rust,原 `olp-answer.py serve` | 单元(ssh 强制命令) |
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
| `olp duty hold\|check\|lease …` | `octoscode outer-duty` | hub |
| `olp outer supervise\|resume\|install …` | `olp-outer.py` | hub(外环所在机器) |

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
- **`olp doctor`** 还检查:单元工作仓库的 hub 远程是否照抄了 hub 上的本地路径(fail);
  本机还在跑已被替换的 octoscode / octos / herdr 的进程(warn,升级后要重启);hub 上
  旧格式的单元心跳(warn,到单元上重装心跳任务)。
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

二进制里打包了全部 `scripts/olp-*`、skill 正文,以及 skill 与上岗卡引用到的全部文档
(上岗卡、BOOT、协议、集群与单机文档等,`docs/` 下 16 份;某份文档提到的 `docs/*.md`
没打包时单元测试失败)。第一次运行时把它们解到
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
