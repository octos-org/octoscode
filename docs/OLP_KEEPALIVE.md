# OctoLoop 多机部署:单元保活(launchd)

> 适用于 macOS 上的单元(Mac mini)与 hub。工具是 `scripts/olp-launchd.py`。

单元上有几件事必须一直在跑:向 hub 报心跳、转发负哨事件、终端工作区的后台
服务。这一页说明怎么把它们交给 launchd,以及哪些东西**有意不**自动恢复。

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
`olp-unit.py receive --unit m07`。

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

- **会话。** 内环窗格里的对话在重启后不自动续跑。心跳的巡检会把"没有在跑"
  报给 hub,单元变为降级,由大外环或人决定怎么接。
- **主审权锁。** hub 上的大外环会话死亡后,锁变为 VACANT,重新 hold 要
  operator 确认(协议 R7:活锁接管归 operator)。不要把
  `octoscode outer-duty hold` 注册成常驻任务——自动重启等于自动抢锁。
- **`octos serve`。** 标准用法里它是 TUI 的 stdio 子进程,随 TUI 启停,不单独
  注册。
- **系统设置。** `doctor` 只报告。
