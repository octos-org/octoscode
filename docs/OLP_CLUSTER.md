# OctoLoop 多机部署:单元登记与健康识别

> 适用于"一台 hub + 多台单元"的部署:hub 放黑板、跑大外环;每个单元是一台跑
> 内环车道的机器(例如一台 Mac mini)。单机使用 OctoLoop 不需要读这一页。

工具是 `scripts/olp-unit.py`(纯 Python 标准库)。它回答两个问题:这台机器是不是
登记过的那个单元;它现在能不能接卡。

## 1. 编组规则

- 单元号用 `m01`、`m02`……,车道署名、设备名都带上它(`lane-m07-a`、`m07-d03`)。
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

派卡前先检查,通过了再写板:

```bash
scripts/olp-unit.py check m07 && scripts/olp-board-append.sh <板> <<'EOF'
…
EOF
```

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
