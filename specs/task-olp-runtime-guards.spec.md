spec: task
name: "运行时守卫：harvest UTF-8 截断、Bash 3.2 EVO 编号解析与 GNU 预检（#12c 独立切片）"
id: task-olp-runtime-guards
status: draft
tags: [olp, evolution, harvest, darwin, utf-8]
satisfies: [REQ-OLP-EVO]
estimate: 0.5d
---

## Intent

`scripts/olp-evo-harvest.sh` 在 Darwin/Bash 3.2 与 `LC_ALL=C` 环境下有三个生产输入侧缺陷：
症状截断按 locale 计数（`LC_ALL=C` 下按字节劈开 UTF-8 序列，EVOLUTION.md 不可解码）、
EVO 编号对账用了 Bash 3.2 不支持的 `$'\uff08'` 转义（对账失效、编号重复风险）、GNU
工具（stat -c / realpath -m / sha256sum / flock）缺失时直接启动脚本会在中途以晦涩错误
失败而不是在任何写入之前给出可操作指引。本切片在采集输入侧修复，不改触发器语义、不写
ACK、不改回放夹具、不加 Cargo 依赖，并预留后续 runtime guards（completion/stall）的
扩展位。

## Decisions

- 症状截断改用 `python3 -X utf8 -c`（`char_prefix` 辅助函数）按 **Unicode 字符** 截断，
  与 `LC_ALL` 无关；显式 `-X utf8` 保证 `PYTHONUTF8=0` / `PYTHONCOERCECLOCALE=0` 等环境
  无法把 argv/stdout 编码拉回 locale 依赖形态。python3 本就是脚本硬依赖，不新增依赖。
  events 源内已有的 `line[:200]`（python 侧）行为不变。
- EVO 对账解析改为可移植的“取前导数字"（`[[ $local_id =~ ^([0-9]+) ]]`），随后沿用既有
  非数字告警分支与 `10#` 基数、`NEXT_ID` 推进逻辑；`### EVO-draft` 类标题行为不变。
- GNU 能力预检（phase 2）：与 `scripts/verify.sh` 同形的能力探测（`stat -c '%s'`、
  `realpath -m`，均在 PATH 上），缺失时在 **任何** mkdir/锁/board append 之前 exit 2
  并给出可操作信息（含 brew install coreutils 指引）；已装 GNU 工具时直接启动脚本也能
  自动发现（brew prefix → /opt/homebrew → /usr/local 的 gnubin），不自动安装。逻辑尽量
  复用/对齐 `scripts/verify.sh`，只在必要处抽小型共享辅助。
- 字节偏移 identity/cursor、append-before-state 崩溃恢复语义逐字保持；外环
  `verify/jev-evo-locale-probe` 证据目录只读。
- 预留扩展：本 spec 的 Rule 结构允许后续追加 completion/stall guard 场景，不挤占本切片
  验收。

## Boundaries

### Allowed Changes
- scripts/olp-evo-harvest.sh
- scripts/verify.sh（或小型共享预检辅助）
- tests/olp_evo_harvest.rs
- tests/verify_environment.rs（相关 verify 包装测试）
- specs/task-olp-runtime-guards.spec.md

### Forbidden
- 不改回放夹具（fixtures/**）、既有断言一律保留。
- 不改 Cargo.toml / Cargo.lock，不新增依赖。
- 不改 agent 权限、profile 配置、.claude/**、.octos/**。
- 不写 ACK、不 push、不 merge；只 commit。
- 不安装任何软件；预检只做能力发现。

## Completion Criteria

### Rule: utf8-symptom-truncation — 症状按 Unicode 字符截断

Scenario: LC_ALL=C 长中文 ACK 截断不劈字符
  Test: olp_evo_harvest_lc_all_c_long_chinese_symptom_truncates_by_characters
  Given 活板含 ASCII 前缀的长中文 `ACK(blocked)` 行（200 字节边界会劈开一个字符），环境
    LC_ALL=C 且 PYTHONUTF8=0、PYTHONCOERCECLOCALE=0
  When harvest 两轮
  Then 第一轮成功且 EVOLUTION.md 按 UTF-8 完整解码，symptom ≤ 200 **字符** 且不含残缺
    字符；第二轮零新增

### Rule: evo-id-reconcile — Bash 3.2 可移植的编号对账

Scenario: 全角标题在 Bash 3.2 下正确对账 next_id
  Test: olp_evo_harvest_bash32_parses_fullwidth_evo_heading_and_reconciles
  Given EVOLUTION.md 含 `### EVO-0007（…，harvest）` 全角标题，state 丢失（崩溃后）
  When /bin/bash 运行 harvest
  Then next_id 恢复为 8，无 "non-numeric" 告警，无重复卡

### Rule: gnu-preflight — GNU 能力缺失在任何写入前失败

Scenario: 缺 GNU 能力时先失败、零副作用
  Test: olp_evo_harvest_fails_before_writes_when_gnu_tools_missing
  Given PATH 上无 GNU 形态 stat/realpath（可探测）
  When 运行 harvest
  Then exit 2 发生在任何 mkdir/board 写之前，stderr 含可操作指引；已装 GNU 工具时直接
    启动也被自动发现并成功

### Rule: crash-recovery-idempotent — append 先于 state 的故障恢复幂等

Scenario: 注入 after-append 故障后重跑幂等
  Test: olp_evo_harvest_recovers_after_crash_between_append_and_commit
  Given 注入 OLP_EVO_FAULT=after-append（70 退出）
  When 无注入重跑
  Then 卡片不重复，state 偏移等于源文件大小（既有断言逐字保留）
