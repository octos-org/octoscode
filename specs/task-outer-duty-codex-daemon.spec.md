spec: task
name: "outer-duty hold 拒绝不带 --no-daemon 的交互式 codex"
id: task-outer-duty-codex-daemon
status: implemented
tags: [olp, outer-duty, r7, codex]
estimate: 0.5d
---

## Intent

Mac server 预检(2026-11-14)发现 R7 的锁约束不到 Codex。实测(codex-cli 0.169.0):不带
`--no-daemon` 时,交互式 Codex 的会话在共享的后台服务(`codex app-server daemon`)里运行,
终端程序只是客户端;它执行的命令挂在后台服务下面,不是 wrapper 的后代。wrapper 被 SIGKILL 后
锁立即 VACANT,而那条命令照常跑完并写了文件——锁释放之后仍有写入,正是 R7 要防的双主审。
Codex 退出时也明说 "Disconnected from this task. Any running work continues."。带
`--no-daemon` 时整个 Codex 是 wrapper 下的一个进程,命令是它的后代。

## Decisions

- `outer-duty hold` 在取锁之前检查被包裹的命令:程序名(路径的最后一段)是 `codex`,且参数里
  没有 `--no-daemon`,也不是在本进程里运行的 `exec`(别名 `e`)、`review` 或帮助/版本,就拒绝:
  stderr 说明原因并给出加上 `--no-daemon` 的完整命令,退出码 2,不取锁,程序不运行。
- 只认直接写出的 `codex`;经别的程序间接启动(如 `env codex`、`npx codex`)不检查,由文档约束。
- 协议 R7 写明:锁只约束 wrapper 的后代;外环用 Codex 时必须以 `codex --no-daemon` 启动。
- 上岗卡、skill 外环第 4 步、内环文档里 Codex 的启动写法加上 `--no-daemon`(内环不受锁约束,
  但同样的原因会让关掉窗格后工作继续,负哨守护盯着的也不是干活的进程)。skill 的「模式 inner」
  一节受哈希保护,不改,由内环文档说明。

## Boundaries

### Allowed Changes
- src/cmd/outer_duty.rs
- tests/outer_duty_contract.rs
- docs/OUTER_LOOP_PROTOCOL.md
- docs/OCTOLOOP_AGENTS.md
- docs/OCTOLOOP_AGENTS.zh-CN.md
- docs/OLP_INNER_AGENTS.md
- skills/octoloop/AGENTS.md
- specs/task-outer-duty-codex-daemon.spec.md

### Forbidden
- 不改锁本身、死亡耦合、`check` 的输出。
- 不改 `docs/OLP_OUTER_BOOT.md` 的 §0–§6 与 skill 的「模式 init / inner」「自主性纪律」(哈希保护)。
- 不升协议版本号。

## Out of Scope

- agent 自己把执行的命令放进别的进程组(Codex 就这样做)时,wrapper 亡后这些命令不会被杀,
  属于 R7 已写明的"孙辈长驻"边界,另立条目。
- 心跳的 `full` 巡检检查 Codex 是否带 `--no-daemon`。

## Completion Criteria

Scenario: 不带 --no-daemon 的交互式 codex 被拒绝,带上后与 exec 照常运行(critical)
  Tags: critical
  Test: duty_refuses_codex_without_no_daemon
  Given 一个名为 `codex` 的可执行程序,运行时把参数记进文件
  When 经 `outer-duty hold` 依次以 `codex --dangerously-bypass-approvals-and-sandbox`、`codex --no-daemon --dangerously-bypass-approvals-and-sandbox`、`codex exec hi` 启动
  Then 第一次退出码 2、stderr 提到 `--no-daemon`、程序没有运行、锁文件不存在;后两次退出码 0,文件里依次是这两次的参数,结束后锁为 VACANT
