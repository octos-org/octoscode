# OctoLoop 功能清单(一页全景)

> OctoLoop = OLP(Outer-Loop Protocol,协议)的产品化封装。本页列出
> 双环系统近期落地的全部能力:是什么 + 缺省状态 + 用户怎么看到效果。
> 深入规程:OUTER_LOOP_PROTOCOL.md(纪律)/ OLP_OUTER_BOOT.md(外环
> 操作面)/ OLP_QUICKSTART.md(上手)。

## 稳性与自治(引擎侧)

- **断供自动降级(fallback 车道)**:provider 断供(quota/auth 拒付)
  时引擎按 profile `llm.fallbacks` 自动换道,不再全线空转。缺省:
  未配 fallbacks 则单道;配了则逐道自动切换。体感:断供期对话继续
  响应而非报错停摆,恢复后主道自动回归。
- **孤儿 peer 回收(Parked)**:父 goal 结束时未终结的 peer 归档为
  Parked 状态而非悬死。缺省:开。体感:`octos peer list` 不再有
  永远 running 的僵尸行。
- **malformed 指令自纠**:修复循环中识别并跳过畸形注入帧,不吞后续
  合法指令。缺省:开。体感:坏帧只丢自己,队列继续走。
- **预算 checkpoint**:goal 按预算档自动落 checkpoint,断档续跑不
  重来。缺省:goal 建立即有。体感:`/goal` 面板 tokens_used 连续。
- **断拍自续(master-sentry)**:旗标开+空闲即自动续拍黑板队列;3 次
  无进展升级外环自停。缺省:外环旗标控制。体感:黑板有条目时内环
  不停摆,队列空则落 QUEUE-EMPTY 旗标。

## 结果与审计(交付侧)

- **result.md 单写者契约**:peer 交付只经 result.md frontmatter
  (slug/outcome/updated_unix/turn)写回,杜绝双写竞争。缺省:开。
  体感:goal ledger 的 findings 干净、可审计。
- **文件变更回执 + scope**:每轮交付列变更文件与影响面,复验按图
  索骥。缺省:master 复验必查。体感:R2 复验命令可直接对着回执跑。
- **写策略三档 + 逃生口**:workspace 只读/受写/host 三档 + 显式逃生
  口,越权写被拒。缺省:workspace。体感:误写仓库外文件直接报错。

## 通信与信道

- **纯 Rust MCP 第五信道**(#31):`octoscode olp-mcp-serve` 子命令,
  turn 内同步问外环(ask_outer/report_blocked 两工具、90s 超时降级、
  每进程限 3 次 + tried 必填、board 审计、取答归档)。缺省:profile
  mcp_servers 挂载即开。体感:内环遇分歧 90s 内拿到外环人工作答,
  超时得降级指引不卡 turn。
- **startup --prompt**(#30):Omarchy 默认 Agent 唤起即用;引导完成
  后恰好一次自动 turn/start(派发前重连补发/派发后不重发)。
  缺省:CLI 旗标。体感:`octoscode --prompt "任务"` 启动即开工,
  TUI 全程可交互。

## 命名

- **OLP** = 协议名(Outer-Loop Protocol,文档与 ACK 定式沿用)。
- **OctoLoop** = 产品名(用户视角的一键化封装;skill 入口与本页)。
