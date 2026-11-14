# OctoLoop 多机部署:单元登记与健康识别

> 适用于"一台 hub + 多台单元"的部署:hub 放黑板、跑大外环;每个单元是一台跑
> 内环车道的机器(Mac mini 或 Linux 机器,hub 也可以是 Linux)。单机使用
> OctoLoop 不需要读这一页。第一次把一台单元接进来,按 §7 的清单从头做。

工具是 `scripts/olp-unit.py`(纯 Python 标准库)。它回答两个问题:这台机器是不是
登记过的那个单元;它现在能不能接卡。

## 1. 编组规则

- 单元号用 `m01`、`m02`……,署名、设备名都带上它(`m07`、复验署名 `m07-codex+claude`、`m07-d03`)。
- 每个单元在 hub 上登记四样东西,各自唯一:

| 登记项 | 用途 |
|---|---|
| 单元号 | 派卡、署名、herdr 机器标签都用它 |
| 主机名 | 单元把主机名固定成单元号:`sudo scutil --set HostName m07` |
| 内置网卡地址 | 路由器按它做 DHCP 保留;心跳里核对,用来认机器 |
| ssh 主机密钥指纹 | hub → 单元方向固定 known_hosts,一个单元号只对应一把主机密钥 |

- 网卡地址**不是**安全凭据(可以伪造,换转接头也会变)。登记**内置以太网口**的
  地址;macOS 的 Wi-Fi 私有地址会轮换,不要登记它。
- 信任锚是 ssh:见 §3 的强制命令。

在 hub 上登记:

```bash
scripts/olp-unit.py register m07 --hostname m07 --mac aa:bb:cc:dd:ee:07 \
    --hostkey "$(ssh-keyscan -t ed25519 m07 2>/dev/null | ssh-keygen -lf - | awk '{print $2}')" \
    --serial C07XXXXXXX
```

hub 用 herdr 唤醒单元上的窗格时,机器标签也用单元号:`herdr machine add`
登记一次,之后 `herdr --machine m07 agent prompt …`。

## 2. 心跳与巡检(单元上)

```bash
scripts/olp-unit.py heartbeat --unit m07 | ssh hub scripts/olp-unit.py receive
```

`heartbeat` 只做本机巡检并把一条 JSON 打印到 stdout,发送由管道完成。每分钟跑
一次(macOS 用 launchd 定时,Linux 用 systemd 定时器,见
`docs/OLP_KEEPALIVE.md`)。hub 不可达时这条命令失败,下一分钟再试即可——心跳是
状态不是日志,不需要离线队列。

巡检项:

- 内置 `agent`:herdr 里至少有一个内环(`herdr agent list`),哪一种都行——
  octoscode、Claude Code、Codex。心跳同时报告有哪些内环、各自状态,hub 的
  `status` 里显示为"内环"一列。herdr 不在 PATH 上时用环境变量 `OLP_HERDR` 指定;
  两个保活工具注册心跳时会记下 herdr 的位置。
- 内置 `serve`:本机有 `octos serve` 进程。只适用于 octos 内环;需要时显式选它。
- 内置 `disk`:HOME 所在磁盘空闲比例不低于阈值(默认 10%,`--disk-min-free`)。
- `--require` 选择必须通过的内置项,默认 `agent,disk`。保活工具注册心跳时也接受
  `--require`,原样传过去。
- 扩展:`~/.octos/outer/unit-checks.d/` 下每个可执行文件是一项检查。退出 0 为
  通过,其余为不通过;stdout 第一行作为说明;10 秒超时。项目专用的检查(设备
  是否在线、循环是否暂停、双哨是否在位、fallbacks 是否已快照)放在这里。

## 3. 接收心跳(hub 上)

给每个单元的 ssh 公钥在 hub 的 `~/.ssh/authorized_keys` 里配强制命令:

```
command="/path/to/scripts/olp-unit.py receive --unit m07",no-pty,no-port-forwarding ssh-ed25519 AAAA… m07-heartbeat
```

