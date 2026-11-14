# OctoLoop 多机部署:本地 git 仓库(本地 PR)

> 规则见 `docs/OUTER_LOOP_PROTOCOL.md` 附录 C。这一页是操作步骤。

各单元不直接推 GitHub,只向 hub 上的一个裸仓库推 `task/<N>`。一次推送就是一个
"本地 PR":大外环基于这个仓库审核与回测,通过后才由 hub 推到 GitHub。

## 1. hub 上:建仓库

```bash
scripts/olp-git-hub.py init /srv/olp/hub.git --board <repo>/.octos/OUTER_LOOP_REVIEW.md
```

建裸仓库、装两个钩子、把黑板路径记进仓库配置。重跑是安全的(换了脚本路径或
黑板路径后重跑一次)。集成分支、种子分支由大外环在 hub 本机推进去,不受限制。

## 2. hub 上:给每个单元的密钥配强制命令

在 hub 的 `~/.ssh/authorized_keys` 里,每个单元一行:

```
command="/path/scripts/olp-git-hub.py serve --unit m07 --repo /srv/olp/hub.git",no-pty,no-port-forwarding,no-agent-forwarding ssh-ed25519 AAAA… m07-git
```

这个密钥只能对这个仓库做 git 推送与拉取,钩子看到的单元号由这一行决定,单元
自己改不了。**不要给单元发不带强制命令的密钥**:不带单元号的推送被视为 hub
本机操作,不受限制。

## 3. 单元上:加远程

```bash
git remote add hub ssh://<hub 主机>/srv/olp/hub.git
git fetch hub
```

## 4. 派卡(hub 上)

条目里用一行 `unit: m07` 指定单元:

```
### 212. 修复 xxx(2026-11-12,外环(hub))
unit: m07

背景、文件与行号、修法、验收标准、分支 task/212 …
```

## 5. 单元内复验通过后推送(由单元主持执行)

olp/v3 起,单元主持在卡上指定的复验方全部通过后推送,署名写出各复验方:

```bash
scripts/olp-task-push.sh 212 --verified-by m07-codex+claude
```

返工或 rebase 到新种子后覆盖先前推过的分支,加 `--force`。

hub 的钩子会拒收:

| 情况 | 提示 |
|---|---|
| 推的不是 `task/<N>`(集成分支、标签等) | 单元只能推 refs/heads/task/<N> |
| 板上没有这张卡 | 黑板上没有条目 #N |
| 这张卡没有派给本单元 | #N 派给的是 … |
| 没有本地复验记录,或记录不是这个提交的 | 缺少本地复验记录 |
| 删除分支 | 单元不能删除分支 |

内环自己直接 `git push hub task/212` 属于第四种,推不上去。

## 6. 推送之后

钩子在权威黑板末尾追加一行:

```
> 单元(m07)·本地复验通过(#212): task/212 @ 3f2a9c1d4e5f 复验 m07-codex+claude
```

大外环对 `本地复验通过(#212)` 挂正哨即可被唤醒。olp/v3 起**大外环不复验**,只核对
手续:有这条登记、提交号等于分支头、单元的 `ACK(done)` 带着各复验方的证据、改动
范围与卡相符(例如 `git --git-dir <仓库> diff --stat main..task/212`)。齐了就采认,
由 hub 推 GitHub;不齐就在黑板上打回并唤醒该单元。单机非集群的双环不变。

带复验记录、却没有经过 `serve` 的推送(克隆的 `hub` 远程直接写成裸仓库路径)会被
拒收,提示改用 `connect-local`:照收会绕开上面的规则,黑板上也不会登记。

黑板登记失败不回滚推送,只在推送方的输出里告警——这时请直接告知大外环。
hub 上的写板脚本依赖 `flock`,非交互 ssh 的 PATH 里要能找到它。

## 单机:hub 仓库在本机

hub 与单元在同一台机器时,在单元的工作仓库里执行一次:

```bash
scripts/olp-git-hub.py connect-local --unit m01 --repo /srv/olp/hub.git
```

它把 `hub` 远程指向本机的裸仓库,并让 git 的推送与拉取经同一个 `serve` 进行,单元号由
这里给出,推送规则不变。之后 `scripts/olp-task-push.sh` 照常使用。同一个用户下没有身份
隔离,见 `docs/OLP_CLUSTER.md` §8。
