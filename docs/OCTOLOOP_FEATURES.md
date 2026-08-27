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
- **孤儿 peer 恢复态(Parked)**:serve **重启**导致 client 绑定丢失的
  peer_handoff 孤儿转为 Parked 可恢复态(其余孤儿仍真 Failed)。
  缺省:开。体感:重启后 agents 栏出现 parked·orphaned across
  restart 而非 failed。
- **malformed 自纠**:模型自己产出的畸形 tool_call 参数,诊断作为
  tool result 喂回模型自纠,限每 turn 3 次,耗尽才终止(stream 层
  不可重试语义不变)。缺省:开。体感:畸形参数不直接终结 turn,
  模型拿到纠错反馈重发。
- **预算 checkpoint**:50 轮迭代耗尽且工作树脏时,自动 wip commit +
  阶段版 result(有 .result-owner 时写 result.checkpoint.md,不覆盖
  peer 终稿),goal 转 budget_exhausted 独立状态。缺省:开。体感:
  超时任务的工作不再全丢,可从 checkpoint 续。
- **turn-continuation 钩子**:活 goal 的 turn 之间零延迟自动续拍
  (引擎特性)。缺省:开。体感:goal 推进不再等外环唤醒节拍。
  (备注:master-sentry 是外环侧的兜底哨兵,非引擎机制。)

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
