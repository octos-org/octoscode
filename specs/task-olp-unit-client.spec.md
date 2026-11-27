spec: task
name: "OctoLoop 事件日志:单元一侧的回执、进度上报与只读镜像"
id: task-olp-unit-client
status: implemented
tags: [olp, store, jsonl, unit]
estimate: 1d
---

## Intent

hub 一侧(task-olp-unit-serve)规定了单元能做什么;本任务是单元上敲的命令。命令名不变:
`octoscode olp remote ack`、`remote mirror` 照旧,新增 `remote progress`。hub 用事件日志的单元
走新通道,还在用 Markdown 黑板的单元一切照旧。

## Decisions

- 开关在单元的配置里:`unit.conf` 的 `store = log`(或环境变量 `OLP_STORE=log`)。有它,
  `remote ack` 与 `remote mirror` 由 `src/olp/client.rs` 处理;没有,原样交给
  `scripts/olp-board-remote.py`。`remote progress` 只有新通道,没有开关时退出码 2 并说明。
- 配置沿用现有的键:hub(`--hub` / `OLP_HUB` / `hub=`)、单元号(`--unit` / `OLP_UNIT_ID` /
  `unit=`);`hub = local` 时另需日志目录(`OLP_LOG` / `log=`,绝对路径),此时在本机直接运行
  hub 一侧的 `remote serve-log`,规则相同。否则经 `ssh -o BatchMode=yes -o ConnectTimeout=10
  <hub> <请求>`(`OLP_SSH` 可替换 ssh),请求按 shell 词法转义成一个参数。
- `remote ack <N> <done|wontdo|blocked> <说明…>`:说明经 stdin 送达;带幂等键
  `<单元>:ack:<N>:<卡号、结果、说明的摘要>`,所以回执丢失后原样重跑只记一次(hub 一侧的 `ack`
  接受可选的第四个词作键,必须以 `<单元>:` 开头)。
- `remote progress <N> claimed | review <轮次> | rework <轮次> [说明…]`。
- `remote mirror [--to <文件>]`(`OLP_MIRROR` / `mirror=`):向 hub 要上次之后的事件,追加到
  `<文件>.events.jsonl`,再把全部事件渲染成 `<文件>`(只读的 Markdown,只含本单元的卡)。两个
  文件都是整体替换。新的镜像路径从头取。
- 退出码:0 完成;2 用法或配置;3 hub 拒收;4 hub 的日志读写失败或返回无法解析;75 没连上
  hub(ssh 退出码 255)——未送达,原样重跑即可。本任务没有离线排队:回执与进度都带键,重跑安全。

## Boundaries

### Allowed Changes
- src/olp/client.rs、src/olp/serve.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_unit_client.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md
- specs/task-olp-unit-client.spec.md

### Forbidden
- 不改 `scripts/olp-board-remote.py`;没有 `store = log` 的单元行为不变。

## Out of Scope

- 离线排队与 `remote flush`、派卡收件箱的查收与领取、提问转发、推送。
- 镜像缓存与 hub 日志不一致(hub 重新导入)时的自动重建:删掉缓存文件再同步。

## Completion Criteria

Scenario: 单元上报进度与回执,重发不重复(critical)
  Tags: critical
  Test: a_unit_reports_progress_and_acks_once
  Given hub 的日志里 #1 派给 m07、#2 派给 m08;单元配置 store = log、hub = local
  When m07 上报 claimed、review 1、rework 1,回执 done 两次,再报一次 claimed;然后对 #2 回执与报进度
  Then 日志新增三条进度、一条回执(署名 内环(m07),带本单元的键);对 #2 的请求退出码 3;缺参数退出码 2

Scenario: 镜像只有本单元的卡,每次只取新增的事件(critical)
  Tags: critical
  Test: the_mirror_holds_only_this_units_cards_and_grows_incrementally
  Given 同样的日志
  When m07 同步镜像,无变化时再同步,外环打回 #1 后再同步,最后换一个镜像路径同步
  Then 镜像里只有 #1 及其事件、没有 m08 的卡;三次分别新增 2、0、2 条;收件箱读镜像得到"m07 忙,#1 已打回";新路径从头取

Scenario: hub 的日志换过或压缩过,本机的事件缓存按 hub 现有的内容重建
  Test: a_cache_about_another_log_is_rebuilt_from_the_hub
  Given m07 已同步过镜像
  When 把缓存改成另一份日志的事件(序号相同、时间不同)后同步,再同步;hub 把那张卡了结并压缩归档后同步
  Then 同步从缓存的最后一条事件问起,hub 没有原样送回这一条就整份重取并说明;随后又是增量;hub 归档后缓存跟着只剩 hub 现有的内容

Scenario: 经 ssh 的请求原样到达 hub;连不上时退出码 75,重跑不重复
  Test: requests_travel_over_ssh_and_an_unreachable_hub_is_retryable
  Given 单元配置 hub = hub-01;一个连不上的 ssh 与一个连得上的 ssh
  When 连不上时回执与同步;连上后同一条回执(说明里有引号和分号)发两次,再报进度、同步
  Then 连不上时退出码 75、日志不变;连上后说明原样到达、只记一次;ssh 的目标是 hub-01

Scenario: hub 还在用 Markdown 黑板的单元,命令照旧走原通道
  Test: a_unit_on_a_markdown_hub_keeps_the_old_channel
  Given 单元配置里没有 store = log
  When 执行 remote ack 与 remote progress
  Then 回执以原定式追加到 Markdown 黑板,事件日志不变;progress 退出码 2 并说明需要 store = log

Scenario: 连上以后卡住的 ssh 到时限就结束,算没有连上
  Test: a_connection_that_stalls_ends_at_the_deadline
  Given 一个连上后长时间不返回的 ssh,OLP_SSH_TIMEOUT=1
  When 执行 remote ack 与 remote mirror
  Then 两条命令都在时限到时结束,退出码 75 并提示没有连上 hub;日志里没有新事件
