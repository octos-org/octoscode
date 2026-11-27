# OctoLoop 多机部署:单元保活(launchd 与 systemd)

> macOS 上的单元(Mac mini)与 hub 用 `scripts/olp-launchd.py`;Linux 上用
> `scripts/olp-systemd.py`。两个工具的子命令和含义相同,下面 §1–§3 以 macOS
> 为例,Linux 的不同之处见 §5。

单元上有几件事必须一直在跑:向 hub 报心跳、转发负哨事件、终端工作区的后台
服务。这一页说明怎么把它们交给 launchd 或 systemd,以及哪些东西**有意不**自动恢复。

## 1. 先检查机器

```bash
scripts/olp-launchd.py doctor
```

检查两项电源设置并给出修正命令(需要 sudo,由人执行;工具不改设置):

| 检查 | 期望 | 修正 |
|---|---|---|
| 不自动休眠 | `sleep = 0` | `sudo pmset -a sleep 0` |
| 断电恢复后自动开机 | `autorestart = 1` | `sudo pmset -a autorestart 1` |

另外,launchd 的用户任务在登录后才启动,所以单元要设成**开机自动登录**。

## 2. 注册心跳

```bash
scripts/olp-launchd.py heartbeat --unit m07 --hub olp@hub-01
```

每 60 秒执行一次 `olp-unit.py heartbeat --unit m07 | ssh olp@hub-01 receive`。
hub 上这把密钥配了强制命令(见单元登记的说明),远端实际执行的是
`olp-unit.py receive --unit m07`。单机部署(hub 就是本机)写 `--hub local`,心跳直接交给本机的
`olp-unit.py receive`,不经 ssh。

## 3. 注册别的常驻任务

```bash
# 常驻:退出即重启
scripts/olp-launchd.py install workspace --keepalive -- /path/to/command --flag
# 定时:每 30 秒执行一次
scripts/olp-launchd.py install forwarder --interval 30 --env OLP_HUB=hub-01 -- /path/to/command
```

`--` 之后是任务自己的命令,原样传给 launchd。任务的标签是
`org.octos.olp.<名字>`,日志在 `~/.octos/outer/logs/<名字>.log`。launchd 给任务的
PATH 很短,所以默认写入 `/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`。

```bash
scripts/olp-launchd.py list              # 已注册的任务,是否已加载
scripts/olp-launchd.py uninstall forwarder
scripts/olp-launchd.py render demo --interval 60 -- /bin/echo hi   # 只看 plist
```

## 4. 有意不自动恢复的

- **内环的会话。** 内环窗格里的对话在重启后不自动续跑。心跳的巡检会把"没有在跑"
  报给 hub,单元变为降级;单元守护的告警会叫醒外环;主持被拉起后(§4b)按规程重新组织内环。
- **主审权锁的接管。** 锁被占着(HELD)时,换掉正在工作的外环只归 operator
  (协议 R7)。锁空着时的自动重启见 §4a。
- **`octos serve`。** 标准用法里它是 TUI 的 stdio 子进程,随 TUI 启停,不单独
  注册。
- **系统设置。** `doctor` 只报告。

## 4a. 外环自动重启

外环跑在 herdr 的一个窗格里。它死了锁就空出来;监督任务每分钟检查一次,锁空着
就在那个窗格里重新 `hold`,锁被占着就什么都不做——不会造出第二个外环,也不会
接管一把还活着的锁。不要直接把 `octoscode outer-duty hold` 注册成常驻任务:
外环是交互式的,要有终端,所以在 herdr 窗格里跑,由监督任务负责拉起。

herdr 自己重启时窗格里的进程全没了。监督任务拉起外环后,会把带 `hold` 的整条命令登记成
这个窗格的 herdr 恢复命令(herdr 0.9.2 起),herdr 重启后回来的直接就是持锁的外环。外环启动时
去掉了 `HERDR_ENV`,所以 agent 自带的 herdr 集成在外环窗格里不生效——有的集成会连状态一起报,
那样 herdr 只认它登记的裸 agent 命令,回来的外环不持锁。锁空着、窗格里却一直有程序在跑时,监督任务不会重启它,
会在黑板上留言一次。herdr 不恢复窗格创建时给的环境变量:外环依赖的变量要写进 shell 的启动
文件,或写进外环命令本身。

```bash
# 一次性:注册监督任务(macOS launchd / Linux systemd,每 60 秒检查一次)
octoscode olp outer install --project <项目> --signature <署名> --duties <职责> \
    --board <黑板> -- claude --continue
# <黑板> 是事件日志目录时,上面这条命令另注册 outer-watch:外环值守(日志有新事件时叫醒外环)
# 手工检查一次(与任务做的事相同)
octoscode olp outer supervise --project <项目> --signature <署名> --duties <职责> \
    --board <黑板> -- claude --continue
```

