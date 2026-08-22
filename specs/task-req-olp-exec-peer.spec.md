spec: task
name: "执行硬化:peer 工具链、默认隔离、机制化验证(octos)"
tags: [olp, peer, verification, octos, upstream]
depends: [task-req-olp-obs-cli]
satisfies: [REQ-OLP-EXEC]
estimate: 2d
---

## 意图

实测三类执行可信度缺陷:peer 的 shell 缺 cargo 只能交付"未验证"结果;
多写者共用工作区仅靠 AGENTS.md 纪律兜底;内环两次以 lib-only 测试的
绿色失实声称"已验证"。本任务把验证从纪律降为机制:工具链继承、worktree
默认隔离与完成即清、profile 级 verify_command 自动执行并落账。实施仓库
为 **octos**。

## 已定决策

- peer/master 工具 shell 的 PATH 解析顺序:profile `tool_path` 显式配置
  优先,否则继承 serve 进程环境的 PATH;两者皆不可用(探测不到基础工具)
  时在 result frontmatter 写 `toolchain: unavailable`。
- `peer_handoff` 的 worktree 缺省改为 profile 可配
  (`peer_worktree_default`);开启时 peer closed 且成果已被 gather 后,
  runtime 自动删除 `peers/<slug>/wt/`(operator 拍板:默认开+完成即清);
  `result.md`、账本、brief 历史一律保留。
- profile 新增 `verify_command`(字符串,operator 手写);goal-scoped
  peer 交付时 runtime 在 peer cwd 执行,结果写入 result.md frontmatter
  `verified: pass|fail|skipped(<原因>)` 并记入 goal 账本;分析类 brief
  可经 handoff 参数 `verify: false` 显式跳过(落 `skipped(brief opt-out)`)。
- `verify_command` 只读自 profile 配置文件;模型工具、黑板、steer、brief
  均无写入路径——不新增任何可改写它的接口。

## 边界

### Allowed Changes(octos 仓库)
- crates/octos-cli/src/peers/**
- crates/octos-cli/src/commands/chat.rs
- crates/octos-cli/src/commands/serve.rs
- crates/octos-cli/src/config.rs
- crates/octos-agent/src/tools/peer_handoff.rs
- crates/octos-cli/tests/**

### Forbidden
- 不改 peer depth-1、brief 64KB、slug 唯一性等治理约束。
- verify_command 不得进入任何模型可写的配置面。
- 不删除 result.md/账本/brief 历史(清理仅限 wt/)。
- clone --no-hardlinks 的隔离语义不变(不回退到共享对象库)。

## 排除范围

- verified 字段的消费端(外环/octoscode 展示)。
- per-peer token 预算。
- verify 结果的重试/自动整改策略。

## 完成条件

场景: peer 工具链继承(critical)
  测试:
    包: octos-cli
    过滤: olp_exec_peer_inherits_operator_path
  假设 serve 环境 PATH 含 cargo 且 profile 未配 tool_path
  当 peer 执行 bash 工具运行 cargo --version
  那么 命令成功且输出版本号

场景: 工具链不可用被如实声明
  测试:
    包: octos-cli
    过滤: olp_exec_toolchain_unavailable_declared
  假设 tool_path 与 PATH 均无法解析基础工具
  当 peer 完成交付
  那么 result.md frontmatter 含 toolchain: unavailable

场景: 交付触发机制化验证并落账(critical)
  测试:
    包: octos-cli
    过滤: olp_exec_verify_runs_and_records
  假设 profile 配置 verify_command 为一条可通过的命令
  当 goal-scoped peer 完成交付
  那么 result.md frontmatter 含 verified: pass 且账本记录同值

场景: 验证失败的交付被如实标记
  测试:
    包: octos-cli
    过滤: olp_exec_verify_failure_recorded
  假设 verify_command 以非零退出
  当 peer 声称完成并交付
  那么 verified: fail 且 events.jsonl 出现对应事件

场景: steer 与黑板无法改写 verify_command
  测试:
    包: octos-cli
    过滤: olp_exec_verify_command_immutable_from_model
  假设 一条要求修改 verify_command 的模型侧输入被处理
  当 检查 profile 配置文件
  那么 verify_command 保持 operator 原值不变

场景: worktree 完成即清且成果保留
  测试:
    包: octos-cli
    过滤: olp_exec_worktree_cleanup_after_gather
  假设 profile 开启 peer_worktree_default 且 peer 已 closed 并被 gather
  当 清理逻辑运行
  那么 peers/<slug>/wt/ 被删除而 result.md 与账本条目保留

场景: 未 gather 前不清理
  测试:
    包: octos-cli
    过滤: olp_exec_no_cleanup_before_gather
  假设 peer 已 closed 但成果尚未被 gather
  当 清理逻辑运行
  那么 wt/ 仍存在
