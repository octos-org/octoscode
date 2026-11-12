spec: task
name: "outer-duty 主审权锁支持 macOS:守护进程实现死亡耦合(R7 平台范围修订)"
id: task-outer-duty-macos
status: implemented
tags: [olp, outer-duty, macos, r7]
estimate: 1d
---

## Intent

协议 R7 的主审权锁(`octoscode outer-duty hold/check`)此前只支持 Linux:死亡耦合依赖
`PR_SET_PDEATHSIG`,macOS 没有等价的内核机制,命令在 macOS 上显式 unsupported 退出 2。
集群方案把大外环直接放在 Mac server(macOS)上并要求经 `outer-duty hold` 包裹启动,二者不能
同时成立。本任务让锁在 macOS 上可用,且保持 R7 的核心不变量:**wrapper 亡则 agent 必亡,
锁绝不在 agent 存活时显示 VACANT**。Linux 路径逐字保持。

## Decisions

- 进程结构(仅 macOS):`wrapper → guardian → agent`。guardian 是 wrapper 的 fork(不 exec),
  因此继承锁 fd(同一 open file description,同一把 flock);agent 由 guardian fork+exec,
  锁 fd 的 CLOEXEC 保持置位,agent 永不持锁。
- guardian 监视 wrapper:kqueue `EVFILT_PROC`/`NOTE_EXIT`,并以 100ms 间隔轮询 `getppid()`
  兜底。wrapper 亡 → 对 agent 进程组发 SIGKILL → 回收 agent → 确认进程组内无存活成员后才
  退出(放锁)。等待上限 30 秒,超时后放锁,锁不会被永久卡住。
- wrapper 监视 guardian:`waitpid`。guardian 正常退出时其退出码即 agent 的退出码(信号致死为
  128+signal),wrapper 原样返回;guardian 被信号杀死时,wrapper 对 agent 进程组发 SIGKILL、
  等其消失,然后以失败退出,stderr 含 `guardian died abnormally`。其间锁仍由 wrapper 持有。
- guardian 自成进程组并忽略 SIGINT/SIGTERM/SIGHUP/SIGQUIT,终端信号不会同时带走两个监督者;
  agent exec 前恢复这四个信号进入 wrapper 时的原始处置。
- agent 正常退出而孙辈长驻:guardian 不杀进程组,直接转交退出码;孙辈不持 fd,锁为 VACANT
  (与 Linux 的契约 2b 一致)。
- agent exec 失败:stderr 输出 `failed to exec the duty child`,退出码 127。
- fork 之后只使用原始 libc 调用(无分配、无 std I/O);argv 的 CString 在 fork 之前备好。
  `outer-duty` 子命令在 `main` 起任何线程之前分发,fork 时进程为单线程。
- sidecar 的 `wrapper_starttime` / `child_starttime` 在 macOS 上取
  `proc_pidinfo(PROC_PIDTBSDINFO)` 的启动时间(微秒),仍只用于诊断。`child_pid` 记录真实
  agent 的 pid(不是 guardian)。
- 公开签名:`spawn_holder_child` 返回新类型 `HolderChild`(`id()` 为真实 agent pid,`wait()`
  返回 `ExitStatus`),Linux 内部仍是 `std::process::Child`;唯一调用点
  `src/cmd/outer_duty.rs` 同 commit 更新。
- 协议:R7 平台范围由仅 Linux 改为 Linux 与 macOS,保持 `olp/v2`(锁的语义不变,只新增一个
  平台的实现)。Windows 仍 unsupported。
- 与 Linux 的已知差别(写入 R7):macOS 可承受 wrapper 或 guardian 任一被杀,二者同一瞬间均
  被 SIGKILL 时 agent 成为孤儿;wrapper 亡时 macOS 杀整个进程组(含孙辈),严于 Linux。

## Boundaries

