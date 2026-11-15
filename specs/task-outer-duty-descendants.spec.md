spec: task
name: "外层亡后 agent 的整棵后代树都死,锁才释放"
id: task-outer-duty-descendants
status: implemented
tags: [olp, outer-duty, r7, guardian]
estimate: 1d
---

## Intent

实测(2026-11-15,codex-cli 0.169.0 `--no-daemon`):Codex 把它执行的命令放进新的进程组。
wrapper 被 SIGKILL 后,原实现只杀 agent 的进程组(macOS)或只让内核杀 agent 本身(Linux
PDEATHSIG),那条命令活下来;锁已 VACANT,它跑完后照样写文件——锁释放之后仍有写入。本任务
让"wrapper 亡 ⇒ agent 及其启动的一切亡,锁只在它们全部消失后才 VACANT"在两个平台上成立。

## Decisions

- 两个平台统一为 `wrapper → guardian → agent`:guardian 是 wrapper 的 fork(不 exec),继承
  锁 fd,在 wrapper 亡后继续持锁。Linux 原来"wrapper 是唯一 fd 持有者 + agent 的 PDEATHSIG"
  的结构撤掉,因为 wrapper 一死锁就释放,来不及清理。
- wrapper 亡时 guardian 杀 agent 的整棵后代树:扫描进程表(macOS `proc_listallpids` +
  `proc_pidinfo`;Linux `/proc/<pid>/stat`),取 guardian 之下的全部后代;先 SIGSTOP,重复扫描
  直到不再出现新成员(冻结,防止边找边 fork 逃走),再 SIGKILL;全部消失(不存在或僵尸)后才
  退出放锁,上限 30 秒。不论成员换到了哪个进程组或会话。
- Linux:guardian 与 wrapper 都设 `PR_SET_CHILD_SUBREAPER`,孤儿后代过继给它们而不是 init,
  因此始终在扫描范围内;guardian 平时回收过继来的僵尸。agent 另设对 guardian 的
  `PR_SET_PDEATHSIG(SIGKILL)` 作为兜底。guardian 用 pidfd 等 wrapper 与 agent 退出,并每 100ms
  复查 `getppid()`。
- guardian 异常死亡:wrapper 仍持锁,杀掉剩下的树(Linux:过继给自己的全部后代;macOS:以 agent
  为根的树)后以失败退出。
- 终端交接与 ^Z 转发在 Linux 上也改由 guardian 完成(与 macOS 相同),行为不变。
- agent 自己正常退出时不杀长驻的孙辈(原契约 2b 不变)。
- 进程表扫描用定长缓冲(16384 个进程、一棵树 4096 个),fork 之后不分配内存;Linux 的
  `opendir` 在单线程进程里调用。

## Boundaries

### Allowed Changes
- src/outer_duty.rs、src/cmd/outer_duty.rs、src/cmd/mod.rs(注释)
- tests/outer_duty_contract.rs
- docs/OUTER_LOOP_PROTOCOL.md、docs/OCTOLOOP_AGENTS.md、docs/OCTOLOOP_AGENTS.zh-CN.md、
  docs/ARCHITECTURE.md、AGENTS.md、skills/octoloop/AGENTS.md(外环第 4 步)
- specs/task-outer-duty-descendants.spec.md

### Forbidden
- 不改锁命名、锁文件权限、`check` 的输出、sidecar 字段。
- 不改 `docs/OLP_OUTER_BOOT.md` §0–§6 与其 golden 哈希(需 operator 批准,见 Out of Scope)。
- 不升协议版本号。

## Out of Scope

- `docs/OLP_OUTER_BOOT.md` §3.5 仍写着"Linux:wrapper 是唯一锁 fd 持有者,agent 经
  PR_SET_PDEATHSIG 与 wrapper 同死",已不准确;它在哈希保护范围内,改它要同时更新
  `tests/olp_evo_retro.rs` 的 golden,等 operator 批准。
- macOS 上已经离开这棵树(父进程先退出、过继给 launchd)的后代看不到;wrapper 与 guardian
  同一瞬间均被 SIGKILL 时不保证。
- agent 正常退出后长驻的孙辈。

## Completion Criteria

Scenario: wrapper 被杀后,agent 放进新会话的命令在锁 VACANT 前已死,之后不再写入(critical)
  Tags: critical
  Test: duty_wrapper_death_kills_commands_in_other_sessions
  Given 一个持锁的 wrapper,agent 启动了一条在新会话里运行、3 秒后写文件的命令
  When wrapper 被 SIGKILL,并不间断地轮询 `outer-duty check`
  Then 首次观察到 VACANT 时那条命令已死;4 秒后文件不存在

Scenario: guardian 被杀后,wrapper 杀掉 agent 与新会话里的命令再失败退出(critical)
  Tags: critical
  Test: duty_guardian_death_kills_commands_in_other_sessions
  Given 同样的 wrapper 与命令
  When guardian 被 SIGKILL
  Then wrapper 以非 0 退出,此时那条命令已死,锁为 VACANT;4 秒后文件不存在

Scenario: guardian 被杀后 agent 死亡、wrapper 失败退出(两个平台)
  Test: duty_guardian_death_kills_agent_and_fails_wrapper
  Given 一个持锁的 wrapper 及其 guardian 子进程
  When guardian 被 SIGKILL
  Then wrapper 以非 0 退出且 stderr 含 `guardian died abnormally`,真实 agent 已死,锁为 VACANT

Scenario: 锁显示 VACANT 时 agent 必已死亡(两个平台)
  Test: duty_vacant_is_never_observed_while_agent_alive
  Given 一个持锁的 wrapper
  When wrapper 被 SIGKILL,并不间断地轮询 `outer-duty check`
  Then 首次观察到 VACANT 时真实 agent 已不存活(连续 5 轮)

Scenario: agent 正常退出时长驻的孙辈不受影响
  Test: duty_grandchild_lingering_yields_vacant
  Given agent 启动一个后台孙辈后立即退出
  When 观察锁状态
  Then 锁为 VACANT,孙辈仍存活