这样 m07 的密钥只能报 m07 的心跳:自报单元号与 `--unit` 不一致时拒收(退出码 3)。
未登记的单元同样拒收。

## 4. 状态与派卡前检查(hub 上)

```bash
scripts/olp-unit.py status          # 每个单元一行;--json 给程序读
scripts/olp-unit.py check m07       # 只有"正常"退出 0
```

| 状态 | 条件 | 大外环怎么做 |
|---|---|---|
| 正常 `healthy` | 心跳按时,身份相符,巡检全过 | 可以派卡 |
| 降级 `degraded` | 心跳按时,但巡检有项不过,或主机名/网卡地址与登记不符 | 不派新卡,在途的卡继续 |
| 失联 `stale` | 没有心跳,或最近一次超过 180 秒(`--stale-after`) | 不派卡,告警 |
| 离线 `offline` | 人工标记:`olp-unit.py set-state m07 offline` | 不派卡;卡改派到其他单元 |

是否失联按 **hub 收到心跳的时间**算,不看单元自己的时钟。

`status` 还显示每台单元内环的**启动权限**,由心跳自报:

| 显示 | 含义 |
|---|---|
| 完全访问 `full` | 每个内环都以完全访问启动 |
| 沙箱 `sandboxed` | 都没有 |
| 混合 `mixed` | 有的有有的没有 |
| 无内环 `none` | 没有内环在跑 |
| 未知 `unknown` | 读不到进程表或 herdr;或者有 Claude Code / Codex 内环没带完全访问的启动参数;或心跳来自旧版本 |

"完全访问"只看启动参数:

| 内环 | 算完全访问的启动参数 |
|---|---|
| octoscode | 后端 `octos serve` 带 `--danger-full-access` |
| Claude Code | `--dangerously-skip-permissions`,或 `--permission-mode bypassPermissions` |
| Codex | `--dangerously-bypass-approvals-and-sandbox`、`--yolo`,或 `--sandbox danger-full-access` |

Claude Code 与 Codex 的权限也可以写在它们各自的配置文件里;本工具不读配置,所以没带
上面这些参数时显示"未知",而不是"沙箱"。心跳只上报这几个开关,命令行的其余部分不离开单元。
这一项只是告知,不参与健康判定,也不是安全边界(它是单元自报的)。权限档 1–4 靠
Linux 的 bwrap 沙箱实现,在 macOS 上不成立,所以 Mac mini 上要跑构建的内环通常都是
完全访问;这一列让大外环一眼看到有没有哪台机器是例外。

## 4b. 派卡:一条命令

```bash
scripts/olp-dispatch.py m07 --board <板> --title '修复启动崩溃' --wake w1:p2 <<'EOF'
背景、精确文件与行号、修法方向、验收标准。
分支 task/{N},只推到 hub 的本地仓库。
EOF
```

olp/v3 起由大外环按卡指定单元里的分工:`--impl <种类>` 是实现方,`--review
<种类>[,<种类>…]` 是对抗复验的复验方(herdr 的 agent 种类名,例如
`--impl octoscode --review codex,claude`)。卡上会多出 `impl:`、`review:` 两行,
单元主持据此组织实现与复验(skill 的 unit 模式);复验全部通过,单元才推送并
回执。大外环不复验,只核对手续后收单(协议附录 C)。

依次完成:查单元健康(`olp-unit.py check`)→ 查单元空闲 → 在锁内分配编号并
写板(条目带 `unit: m07`,正文里的 `{N}` 换成编号)→ 用 herdr 唤醒那台单元的
窗格。stdout 最后一行是编号,随后对 `本地复验通过(#N)` 挂正哨。

唤醒走 `herdr --machine <单元号> agent prompt <窗格> …`,所以 hub 上要先把这台
单元登记成 herdr 的机器,标签等于单元号(§7 第 11 步)。唤醒的那句话指向条目号,
并带着一条可以原样执行的命令,让内环先把本机的黑板镜像同步到最新再读卡
(原因见 `docs/OLP_REMOTE_BOARD.md` §5)。

