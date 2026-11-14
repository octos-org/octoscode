spec: task
name: "outer-duty hold 把终端前台交给交互式 agent"
id: task-outer-duty-terminal
status: implemented
tags: [olp, outer-duty, r7, terminal]
estimate: 0.5d
---

## Intent

Mac server 上的预检(2026-11-14,战役 nh-hk1000-p1)发现:经 `octoscode outer-duty hold`
启动的交互式 agent 一读终端就被 SIGTTIN 挂起。原因是 wrapper 让 agent 自成进程组(死亡耦合
需要按组杀),却没有把终端前台交给这个组,于是 agent 处在后台组里。R7 要求外环必须经 `hold`
启动,而外环本身就是交互式的 Claude Code 或 Codex,两者不能同时成立。本任务让 `hold` 在终端
前台启动时把前台交给 agent,并让 ^Z / `fg` 照常可用;死亡耦合与锁的语义不变。

## Decisions

- 是否交接只看启动时:stdin、stdout、stderr 中第一个是终端、且其前台进程组就是 wrapper 所在
  组的,即为要交接的终端;都不满足(在后台启动、没有终端)则行为与以前完全相同。
- agent 在 `setpgid(0,0)` 之后、exec 之前调用 `tcsetpgrp` 把自己的组设为前台;调用期间忽略
  SIGTTOU,之后恢复原处置。Linux 在 `pre_exec` 里做,macOS 在 guardian fork 出的 agent 分支
  里做;两处都只用异步信号安全的调用。
- agent 结束后(无论正常退出、被信号杀死,还是 macOS 上 guardian 异常死亡),wrapper 在终端前台
  不是自己的组时把它收回。
- ^Z 转发:agent 被挂起时,先把终端交回 wrapper 的组,再让 wrapper 停下(shell 看到的是一个
  普通的已停止作业,锁仍持有);wrapper 被继续后,若终端前台是 wrapper 的组(`fg`),交回给
  agent,然后对 agent 的组发 SIGCONT;若是 `bg`,只发 SIGCONT。
  - Linux:wrapper 是 agent 的父进程,用 `waitpid(WUNTRACED)` 看到挂起,对自己发 SIGSTOP,
    返回即表示已被继续。
  - macOS:guardian 是 agent 的父进程,看到挂起后对 wrapper 发 SIGSTOP,先等它确实停下(上限
    1 秒),再以 100ms 间隔轮询 `proc_pidinfo` 的进程状态,直到 wrapper 再次运行或已死亡。
- ^C 在交接后只发给 agent 的组:agent 自己处理或退出,wrapper 转交其退出码(130)。
- `spawn_holder_child` 与 `HolderChild` 的公开签名不变,唯一调用点不改。

## Boundaries

### Allowed Changes
- src/outer_duty.rs
- tests/outer_duty_contract.rs
- docs/OUTER_LOOP_PROTOCOL.md
- specs/task-outer-duty-terminal.spec.md

### Forbidden
- 不改死亡耦合(Linux 的 setpgid + PR_SET_PDEATHSIG + getppid 复核;macOS 的 guardian 监督链)。
- 不改锁命名、锁文件权限、`check` 的三态输出与 sidecar 字段。
- 不改 Cargo.toml / Cargo.lock。
- 不升协议版本号(锁与信道语义不变)。

## Out of Scope

- Codex 共享 app-server 守护进程不受 R7 锁约束的问题(另立条目)。
- Windows。

## Completion Criteria

### Rule: terminal — 交互式 agent 拿到终端
Scenario: 在终端前台启动时 agent 能读终端输入(critical)
  Tags: critical
  Test: duty_interactive_agent_reads_the_terminal
  Given 一个作为伪终端会话首进程且处于前台的 `outer-duty hold`,agent 读一行终端输入
  When 向终端输入 `hello` 并回车
  Then agent 读到 `hello` 并以 3 退出,wrapper 的退出码为 3,锁为 VACANT

Scenario: ^Z 挂起 agent 时 wrapper 随之挂起,继续后 agent 重新拿到终端(critical)
  Tags: critical
  Test: duty_interactive_agent_stop_is_relayed
  Given 同样的 wrapper,agent 已读完第一行、正在读第二行
  When 向终端输入 ^Z,确认 wrapper 已停止后对它发 SIGCONT,再输入第二行
  Then wrapper 停止期间锁为 HELD;继续后 agent 读到第二行并以 7 退出,wrapper 的退出码为 7
