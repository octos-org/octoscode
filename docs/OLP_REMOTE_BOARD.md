# OctoLoop 多机部署:远程写板、离线队列与只读镜像

> 黑板只在 hub 上。单元(mini)的写入经 ssh 在 hub 本机、在 flock 下完成;
> 读的是本机的只读镜像。工具是 `scripts/olp-board-remote.py`。

不要把黑板放到 SMB/NFS 共享盘上直接写:网络文件系统上的 flock 不可靠。

## 1. hub 上:给每个单元的密钥配强制命令

```
command="/path/scripts/olp-board-remote.py serve --unit m07 --board /path/.octos/OUTER_LOOP_REVIEW.md",no-pty,no-port-forwarding,no-agent-forwarding ssh-ed25519 AAAA… m07-board
```

这把密钥只能做两件事:`append <键>`(追加)和 `read`(读全板)。黑板路径和
单元号由这一行决定,单元改不了。

hub 对单元的写入有四条限制,违反即拒收(退出码 3):

| 限制 | 原因 |
|---|---|
| 键必须以 `<单元号>:` 开头 | 单元之间不会撞键,也不能冒名去重 |
| 不能有以 `外环(` 署名开头的行 | 采认、打回、改判、搁置只在 hub 上写,否则单元能给自己签发采认 |
| 不能有编号标题(`### 12.`、`### {N}.`) | 立卡只在大外环 |
| 正文不超过 256 KiB | — |

## 2. 单元上:配置

`~/.octos/outer/unit.conf`:

```
hub = olp@hub-01
unit = m07
```

也可以用环境变量 `OLP_HUB`、`OLP_UNIT_ID`,或每条命令带 `--hub`、`--unit`。

## 3. 单元上:写

```bash
# ACK:追加一行  > 内环(m07)·#212 ACK(done): commit abc123,测试 12/12
scripts/olp-board-remote.py ack 212 done commit abc123,测试 12/12

# 任意内容(正文从 stdin)
scripts/olp-board-remote.py append <<'EOF'
> 单元(m07)·进度(#212):构建通过,开始跑测
EOF
```

单元上的 ACK 不再是"在条目下面原地写",而是带条目号的署名行,追加在板尾。
收件箱(`olp-inbox.py`)按 `#N` 把它归到对应的条目。

每次写入都带键。不给 `--key` 时用正文的哈希,所以同一段正文重发不会在板上
出现两次。

## 4. hub 不可达时

写入存进 `~/.octos/outer/queue/`,命令以退出码 **75** 结束并提示已排队——
内环可以继续干活。

- 之后每次 `append`/`ack` 会先把队列按原顺序重发,再发新的。
- `olp-board-remote.py flush` 单独重发(可以每分钟跑一次);`queue` 查看队列。
- "hub 已经写了但回执丢了"的情况下,重发由 hub 按键去重,板上不会重复。
- hub 明确拒绝的写入不进队列。

## 5. 只读镜像

```bash
scripts/olp-board-remote.py mirror --to <repo>/.octos/OUTER_LOOP_REVIEW.md
```

把 hub 的黑板拉到本机并设为只读。内环照常读这个文件;写入一律走上面的命令。
每分钟同步一次即可。hub 不可达时镜像保持上一次的内容。镜像永不回写。

## 6. 把内环的提问送到 hub

内环的 `ask_outer` / `report_blocked` 会调用 `~/.octos/outer/board_append.sh`
记一行审计。在单元上把这个文件写成:

```sh
#!/bin/sh
exec python3 /path/scripts/olp-board-remote.py append
```

这些行就会出现在 hub 的黑板上,大外环能看到单元在提问或阻塞。

**没有解决的部分**:回答仍然要送回单元本机的信箱
(`~/.octos/outer/mcp/answers/`)。大外环看到提问后,需要经 ssh 把回答写过去,
或者用 herdr 直接对那台单元的窗格回复。

## 7. 唤醒单元上的窗格

hub 用 herdr 自带的多机命令:

```bash
herdr machine add          # 登记一次,标签用单元号
herdr --machine m07 agent prompt <pane> '<一句话,指向黑板条目号>'
```

## 已知局限

- 进化环采集只认行首的 `ACK(blocked):` / `ACK(wontdo):`,不认署名写法。经远程
  写板的 blocked/wontdo 目前不会生成症状卡。
- 每条远程写入后面会多一行 `<!-- olp-key: … -->` 标记,这是去重的依据。