| 情况 | 结果 |
|---|---|
| 单元不是"正常" | 拒绝,不写板(`--skip-health` 跳过) |
| 单元还占着没了结的卡 | 拒绝,不写板(`--allow-busy` 放行) |
| `--verify-of M` 且目标就是做 #M 的单元 | 拒绝:回测卡要派给另一台 |
| 写板成功、唤醒失败 | 退出码 5;**原样重跑**只补做唤醒,不会写第二张卡 |

一台单元同一时间只做一张卡。占用情况:

```bash
scripts/olp-inbox.py <板> --units        # 每个单元:空闲,或还占着哪些卡
```

带 `unit:` 行的卡被打回后仍算占着那台单元(它要返工并重新推送),直到采认、
改判作废或归档。

把 R1 冒烟这类回测派回单元时,用 `--verify-of`:

```bash
scripts/olp-dispatch.py m08 --board <板> --title 'R1 冒烟:#212' --verify-of 212 <<'EOF'
…
EOF
```

## 4c. 负哨:单元上的异常不沉默

每台单元在内环旁边跑一个只观察的守护,并把值得知道的事送到 hub:

```bash
octoscode olp-runtime-watch --agent <herdr 名字> --agent-pid <pid> --backend-pid <pid> \
    --repo <仓库> --ledger <当前 UI ledger> --session <会话> --turn-id <turn> \
    --task 212 --progress-file <本卡要改的文件> --stall-seconds 900 \
  | scripts/olp-alert-forward.py
```

`--backend-pid` 和 `--ledger/--session/--turn-id` 只适用于 octos 内环。内环是 Claude Code
或 Codex 时不给它们(ledger 三项要么一起给,要么都不给):

```bash
octoscode olp-runtime-watch --agent <herdr 名字> --agent-pid <claude 或 codex 的 pid> \
    --repo <仓库> --task 212 --progress-file <本卡要改的文件> --stall-seconds 900 \
  | scripts/olp-alert-forward.py
```

这时没有 turn 一层(不报 `turn_error`),停滞提示靠 herdr 的状态区分"在忙但文件不动"
(`events_continuing`)和"什么都没做"(`no_events`)。agent 的 pid 用
`herdr pane process-info --pane <窗格>` 查。

守护每个周期输出一行 NDJSON:进程是否还在、目标 turn 有没有报错或结束、命名的文件有没有
真实变化。它只观察——不杀进程、不重启、不写板,`task_accepted` 恒为 false。

转发器把下面这些事各报一次(持续期间不重复),落到 hub 的黑板:

| 种类 | 含义 |
|---|---|
| `agent_missing` / `backend_missing` | 内环或后端进程不在了 |
| `watcher_missing` | 观测哨进程不在了 |
| `process_identity_changed` | pid 被别的进程复用 |
| `turn_error` | 目标 turn 报错 |
| `stalled_no_progress` | 超过阈值没有真实进展(提示性质:小的只读任务本来就不改文件) |

```
> 单元(m07)·负哨(#212): turn_error turn=01a0d877… seq=9
```

大外环在 hub 上对 `负哨(` 挂哨。单次工具失败默认不转发(正常工作中很常见),需要时加
`--include tool_failed`。hub 不可达时由远程写板的队列兜住。

## 4d. 汇总:交给项目全景台

```bash
scripts/olp-summary.py --board <板> --repo <hub 仓库> --name <项目名>          # 给人看
scripts/olp-summary.py --board <板> --repo <hub 仓库> --name <项目名> --json   # 给程序读
```

把卡(含实现方与复验方、所处分区)、各区计数、单元状态、本地 PR、待答提问、未了结
卡上的负哨收成一份。全景台不在 OctoLoop 里,用哪个工具由用户指定;大外环把这份汇总
回报给它,再从它读下一步的目标。命令只读,不写任何文件。

## 5. 不自动做的事