- 外环命令要能接着上次的会话:Claude Code 用 `claude --continue`,Codex 用
  `codex resume --last --no-daemon`(hold 拒绝不带 `--no-daemon` 的 codex)。
- 外环所在的工作区按 label 找(`--label`,默认 `olp-outer`),没有就新建。窗格里
  还有程序在跑而锁空着时不重启(退出码 3),要人看。
- 拿到锁后要保持 2 秒(`--settle`)才算重启成功。30 分钟内重启 3 次(`--window`、
  `--max-restarts`)就暂停,在黑板上告警一次(退出码 4);人看过后
  `supervise --reset` 清掉记录。
- 给了 `--board` 时,每次自动重启在黑板上记一行。
- 重启成功后,等 herdr 认出 agent 已就绪(最多 `--ready` 秒,默认 90),把接班提示送进去
  (`herdr agent prompt`):让它先跑 `octoscode olp outer resume`。`--prompt` 换成自己的
  话,`--no-prompt` 不送。agent 卡在确认界面(例如第一次进某个目录时的"信任此目录")、
  迟迟不就绪时,黑板上记一行"接班提示未送达",要人看一眼。
- `octoscode olp outer resume --board <板> --project <项目> [--signature <署名>]`:接班
  简报——本会话是否持锁、最近的自动重启、停机期间到的回执与待办(收件箱)、待回答的
  提问、负哨、单元,以及逐项要做的检查单。挂哨以挂上那一刻为基线,停机期间写进黑板
  的行只在收件箱里,所以先处理收件箱再挂哨。
- herdr 服务本身由常驻任务(§3)或人启动;监督任务找不到它时什么都不做。


## 4b. 单元主持的保活

单元上领卡、组织内环的主持 agent 也跑在 herdr 的一个窗格里。herdr 重启、agent 崩溃或机器
重启之后窗格里只剩 shell;单元值守只会向 hub 报告"接不了卡"。注册一个保活任务把它拉起来:

```bash
octoscode olp keepalive install unit-host --interval 60 \
  --env OLP_HERDR="$(command -v herdr)" --env OLP_UNIT_CONF="$HOME/.octos/outer/unit.conf" -- \
  octoscode olp unit host --pane w1:p1 -- claude --continue
```

- `--pane` 写派卡时 `--wake` 的那个窗格;或用 `--label <工作区标签>`,工作区不存在时新建。
- 窗格前台有 shell 以外的程序就什么都不做;没有才在原窗格执行 `--` 之后的启动命令。
  命令由使用者给,最好能接上次会话(`claude --continue`、`codex resume --last`)。
- 拉起后送一句接班提示:先 `octoscode olp remote inbox <窗格>`,在手的卡接着办。
  `--prompt` 换成自己的话,`--no-prompt` 不发。
- 30 分钟内重启 3 次(`--window`、`--max-restarts`)就暂停,退出码 4;看过后带 `--reset` 跑一次。
- 内环不由它恢复:主持接班后自己重新组织。

## 5. Linux:systemd 用户单元

```bash
scripts/olp-systemd.py doctor
sudo loginctl enable-linger <运行内环的用户>     # doctor 报 FAIL 时
scripts/olp-systemd.py heartbeat --unit m07 --hub olp@hub-01
scripts/olp-systemd.py install workspace --keepalive -- /path/to/command --flag
scripts/olp-systemd.py install forwarder --interval 30 --env OLP_HUB=hub-01 -- /path/to/command
scripts/olp-systemd.py list
scripts/olp-systemd.py uninstall forwarder
```

与 macOS 的不同:

| | macOS(launchd) | Linux(systemd) |
|---|---|---|
| 任务文件 | `~/Library/LaunchAgents/org.octos.olp.<名字>.plist` | `~/.config/systemd/user/olp-<名字>.service`,定时任务另有 `.timer` |
| 无人登录时运行 | 要设开机自动登录 | 要开 linger:`sudo loginctl enable-linger <用户>` |
| `doctor` 检查 | 不休眠、断电后自动开机 | linger 已开 |
| 命令的写法 | 任意 | 第一个词必须是绝对路径(systemd 不按 PATH 找) |
| 默认 PATH | 含 Homebrew | `~/.local/bin:~/.cargo/bin:/usr/local/bin:/usr/bin:/bin` |

**linger 是必须的。** 不开的话用户单元只在该用户登录期间运行,最后一个会话结束
就全部停掉,机器重启后也不会启动。没开 linger 时安装会失败,报错里直接给出
修正命令;单元文件已经写好,开了 linger 之后重跑同一条命令即可。

日志、任务形态(定时或常驻)、有意不自动恢复的东西(§4)两个平台相同。常驻任务
退出后 10 秒重启。

只支持有 systemd 的发行版。octos 的官方 Linux 发布包要求 glibc 2.39(Ubuntu
24.04 起);更旧的系统要从源码构建 octos。