### Allowed Changes
- src/outer_duty.rs
- src/cmd/outer_duty.rs
- src/cmd/mod.rs
- tests/outer_duty_contract.rs
- tests/olp_contract.rs
- tests/olp_evo_retro.rs(BOOT §0-§6 的 golden 哈希,随 §3.5 的有意修订更新)
- docs/OUTER_LOOP_PROTOCOL.md
- docs/OLP_OUTER_BOOT.md
- docs/OCTOLOOP_AGENTS.md
- docs/OCTOLOOP_AGENTS.zh-CN.md
- docs/OCTOLOOP_GUIDE.md
- docs/ARCHITECTURE.md
- AGENTS.md
- README.md
- .claude/skills/octoloop/SKILL.md
- specs/task-outer-duty-macos.spec.md

### Forbidden
- 不改 Linux 的死亡耦合实现(setpgid + PR_SET_PDEATHSIG + getppid 复核)。
- 不改锁命名、锁文件权限、`check` 的三态输出与 sidecar 的字段名。
- 不改 Cargo.toml / Cargo.lock(`libc` 已是依赖)。
- 不升协议版本号。

## Out of Scope

- Windows(LockFileEx)支持。
- 跨机器的锁(NFS 或网络锁);R7 仍是单机锁。
- 写入面的硬 gate(board-append/push 校验 lease)。

## Completion Criteria

### Rule: parity — 原有契约在 macOS 上成立
Scenario: 两个竞争者恰好一个获胜(critical)
  Tags: critical
  Test: duty_two_contenders_exactly_one_wins
  Given 两个 `outer-duty hold` 在同一起跑门后同时启动
  When 起跑门放开
  Then 恰好一个退出码为 0,另一个 stderr 为锁竞争诊断,结束后锁为 VACANT

Scenario: wrapper 被 SIGKILL 后真实 agent 死亡且锁释放(critical)
  Tags: critical
  Test: duty_wrapper_death_kills_agent_and_releases
  Given 一个持锁的 wrapper,sidecar 记录了真实 agent 的 pid 与启动时间
  When wrapper 被 SIGKILL
  Then 该 agent(按 pid + 启动时间识别)死亡,随后锁为 VACANT

Scenario: agent 退出而孙辈长驻时锁为 VACANT
  Test: duty_grandchild_lingering_yields_vacant
  Given agent 启动一个后台孙辈后立即退出
  When 观察锁状态
  Then 锁为 VACANT,且此时孙辈仍存活

Scenario: check 不打扰持锁者
  Test: duty_check_does_not_disturb_holder
  Given 一个持锁的 wrapper
  When 反复执行 `outer-duty check`
  Then 每次输出 HELD,持锁者不受影响

### Rule: guardian — macOS 的监督链
Scenario: guardian 被杀后 agent 死亡、wrapper 失败退出(critical)
  Tags: critical
  Test: duty_guardian_death_kills_agent_and_fails_wrapper
  Given 一个持锁的 wrapper 及其 guardian 子进程
  When guardian 被 SIGKILL
  Then wrapper 以非 0 退出且 stderr 含 `guardian died abnormally`,此时真实 agent 已死亡,锁为 VACANT

Scenario: 锁显示 VACANT 时 agent 必已死亡(critical)
  Tags: critical
  Test: duty_vacant_is_never_observed_while_agent_alive
  Given 一个持锁的 wrapper
  When wrapper 被 SIGKILL,并不间断地轮询 `outer-duty check`
  Then 首次观察到 VACANT 时真实 agent 已不存活(连续 5 轮均成立)

### Rule: docs — 协议与文档的平台范围一致
Scenario: 三处权威文档声明 Linux 与 macOS 且不再出现 Linux-only
  Test: outer_duty_docs_pin_guardian_semantics_and_h1
  Given `docs/OUTER_LOOP_PROTOCOL.md`、`docs/OLP_OUTER_BOOT.md`、`AGENTS.md`
  When 读取文本
  Then 均含"守护式死亡耦合"与"Linux 与 macOS",均不含 `Linux-only`