- **不自动改派。** 单元失联后,卡是否改派由大外环或 operator 决定。
- **不自动接管主审权。** hub 上的 `outer-duty` 锁在大外环会话死亡后变为
  VACANT,重新 hold 要 operator 确认(协议 R7)。
- **不恢复会话。** 单元重启后,内环窗格里的对话由巡检上报,不自动续跑。

## 6. 文件

| 路径(hub) | 内容 |
|---|---|
| `~/.octos/outer/units/registry.tsv` | 登记文件:单元号、主机名、网卡地址、主机密钥指纹、序列号、是否启用、备注 |
| `~/.octos/outer/units/heartbeat/<单元号>.json` | 该单元最近一次心跳,加 hub 写入的 `received_at` |

`OLP_UNITS_DIR` 改变上面的目录,`OLP_UNIT_CHECKS_DIR` 改变扩展检查目录。

## 7. 单元上线清单

把一台新机器接成单元,从头到尾是下面这些步骤。每一步的细节在括号里的文档;
这里给顺序和最短的命令。`m07` 是单元号,`hub-01` 是 hub 的主机名,`olp` 是两边
的用户。2026-11 的试点里这套顺序在 Linux(Ubuntu 24.04,真实 sshd 与 systemd)
上完整走通过。

**顺序有讲究**:单元连 hub 的每条信道都靠 hub 上的一行 `authorized_keys`,心跳
还要求单元已登记。所以生成密钥之后先做 hub 一侧(B),再回单元继续(C)。

下面的 `scripts/…` 指 octoscode 仓库里的脚本。单元的项目目录里没有 `scripts/`,
在那里执行时写全路径,第 4 步之后可以写成
`"$(cat ~/.octos/outer/octoscode-home)/scripts/…"`。

### 前提

| | macOS | Linux |
|---|---|---|
| octoscode | 从本仓库最新 `main` 构建(`cargo build --release`) | 同左 |
| octos | 2.1.0-rc.1 | 2.1.0-rc.1;官方发布包要求 glibc 2.39(Ubuntu 24.04 起),更旧的系统从源码构建(release 构建很吃内存:4 GB 的机器上与另一个构建并行时编译器被杀过) |
| herdr | `hagency-org/herdr` 的默认分支;构建需要 Zig 0.16.0 | 同左 |
| 其他工具 | `brew install coreutils flock` | 发行版自带 |
| 主机名等于单元号 | `sudo scutil --set HostName m07` | `sudo hostnamectl set-hostname m07` |

依赖的完整说明在 `docs/OLP_QUICKSTART.md` §1。

### A. 在单元上:生成密钥

1. 三把密钥,各管一条通往 hub 的信道,并在 `~/.ssh/config` 里各配一个别名:

   ```bash
   for k in hb push board; do ssh-keygen -q -t ed25519 -N '' -C "m07-$k" -f ~/.ssh/olp-$k; done
   cat >> ~/.ssh/config <<'EOF'
   Host hub-hb hub-git hub-board
     HostName hub-01
     User olp
     IdentitiesOnly yes
   Host hub-hb
     IdentityFile ~/.ssh/olp-hb
   Host hub-git
     IdentityFile ~/.ssh/olp-push
   Host hub-board
     IdentityFile ~/.ssh/olp-board
   EOF
   ```

### B. 在 hub 上:放行这台单元

2. 把单元的三把公钥加进 hub 的 `~/.ssh/authorized_keys`,每把带各自的强制命令
   (路径换成 hub 上 octoscode 仓库的位置;`<板>` 是 hub 上权威黑板的绝对
   路径):

   ```
   command="/path/scripts/olp-unit.py receive --unit m07",no-pty,no-port-forwarding <olp-hb.pub>
   command="/path/scripts/olp-git-hub.py serve --unit m07 --repo /srv/olp/hub.git",no-pty,no-port-forwarding <olp-push.pub>
   command="/path/scripts/olp-board-remote.py serve --unit m07 --board <板>",no-pty,no-port-forwarding <olp-board.pub>
   ```

   这三把都**必须**带强制命令;不要给单元发不带强制命令的密钥。
