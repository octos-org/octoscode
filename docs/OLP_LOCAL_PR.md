# OctoLoop 多机部署:本地 PR 的状态与推 GitHub 的闸门

> 前提:单元把 `task/<N>` 推到 hub 的本地仓库(一次推送就是一个本地 PR),
> hub 的钩子在黑板上登记;大外环用采认、打回两个行首定式写结论。

工具是 `scripts/olp-local-pr.py`,在 hub 上运行,只读仓库和黑板,不联网。

## 看状态

```bash
scripts/olp-local-pr.py status --repo /srv/olp/hub.git --board <板>          # --json 给程序读
```

| 状态 | 条件 | 等谁 |
|---|---|---|
| 待审核 `pending` | 板上这张卡的最后一行是单元的推送登记 | 大外环 |
| 被退回 `returned` | 最后一行是打回 | 单元:返工后重新推送 |
| 已采认 `accepted` | 最后一行是采认,GitHub 上还没有这个提交 | 大外环:`promote` |
| 已提交 `submitted` | 最后一行是采认,GitHub 远程的跟踪引用等于分支头 | 人:在 GitHub 上审核合并 |

采认之后单元又推送,状态回到待审核:采认的是先前那个提交。

`--json` 的每一项带 `partition`(`inbox` 或 `working`)和 `waiting_on`
(`outer`、`unit`、`human`),全景状态台的"本地 PR 看板"按它分栏。

## 推 GitHub

```bash
scripts/olp-local-pr.py promote 212 --repo /srv/olp/hub.git --board <板>     # --dry-run 只检查
```

只有状态是"已采认"时才推;待审核、被退回都以退出码 1 拒绝,什么都不推。推完
由大外环开 PR(标题与正文)。GitHub 远程默认叫 `github`,用 `--remote` 改。

## 警告

- 分支头与最近一次推送登记的提交号不符:有人绕过钩子改了分支。
- GitHub 上是旧提交:返工后的新提交已采认但还没 `promote`。
- 板上没有这个分支的任何记录。

"已合并"要查 GitHub,本工具不判断。
