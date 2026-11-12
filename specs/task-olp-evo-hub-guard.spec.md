spec: task
name: "进化环主机守卫:harvest 与 retro 只在 hub 上运行"
id: task-olp-evo-hub-guard
status: implemented
tags: [olp, evolution, hub, cluster]
satisfies: [REQ-OLP-EVO]
estimate: 0.5d
---

## Intent

`olp-evo-harvest.sh` 与 `olp-evo-retro.sh` 各自维护本机状态:EVO-NNNN 与 FLAW-NNN 的计数、
三个来源的字节游标、`retro.json`。集群方案里黑板只在 hub 上,其他机器只有只读镜像。如果有人
在非 hub 机器上对着镜像跑一遍采集或 retro,会用另一套计数器分配出与 hub 重复的编号,并把
同一条信号再记一次。本任务加一道主机守卫:配置了 hub 之后,这两个脚本的写入模式只能在 hub
上运行。

## Decisions

- 新文件 `scripts/olp-hub-guard.sh`(被 source),提供 `olp_hub_guard <脚本名>`:通过返回 0,
  拒绝返回 3 并向 stderr 说明本机名与 hub 名。
- hub 主机名的来源,按顺序:环境变量 `OLP_HUB_HOST`;文件
  `${OLP_HUB_HOST_FILE:-~/.octos/outer/hub-host}` 的第一个非空、非 `#` 开头的行(容忍首尾
  空白与 CRLF)。两处都没有(或为空)视为**单机部署,守卫放行**——现有用户的行为不变。
- 比较规则:两边都取第一个标签(去掉第一个 `.` 之后的部分)并转小写。本机名取 `hostname`
  的输出,取不到时退到 `uname -n`;都取不到则拒绝(配置了 hub 却无法确认本机身份时不写)。
- 接入点:两个脚本都只在**非 dry-run** 时调用守卫,且在任何写入(建状态目录、开锁文件、写
  进化板)之前;拒绝时退出码 3。dry-run 不写任何东西,在任何机器上都可用。
- harvest 里守卫放在"活板存在性检查"之后:活板缺失仍先以退出码 2 报告,既有契约不变。
- 脚本被单独拷贝、旁边没有 `olp-hub-guard.sh` 时(`tests/olp_watch_board.rs` 的安装拷贝场景):
  未配置 hub 则照常运行;已配置 hub(环境变量非空或 hub-host 文件存在)则以退出码 3 拒绝
  ——无法核对身份时不写。
- `olp-watch-board.sh --harvest` 不改:它调用 harvest 脚本,拒绝时按既有逻辑打印
  `harvest: failed (exit 3)` 并继续监视。
- 没有绕过开关:要在另一台机器上运行,就改配置把它指定为 hub。
- 插入位置避开了另外两个未合并分支(PR #703、#707)在这两个脚本里的改动区域,三者可以
  按任意顺序合并。

## Boundaries

### Allowed Changes
- scripts/olp-hub-guard.sh
- scripts/olp-evo-harvest.sh(仅守卫接入)
- scripts/olp-evo-retro.sh(仅守卫接入)
- tests/olp_evo_hub_guard.rs
- docs/OCTOLOOP_FEATURES.md
- specs/task-olp-evo-hub-guard.spec.md

### Forbidden
- 不改采集判定、卡片格式、计数器与游标的读写。
- 不改 dry-run 的行为。
- 不改既有退出码(0、1、2、70)的含义。
- 不改 docs/OLP_OUTER_BOOT.md。

## Out of Scope

- 守卫 `olp-board-append.sh`(非 hub 的写板走远程写板,属于另一个工作包)。
- 从 hub 自动下发 hub-host 配置。
- 用主机密钥等更强的身份证明代替主机名。

## Completion Criteria

### Rule: refuse — 非 hub 拒绝写入
Scenario: 非 hub 上 harvest 在任何写入前被拒绝(critical)
  Tags: critical
  Test: evo_harvest_refuses_off_hub_before_any_write
  Given `OLP_HUB_HOST` 指向另一台机器,活板存在
  When 以写入模式运行 `olp-evo-harvest.sh`
  Then 退出码 3,stderr 含脚本名与 hub 名,状态目录未创建,进化板未改动

Scenario: 非 hub 上 retro 在任何写入前被拒绝(critical)
  Tags: critical
  Test: evo_retro_refuses_off_hub_before_any_write
  Given `OLP_HUB_HOST` 指向另一台机器
  When 以写入模式运行 `olp-evo-retro.sh`
  Then 退出码 3,stderr 含脚本名与 hub 名,状态目录未创建,进化板未改动

### Rule: allow — hub 与只读放行
Scenario: 本机就是 hub 时放行(忽略大小写与域名后缀)
  Test: evo_retro_runs_when_this_host_is_the_hub
  Given `OLP_HUB_HOST` 分别写成本机名原样、大写的第一个标签、第一个标签加别的域名后缀
  When 以写入模式运行 `olp-evo-retro.sh`
  Then 三种写法都成功运行并创建状态目录

Scenario: dry-run 在非 hub 上仍可用
  Test: evo_retro_dry_run_is_allowed_off_hub
  Given `OLP_HUB_HOST` 指向另一台机器
  When 以 `--dry-run` 运行 `olp-evo-retro.sh`
  Then 退出码 0,状态目录未创建,进化板未改动

### Rule: config — 配置来源
Scenario: hub-host 文件生效,环境变量优先
  Test: evo_hub_host_file_is_honoured_and_env_overrides_it
  Given hub-host 文件含注释、空行、首尾空白与 CRLF,指向另一台机器
  When 不设环境变量运行 retro;再把 `OLP_HUB_HOST` 设为本机名运行
  Then 第一次以退出码 3 拒绝,第二次成功

Scenario: 未配置 hub 时行为不变(critical)
  Tags: critical
  Test: evo_scripts_run_unguarded_when_no_hub_is_configured
  Given 没有 hub-host 文件且未设环境变量;以及文件只有注释、环境变量为空串
  When 以写入模式运行 `olp-evo-retro.sh`
  Then 两种情况都成功运行

Scenario: 单独拷贝的脚本缺守卫库时,未配置放行、已配置拒绝
  Test: evo_lone_script_copy_without_guard_library_fails_closed
  Given 只拷贝了 `olp-evo-retro.sh` 与 `olp-evo-lib.py` 的目录,没有 `olp-hub-guard.sh`
  When 把 `OLP_HUB_HOST` 设为本机名运行;再不配置 hub 运行
  Then 第一次以退出码 3 拒绝且 stderr 提到 `olp-hub-guard.sh`,状态目录未创建;第二次成功