3. 登记单元(§1)。`--mac` 是内置网口的地址(macOS:
   `networksetup -listallhardwareports`;Linux:`ip -br link`);`--hostkey`、
   `--serial` 可选:

   ```bash
   scripts/olp-unit.py register m07 --hostname m07 --mac <内置网口的地址>
   ```

### C. 回到单元上

4. 记下 octoscode 仓库的位置,内环靠它找到脚本:

   ```bash
   scripts/olp-install-skill.sh --home-only
   ```
5. 取得项目并接上 hub 的本地仓库(`docs/OLP_LOCAL_GIT.md`)。项目从哪里克隆
   都可以(GitHub,或直接从 hub 的本地仓库);要紧的是有一个名为 `hub` 的
   远程:

   ```bash
   git remote add hub ssh://hub-git/srv/olp/hub.git && git fetch hub
   ```
6. 在项目目录里铺上岗卡(`OLP_INIT_LANG=zh` 取中文版)。它同时
   生成一块本地黑板,下一步的镜像会替换它:

   ```bash
   bash "$(cat ~/.octos/outer/octoscode-home)/scripts/olp-init.sh"
   ```
7. 写单元配置,并同步一次镜像,确认写板信道通了(`docs/OLP_REMOTE_BOARD.md`):

   ```bash
   printf 'hub = hub-board\nunit = m07\nmirror = <项目绝对路径>/.octos/OUTER_LOOP_REVIEW.md\n' > ~/.octos/outer/unit.conf
   scripts/olp-board-remote.py mirror
   ```
8. 把 MCP 的审计钩子指向 hub,内环的提问才会出现在 hub 的黑板上:

   ```bash
   printf '#!/bin/sh\nexec python3 <仓库>/scripts/olp-board-remote.py append --sign\n' > ~/.octos/outer/board_append.sh
   chmod +x ~/.octos/outer/board_append.sh
   ```
9. 保活(`docs/OLP_KEEPALIVE.md`)。macOS 用 `olp-launchd.py`,Linux 用
   `olp-systemd.py`,子命令相同;Linux 要先开 linger。herdr 的路径按实际安装
   位置写(`command -v herdr`),systemd 要求绝对路径:

   ```bash
   scripts/olp-systemd.py doctor                     # macOS:scripts/olp-launchd.py doctor
   sudo loginctl enable-linger <用户>                 # 仅 Linux,doctor 报 FAIL 时
   scripts/olp-systemd.py heartbeat --unit m07 --hub hub-hb
   scripts/olp-systemd.py install herdr --keepalive -- <herdr 的绝对路径> server
   ```
10. 在 herdr 里开一个窗格启动内环。`workspace create` 的输出里有窗格号(形如
    `w1:p1`),派卡时的 `--wake` 用的就是它。免沙箱是 operator 的决定(见
    `docs/OLP_QUICKSTART.md` 的"0b. 权限档语义"):

    ```bash
    herdr workspace create --cwd <项目>
    herdr agent start inner --kind octoscode --pane <窗格号> -- --stdio-command 'octos serve --stdio --solo --danger-full-access'
    ```

### D. 再到 hub 上:回程信道与唤醒

11. hub 到单元有两把密钥,公钥都加进**单元**的 `~/.ssh/authorized_keys`:
    - 回答提问用的一把,带强制命令 `command="/path/scripts/olp-answer.py serve",no-pty,no-port-forwarding`;
      约定 hub 上 `ssh m07` 用的就是这一把(`olp-answer.py` 默认连单元号)。
    - herdr 用的一把,**不带**强制命令(它要在单元上执行 herdr 的命令),在 hub
      的 `~/.ssh/config` 里另起一个别名。这是 hub 登录单元,和第 2 步"不给单元
      发不带强制命令的密钥"不冲突。

    ```bash
    herdr machine add <herdr 用的别名> --label m07
    herdr --machine m07 agent list          # 应看到单元上的 octoscode 窗格
    ```
