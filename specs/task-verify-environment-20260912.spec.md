spec: task
name: "统一验证环境入口（verify-environment）"
id: task-verify-environment-20260912
status: implemented
tags: [verify, environment, ci, cross-platform]
---

## Intent

仓库的脚本与测试依赖 GNU 形态的 `stat -c` / `realpath -m`，而 macOS 默认 BSD 工具
不支持；嵌套 Python 子进程也不继承父解释器的 `-B`，会写 `__pycache__`。此前各处
自行配置 PATH/环境变量，行为漂移。本任务提供一个统一验证入口
`scripts/verify.sh`，把这两类环境修正收敛到一处，并让 CI 的既有验证步骤真正
经由该入口运行（整个测试树继承同一环境），同时以真实子进程回归钉住入口契约。

## Decisions

1. `scripts/verify.sh` 是唯一验证入口：export `PYTHONDONTWRITEBYTECODE=1`（整个
   子进程树）；默认梯子 `cargo fmt --all --check` → `cargo clippy --all-targets --
   -D warnings` → `cargo test --all-targets`，任一步失败立即以其实际退出码退出
   （不吞错）；`-- <cmd>` 直通模式在已固定的环境中 exec 任意命令，退出码原样传播；
   未知参数或裸 `--` 在 GNU 预检前打印 usage 并以 64 退出。
2. Darwin 上的 GNU 能力解析：先做**能力探测**——PATH 中 `stat -c` 与 `realpath -m`
   均可用（GNU 形态）则直接采用，不要求 Homebrew；探测失败才回落 gnubin 解析
   （活动 `brew --prefix coreutils` 前缀优先，其后 `/opt/homebrew`、`/usr/local`
   库存前缀），prepend 后**重新探测**；仍不可用则打印含安装指引的明确失败并以
   127 退出（不自动安装、不改全局）。Linux 原生 GNU，跳过探测。混合 PATH（GNU
   stat + BSD realpath）不满足能力门，同样回落。
3. CI 不新增独立 job：既有 `fmt` / `test` / `clippy` job 的验证步骤（含
   `cargo test --doc`）改为 `scripts/verify.sh -- cargo …`，使整套测试树（而非
   仅个别步骤）继承统一环境；`tests/verify_environment.rs` 由既有
   `cargo test --all-targets` 覆盖，`ci-required` 聚合门不变。
4. 回归测试仅用 std（可 `rustc --test` 独立编译）：覆盖脚本存在可执行、GNU 能力
   解析或明确 127 指引、真嵌套 Python（父解释器经 subprocess 启动子解释器）无
   `__pycache__`、自定义命令退出码传播、未知参数/裸 `--` 的 usage 64、混合 PATH
   回落、以及受控 GNU 缺失场景。受控场景一律用可执行 shim 构造 PATH（委托或
   拒绝式 stub），不依赖宿主机特定前缀，不在条件不满足时静默跳过；缺 GNU 负例通过
   uname/brew/stat/realpath 四 shim 确定性触达入口的 127 指引分支（含伪 gnubin
   prepend 后 re-probe 失败），Linux/macOS 行为一致。

## Boundaries

### Allowed Changes
scripts/verify.sh
tests/verify_environment.rs
.github/workflows/ci.yml
specs/task-verify-environment-20260912.spec.md
docs/superpowers/plans/2026-09-12-verify-environment.md

### Forbidden
不改全局/用户 shell 配置；不自动安装依赖；不修改 olp-review-evidence/monitor 相关文件；不引入全局 Cargo 配置变更。

## Completion Criteria

### Rule: verify-entry-contract — 统一入口的环境与退出码契约

Scenario: 脚本存在且可执行
  Test:
    Package: octoscode
    Filter: verify_script_exists_and_is_executable
  Given 仓库已检出
  When 检查 scripts/verify.sh
  Then 文件存在且带执行位

Scenario: GNU 工具解析或明确失败
  Test:
    Package: octoscode
    Filter: gnu_tools_resolved_or_clear_failure
  Given 通过 verify.sh 运行 GNU 形态 stat -c '%s'
  When 在 Linux 或已解析 GNU 能力的 Darwin
  Then 成功输出文件大小；Darwin 缺 GNU 时 127 且指引含 coreutils 与 brew

Scenario: 已具 GNU 能力的 PATH 直接可用
  Test:
    Package: octoscode
    Filter: already_gnu_path_is_used_without_brew_requirement
  Given 宿主已有可委托的 GNU 工具，PATH 前置 stat/realpath 委托 shim（Darwin 缺 GNU 时 fixture 会响亮失败）
  When verify.sh -- stat -c '%s'
  Then 能力探测通过直接成功——不要求 brew

Scenario: 混合 PATH（GNU stat + BSD realpath）不满足能力门
  Test:
    Package: octoscode
    Filter: mixed_path_with_bsd_realpath_still_resolves_gnu
  Given 宿主已有可回落的 GNU 工具，PATH 前置 GNU 形态 stat shim 与拒绝式 realpath stub（无其他 GNU 源在前）
  When verify.sh -- realpath -m /etc/../etc/hosts
  Then 能力门不通过（realpath -m 亦是必检项）→ 回落解析后 realpath -m 正常归一输出

Scenario: 嵌套 Python 不写字节码
  Test:
    Package: octoscode
    Filter: nested_python_inherits_no_bytecode
  Given verify.sh → python3（父）→ subprocess python3（子）导入临时模块
  When 子进程运行
  Then 模块值正确且无 __pycache__ 产生

Scenario: 自定义命令退出码原样传播
  Test:
    Package: octoscode
    Filter: custom_failure_exit_code_propagates
  Given verify.sh -- sh -c 'exit 7'
  When 命令结束
  Then 脚本退出码恰为 7

Scenario: 未知参数与空 -- 打印 usage 非零退出
  Test:
    Package: octoscode
    Filter: unknown_argument_and_empty_dash_print_usage
  Given verify.sh --bad 或裸 --
  When 参数解析
  Then 退出码 64 且 stderr 含 usage 文本

Scenario: 确定性缺失 GNU 负例在脚本内响亮失败
  Test:
    Package: octoscode
    Filter: controlled_missing_gnu_fails_loudly_inside_script
  Given shim 全控 PATH：uname 恒报 Darwin、stat/realpath 均拒绝 GNU、brew shim 的 --prefix coreutils 指向存在但同样拒绝的伪 gnubin
  When 运行 GNU 形态 stat
  Then 回落 prepend 后 re-probe 仍失败 → 脚本自发 127 且含 coreutils/brew 指引（两平台确定性触达，无宿主前缀依赖）
