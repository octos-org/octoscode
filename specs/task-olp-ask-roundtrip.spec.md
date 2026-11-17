spec: task
name: "多机部署:内环提问到达 hub、回答送回单元,MCP 信号进入进化环"
id: task-olp-ask-roundtrip
status: implemented
tags: [olp, cluster, mcp, ask-outer, evolution]
estimate: 1d
---

## Intent

内环卡住时用 `ask_outer` 向外环提问、用 `report_blocked` 报告阻塞。这两个工具的信箱和审计都
在本机:多机部署时大外环在 hub 上,既看不到提问,也没有办法把回答送回去——提问只会在 50 秒
后超时。同时,阻塞报告和超时提问是进化环最直接的实测证据,在集群里却一条也采集不到。本任务
补上去程的署名、回程的工具、收件箱里的展示,以及进化环对这些行的采集。

## Decisions

- 去程:`olp-board-remote.py append --sign` 给正文每个非空行加上 `> 单元(<U>)·` 前缀。单元
  上的 `~/.octos/outer/board_append.sh`(MCP 服务的审计钩子)写成对它的一行转发,于是 hub
  的黑板上出现 `> 单元(m07)·- <时间> MCP(ask_outer) ask: id=<id> question=<前 200 字>`,
  以及同形的 `answer`、`timeout`、`blocked`、`refusal` 行。`src/olp_mcp.rs` 不改。
- 回程:新工具 `scripts/olp-answer.py`(纯 Python 标准库)。hub 侧 `list <U>`、
  `show <U> <id>`、`answer <U> <id>`(回答走 stdin),经
  `ssh -o BatchMode=yes -o ConnectTimeout=10 <目标> <动作> [id]` 到单元;目标默认等于单元号
  (约定主机名即单元号),`--to` 可改。单元侧 `serve [--root] [--max-age]` 是 ssh 强制命令,
  只接受这三个动作,只读写 `<root>/mcp/questions` 与 `<root>/mcp/answers`。
- `list` 的范围:还在等的提问,加上最近 `--list-window` 秒(serve 的选项,默认 86400)内
  超时的;更早超时的不列出,只在 stderr 报一个数。超时的提问文件留在信箱里不会自己消失
  (MCP 服务只归档已回答的),不设窗口的话列表只增不减。本工具不删信箱里的文件,`show`
  仍能按 id 看任何一条。
- `answer` 的规则:提问 id 必须是信箱里还在的提问;提问文件的年龄不得超过 `--max-age`(默认
  50 秒,与 MCP 服务的等待时间一致)——超过则拒绝并提示改用黑板或 herdr,因为内环已按降级
  指引继续,回答没人读。回答上限 64 KiB、UTF-8、非空;写成 `answers/<id>.json`
  (`{"answer": …, "ts": …, "via": "olp-answer"}`),经临时文件原子替换。`show` 只输出提问的
  五个字段,对已超时的提问仍可用。
- 收件箱:`olp-inbox.py` 新增"待回答的提问"——板上有 `MCP(ask_outer) ask: id=X` 而没有同 id
  的 `answer` 或 `timeout` 行。文本输出排在最前面并给出回答命令;JSON 在 `questions` 里
  (`id`、`unit`、`question`、`line`)。未署名的行(单机)同样识别,`unit` 为空。
- 进化环:`olp-evo-harvest.sh` 在活板上识别单元署名的 MCP 审计行
  `单元(<U>)·- <时间> MCP(ask_outer) (blocked|timeout): …`,落成既有的 `report_blocked` /
  `ask_outer_timeout` 卡,症状与 MCP 审计板来源同形并带上单元:
  `kind=<k> id=<id> unit=<U> reason=<前 80 字>`。identity 为
  `board:<路径>#unit:<U>#mcp_<k>#<行哈希>`。`ask`、`answer`、`refusal` 不是摩擦,不落卡;活板
  上未署名的 MCP 行不采集(单机上它们在 MCP 审计板,由既有来源采集,避免重复)。retro 的
  分组文本取 `reason=` 之后的内容,所以两台单元报的同一个原因归为一个候选。
- 退出码(`olp-answer.py`):0 成功;2 用法错误;3 单元拒绝;75 单元不可达。

## Boundaries