12. 确认单元是"正常",权限一列符合预期:

    ```bash
    scripts/olp-unit.py status
    ```

之后就可以用 §4b 的一条命令派卡。

### 一台能交活的单元还需要

上面十二步让单元能接卡、干活、回执。要让活能离开单元,还要这些(olp/v3,协议
附录 C):

- **单元主持**。第 10 步里被唤醒的那个窗格就是它(免沙箱启动的一个 agent)。它按
  卡上的 `impl:`、`review:` 行组织实现方与复验方,全部复验通过后执行
  `scripts/olp-task-push.sh <N> --verified-by m07-<复验方>+<复验方>` 推送,再向 hub
  回执 `ACK(done)`。规程在 skill 的「模式 unit」。实现方自己 push 会被 hub 拒收。
- **实现方与复验方**。由单元主持按卡启动,一律免沙箱,各用独立的克隆;要推送的
  克隆必须接好 hub 远程(单机部署用 `connect-local`)。
- **负哨守护**(§4c)。每张卡开工后,在内环旁边跑
  `octoscode olp-runtime-watch … | scripts/olp-alert-forward.py`,内环挂掉、报错
  或空转时 hub 的黑板上才会有记录。它的参数(pid、ledger、turn)随每次会话
  变化,所以不注册成常驻任务。

可选:把镜像的定时同步注册成常驻任务,作为兜底(`docs/OLP_REMOTE_BOARD.md` §5)。

### 重启之后

心跳和 herdr 服务自己回来;内环不会(§5)。这时 hub 把单元判为降级并拒绝派卡,
直到有人在窗格里重新启动内环(第 10 步)。

## 8. 单机:hub 与单元在同一台机器

单机是集群的一个特例:一台机器既是 hub 又是单元。集群的工具照常用,只是 hub 与单元
之间不经 ssh——hub 一侧的处理程序(与 ssh 强制命令同一段代码、同样的规则)在本机直接
运行。

| 信道 | 集群 | 单机 |
|---|---|---|
| 写板、ACK、镜像 | `unit.conf` 的 `hub = <ssh 别名>` | `hub = local`,另加 `board = <黑板绝对路径>` |
| 推送任务分支 | `git remote add hub ssh://…` | 在工作仓库里执行一次 `scripts/olp-git-hub.py connect-local --unit m01 --repo <裸仓库>` |
| 心跳 | `heartbeat --hub <心跳别名>` | `heartbeat --hub local` |
| 回答提问、唤醒窗格 | hub 经 ssh 与 `herdr --machine` | 单元登记的主机名就是本机时,自动在本机进行 |

最短的配置(单元号 `m01`):

```bash
scripts/olp-unit.py register m01 --hostname "$(hostname -s)" --mac <本机网口地址>
printf 'hub = local\nunit = m01\nboard = %s\nmirror = %s\n' <黑板> <黑板> > ~/.octos/outer/unit.conf
scripts/olp-git-hub.py init <裸仓库> --board <黑板>
( cd <项目> && scripts/olp-git-hub.py connect-local --unit m01 --repo <裸仓库> )
scripts/olp-launchd.py heartbeat --unit m01 --hub local      # Linux:olp-systemd.py
```

内环在本机的同一个项目里工作时,镜像路径就是黑板本身,`mirror` 什么也不做;内环直接读
黑板,写入仍走 `olp-board-remote.py ack/append`(规则由 hub 一侧检查)。`olp-unit.py where
m01` 输出 `local` 表示 hub 把这台单元认作本机。

**单机没有身份隔离。** 同一个用户下,单元号来自配置;直接 `git push <裸仓库路径>` 能绕开
推送规则。单机模式防的是误操作(内环推错分支、给自己写采认),不防有意绕开。要真正的隔离,
用两台机器(或同一台机器上的两个用户)加 ssh 强制命令。
