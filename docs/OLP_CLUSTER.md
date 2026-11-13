# OctoLoop 多机部署:单元登记与健康识别

> 适用于"一台 hub + 多台单元"的部署:hub 放黑板、跑大外环;每个单元是一台跑
> 内环车道的机器(例如一台 Mac mini)。单机使用 OctoLoop 不需要读这一页。

工具是 `scripts/olp-unit.py`(纯 Python 标准库)。它回答两个问题:这台机器是不是
登记过的那个单元;它现在能不能接卡。

## 1. 编组规则

- 单元号用 `m01`、`m02`……,署名、设备名都带上它(`m07`、`m07-ver`、`m07-d03`)。
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
一次(用 launchd 定时)。hub 不可达时这条命令失败,下一分钟再试即可——心跳是
状态不是日志,不需要离线队列。

巡检项:

- 内置 `serve`:本机有 `octos serve` 进程。
- 内置 `disk`:HOME 所在磁盘空闲比例不低于阈值(默认 10%,`--disk-min-free`)。
- `--require serve,disk` 选择必须通过的内置项。
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

## 4b. 派卡:一条命令

```bash
scripts/olp-dispatch.py m07 --board <板> --title '修复启动崩溃' --wake w1:p2 <<'EOF'
背景、精确文件与行号、修法方向、验收标准。
分支 task/{N},只推到 hub 的本地仓库。
EOF
```

依次完成:查单元健康(`olp-unit.py check`)→ 查单元空闲 → 在锁内分配编号并
写板(条目带 `unit: m07`,正文里的 `{N}` 换成编号)→ 用 herdr 唤醒那台单元的
窗格。stdout 最后一行是编号,随后对 `本地复验通过(#N)` 挂正哨。

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
