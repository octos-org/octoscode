spec: task
name: "测试只写自己的沙箱:清掉测试进程继承的 OLP_* 环境变量"
id: task-tests-isolate-olp-env
status: implemented
tags: [olp, tests, safety]
estimate: 0.5d
---

## Intent

复盘第 15 条(2026-11-15):在配好正式环境的单元上跑测试,测试换了临时 HOME,却继承了
`OLP_UNIT_CONF`,把一行合成的 `ACK(done): commit abc` 写进了正式 hub 黑板。工具读 30 多个
`OLP_*` 变量决定写到哪里(`OLP_UNIT_CONF`、`OLP_HUB`、`OLP_SSH`、`OLP_MCP_OUTER_ROOT`、
`OLP_HERDR` …),而测试没有一个清掉继承来的值。测试照样要写黑板——写测试自己建的那块。

## Decisions

- 共用模块 `tests/support/olp_env.rs`:放进二进制的初始化列表(Linux `.init_array`、macOS
  `__DATA,__mod_init_func`、Windows `.CRT$XCU`),在测试框架启动前、还没有线程时,删掉进程
  继承的全部 `OLP_*` 变量;测试自己显式设置的变量不受影响。不加依赖。
- 保留白名单 `OLP_MCP_SERVER_BIN`:它选被测的二进制,不指向任何状态。
- 不把变量设成空值:有读取方把空值当路径(如 `OLP_MCP_OUTER_ROOT` 为空即当前目录)。
- 每个集成测试文件开头一行 `#[path = "support/olp_env.rs"] mod olp_env;`;契约测试检查所有
  `tests/*.rs` 都有这一行,新文件漏了即失败。仓库 `AGENTS.md` 写明。

## Boundaries

### Allowed Changes
- tests/support/olp_env.rs
- tests/*.rs(各加一行引用)
- tests/test_isolation.rs
- AGENTS.md(提交纪律一行)
- specs/task-tests-isolate-olp-env.spec.md

### Forbidden
- 不改任何工具读取环境变量的方式。
- 不加依赖。

## Out of Scope

- `cargo test --lib` 的单元测试(不启动写黑板的工具)。
- 别人自己的测试脚本(harness)对正式路由的隔离。

## Completion Criteria

Scenario: 每个集成测试文件都引用了清理模块
  Test: every_integration_test_file_drops_inherited_olp_variables
  Given 仓库的 tests 目录
  When 逐个读取 `tests/*.rs`
  Then 每个文件都含 `#[path = "support/olp_env.rs"]`

Scenario: 继承的 OLP_* 在测试开始前被清掉,测试开关保留(critical)
  Tags: critical
  Test: inherited_olp_variables_are_gone_before_any_test_runs
  Given 把本测试程序作为子进程重启,传入 `OLP_UNIT_CONF`、`OLP_HUB`、`OLP_MCP_OUTER_ROOT` 与 `OLP_MCP_SERVER_BIN`
  When 子进程里的测试列出它看得到的 `OLP_*`
  Then 前三个都不在,`OLP_MCP_SERVER_BIN` 仍在
