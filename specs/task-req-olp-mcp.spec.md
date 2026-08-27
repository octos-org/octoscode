spec: task
name: "OLP-MCP 外环服务:内环 turn 内主动问询外环的第五信道"
tags: [olp, mcp, outer-loop, tooling]
satisfies: [REQ-OLP-MCP]
estimate: 2d
---

## 意图

OLP 今天有四条信道(黑板批注、ACK 定式、PR review、goal 派发),全部是
外环→内环或异步账本;内环在 turn 内遇到判断分歧时**没有主动问询外环
的同步信道**,只能盲猜或 ACK(blocked) 收场。本任务落地第五信道:
一个 MCP stdio server(`scripts/olp-mcp-server.py`,纯标准库,与
`scripts/olp-init.sh` 同发行哲学),内环经 `octos mcp` 客户端机制挂载后
可在 turn 内 `ask_outer` 提问并等待外环作答。

server 是**外环侧基建**:octos 引擎零改动,octoscode 仓库只新增
server 脚本、本规格、以及 S2 的内环工具配置 diff。operator 已拍板设计
定论(见下),不重开讨论。

## 已定决策

- **工具面仅二**:`ask_outer(question, context, tried)` 与
  `report_blocked(reason, needs)`。**不做** `request_review`——代码
  复审走黑板既有信道,不容第二入口漂移。
- **信箱协议**:server 将问题写
  `~/.octos/outer/mcp/questions/<ts>-<id>.json`,轮询
  `~/.octos/outer/mcp/answers/<id>.json` 取答;**90s 超时**返回降级
  指引(按黑板既有指导继续,或 ACK(blocked)),**永不无限阻塞
  turn**。外环侧监视 questions 目录并作答的接线由外环自己实现,
  不在本任务范围。
- **审计**:每次 Q/A(发问、作答、超时降级、限额拒绝)经
  `~/.octos/outer/board_append.sh` 自动落板,署名 `MCP(ask_outer)`;
  黑板仍是唯一权威账本,信箱文件只是传输介质。
- **防思考外包**:每片(每个 goal/任务切片,以调用方传入的切片标识
  或服务端会话窗口计)限 **3 次** `ask_outer` 问询;`tried` 字段必填
  (内环已尝试过的路径,空值即拒);超限返回拒绝语,不进入信箱。
- **协议**:MCP stdio JSON-RPC(`initialize` / `tools/list` /
  `tools/call`),消息帧与能力声明以 `octos mcp` 客户端实测握手为准;
  server 不持有任何 LLM 凭据,不做任何网络访问。

## 边界

### Allowed Changes
- specs/task-req-olp-mcp.spec.md(本文件)
- octoscode olp-mcp-serve 子命令(#31 Rust 化:src/olp_mcp.rs + src/cmd/olp_mcp.rs,纯 stdlib 无新依赖;Python 原型归档 scripts/reference/)
- S2 的内环 MCP 工具配置(配置 diff 先落板给外环过目,确认后再生效)

### Forbidden
- 不得改动 octos 引擎(octos 仓库)任何代码——挂载完全走 `octos mcp`
  既有客户端机制。
- 不得在 server 内引入第三方依赖(pip 包、vendored 库);纯 Python 3
  标准库。
- 不得新增 `request_review` 或任何第三个工具。
- 不得让 `ask_outer` 无限等待;90s 上限是硬约束。
- 信箱与落板路径不得写进仓库——一律用 `~/.octos/outer/mcp/` 与
  `~/.octos/outer/board_append.sh` 运行态路径。

## 排除范围

- 外环(claude)侧监视 questions 目录、生成 answers 的接线实现(外环
  自己做)。
- 多外环路由(问 claude 还是 codex):v1 只投外环信箱,路由问题另行
  提案。
- WS/HTTP 传输:v1 仅 stdio。

## 完成条件

S0:本规格过 `agent-spec guard --spec-dir specs --code .`(lint 无
ERROR;新 spec 尚无代码可验,verify 全 skipped 可接受)。

S1(#31 Rust 化后):`cargo test --test olp_mcp_contract` 全绿(真子进程
  `octoscode olp-mcp-serve` stdio 驱动;Python 原型归档于
  scripts/reference/olp-mcp-server.py),逐条对应下列断言:

Scenario: initialize 握手
  测试: self_test_initialize_handshake
  Given server 以 stdio 模式启动
  When 客户端发送 initialize 请求
  Then 返回合法 capabilities(tools 非空)且协议版本与 octos mcp 握手一致

Scenario: tools/list 仅二件
  测试: self_test_tools_list_exactly_two
  When 客户端请求 tools/list
  Then 工具集恰为 ask_outer 与 report_blocked,各带 JSON Schema 入参声明

Scenario: ask_outer 正常往返
  测试: self_test_ask_outer_roundtrip
  Given 外环已在 answers/ 预置对应答案(假答案回灌)
  When 调用 ask_outer(question, context, tried)
  Then questions/ 出现 <ts>-<id>.json 且字段齐全
  And 90s 内取回答案并原样返回
  And 该次 Q/A 经 board_append.sh 落板、署名 MCP(ask_outer)

Scenario: 90s 超时降级
  测试: self_test_ask_outer_timeout_degrades
  Given answers/ 无对应答案
  When 调用 ask_outer 且等待超过 90s(测试中以压缩时钟加速)
  Then 返回降级指引文本(按黑板既有指导继续或 ACK(blocked))
  And 不发生无限阻塞
  And 超时事件同样落板

Scenario: 限额拒绝
  测试: self_test_ask_outer_quota_refusal
  Given 同一切片已问询 3 次
  When 第 4 次调用 ask_outer
  Then 返回拒绝语且 questions/ 不新增文件

Scenario: tried 必填
  测试: self_test_ask_outer_requires_tried
  When 调用 ask_outer 且 tried 为空
  Then 返回拒绝语(要求先自行尝试)且不进入信箱

Scenario: report_blocked 直通落板
  测试: self_test_report_blocked_board_only
  When 调用 report_blocked(reason, needs)
  Then 不经信箱、直接经 board_append.sh 落板署名 MCP(ask_outer)

S2(真机端到端,外环过目配置 diff 后执行):`octos mcp` 挂载 server,
内环一次真实 ask_outer 发问 → 外环接问作答 → turn 内收到答案,全程
落板可查。