### Allowed Changes
- scripts/olp-answer.py
- scripts/olp-board-remote.py(`append --sign`)
- scripts/olp-inbox.py(待回答的提问)
- scripts/olp-evo-harvest.sh(仅活板扫描里新增单元署名的 MCP 行)
- tests/olp_ask_roundtrip.rs、tests/olp_evo_harvest.rs(新增一个测试)
- docs/OLP_REMOTE_BOARD.md
- specs/task-olp-ask-roundtrip.spec.md

### Forbidden
- 不改 src/olp_mcp.rs 与 MCP 工具的契约(信箱布局、超时、审计行格式)。
- 不改既有的采集定式与回放夹具。
- 单元侧的强制命令不得读写信箱以外的路径。

## Out of Scope

- 把内环的等待时间做成可协商的。
- 回答的排队重发(单元不可达时直接报告,等待时间内重试没有意义的情况由外环判断)。
- 在 TUI 里展示待回答的提问。

## Completion Criteria

### Rule: roundtrip — 提问与回答的往返
Scenario: 单元里的提问到达 hub,回答送回单元,内环拿到原文(critical)
  Tags: critical
  Test: ask_outer_round_trip_across_machines
  Given 单元上运行真实的 `octoscode olp-mcp-serve`,它的审计钩子转发到 hub
  When 内环调用 ask_outer;大外环在收件箱看到提问,用 show 取全文,用 answer 回答;随后再回答一次
  Then hub 的黑板上出现单元署名的 ask 行;收件箱的 `questions` 里是这条提问并带单元号;show 给出上下文与已试过的内容;内环的 ask_outer 返回回答原文;answer 行到达 hub 后提问离开收件箱;第二次回答以退出码 3 被拒且信箱里不留文件

Scenario: 没人回答时提问超时,板上的超时行让它离开收件箱
  Test: ask_outer_timeout_leaves_the_inbox_and_refuses_a_late_answer
  Given 一条没人回答的提问超时,随后内环调用 report_blocked
  When 查看收件箱;在等待时间过后列出提问并尝试回答
  Then 超时行与署名的 blocked 行都到达 hub;收件箱里没有待回答的提问;提问显示为已超时,迟到的回答以退出码 3 被拒并提示改用其他通道,信箱里不留回答文件;show 仍可用

Scenario: 回答命令的输入检查与单元不可达
  Test: answer_validates_input_and_reports_an_unreachable_unit
  Given 非法单元号、含路径字符的 id、空回答、含空格的 id、写成 ssh 选项的目标;不存在的提问;强制命令收到的其他动作;单元不可达
  When 分别运行
  Then 输入错误为退出码 2;不存在的提问为 3;强制命令对其他动作一律以 3 拒绝;不可达为 75

Scenario: 早已超时的提问不再出现在列表里
  Test: answer_list_hides_questions_that_timed_out_long_ago
  Given 单元的信箱里有四个提问:刚提的、十分钟前超时的、两天前的、三十天前的
  When 经强制命令 list;把 `--list-window` 调大后再 list;show 三十天前的那一条
  Then 默认只列出前两条,stderr 报"另有 2 条"未列出;调大窗口后四条都列出;被隐藏的提问仍能 show;信箱里的文件一个没少

### Rule: evidence — 进入进化环
Scenario: 单元转发来的 MCP 审计行落成既有的 MCP 类卡(critical)
  Tags: critical
  Test: olp_evo_harvest_unit_mcp_lines_become_mcp_cards
  Given 活板上有单元署名的 ask、timeout、两台单元各一条同原因的 blocked、answer,以及一条未署名的 blocked
  When 运行采集,再运行一次
  Then 恰好三张卡:一张 ask_outer_timeout、两张 report_blocked;症状带单元号且两张 blocked 的 reason 相同;timeout 行没有 `reason=` 字段,它的 reason 是 id 之后的文字(`no answer in 90s`),不含 id 的残片;ask、answer 与未署名的行不落卡;第二次运行不新增卡

Scenario: 单机的 MCP 板上,超时行的 reason 是 id 之后的文字
  Test: olp_evo_harvest_mcp_timeout_reason_is_the_text_after_the_id
  Given MCP 审计板上有一行 `timeout: id=0a06174e2d90 no answer in 90s`
  When 运行采集
  Then 症状为 `kind=timeout id=0a06174e2d90 reason=no answer in 90s`
