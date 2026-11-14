spec: task
name: "多机部署:单元登记、心跳与健康状态(派卡前检查)"
id: task-olp-unit-registry
status: implemented
tags: [olp, cluster, unit, health]
estimate: 1d
---

## Intent

集群方案里大外环在 hub 上给十台 Mac mini(单元)派卡。现有脚本没有"机器"的概念:hub 不
知道有哪些单元、哪台机器是哪个单元、它现在是否活着、能不能接卡。本任务提供单元登记(编
组)、单元侧心跳与巡检、hub 侧的状态判定,以及派卡前的检查命令。

设备(D600)的租约与恢复是项目专用的,不在本任务内;它们通过扩展检查目录接进心跳。

## Decisions

- 工具:`scripts/olp-unit.py`,纯 Python 标准库(兼容系统自带的 Python 3.9),子命令
  `register`、`set-state`、`receive`、`status`、`check`(hub)与 `heartbeat`(单元)。
- 登记文件 `${OLP_UNITS_DIR:-~/.octos/outer/units}/registry.tsv`,每行七列:单元号、主机
  名、网卡地址、主机密钥指纹、序列号、enabled、备注。写入在 `registry.tsv.lock` 的 flock
  下、经临时文件原子替换。
- 编组规则:单元号 `^[a-z][a-z0-9-]{0,31}$`(约定 `m01`…);主机名取第一个标签并转小写;
  网卡地址规范成小写冒号分隔。主机名、网卡地址、主机密钥指纹(非空时)在登记文件里各自
  唯一,冲突的登记被拒绝且不改文件。重新登记同一个单元是更新,保留人工标记的离线状态。
- 心跳:`heartbeat --unit <U>` 只做本机巡检并把一条 JSON 打印到 stdout(字段 `v`、
  `unit`、`hostname`、`macs`、`sent_at`、`checks`),不负责传输。传输由调用方的管道完成
  (`… | ssh hub olp-unit.py receive`),所以工具本身不依赖 ssh,可以脱机测试。心跳是状态
  不是日志,hub 不可达时不排队。
- 巡检项:内置 `agent`(herdr 里至少有一个内环,任何一种:octoscode、Claude Code、
  Codex;以 `herdr agent list` 为准,`OLP_HERDR` 可指定 herdr)、`serve`(有 `octos serve`
  进程,只适用于 octos 内环)与 `disk`(HOME 所在磁盘空闲比例 ≥ `--disk-min-free`,默认
  10%),由 `--require` 选择,默认 `agent,disk`。按进程名找 Claude Code、Codex 不可靠——
  开发机上常有别的 claude/codex 进程(外环自己、Codex 的后台服务)——所以以 herdr 报告的
  窗格为准。心跳带 `agents`(每项只有 kind、status、pane);`receive` 只保留三个字段都是
  短的纯 token 的项。扩展检查为
  `${OLP_UNIT_CHECKS_DIR:-~/.octos/outer/unit-checks.d}` 下的可执行文件,退出 0 为通过,
  stdout 第一行为说明,10 秒超时。集群方案里"循环是否暂停、双哨是否在位、fallbacks 是否已
  快照"这几项要读会话内的权威状态面,脚本读不到,留给扩展检查,本任务不实现。
- 启动权限:心跳带 `permission` 与 `backends`。读进程表(`ps -axo pid=,command=`),只算
  argv[0] 的文件名是 `octos` 或 `octos-<后缀>` 且 argv[1] 是 `serve` 的进程——TUI 把同样的
  字样放在一个参数里,不算后端。每个后端只取 `--danger-full-access` 与 `--solo` 两个开关,
  命令行的其余部分不进心跳。Claude Code 与 Codex 内环的启动参数取自 `herdr pane
  process-info`(窗格前台进程的 argv):Claude Code 的 `--dangerously-skip-permissions`、
  `--permission-mode bypassPermissions`,Codex 的
  `--dangerously-bypass-approvals-and-sandbox`、`--yolo`、`--sandbox danger-full-access`
  算完全访问;明确给了别的模式算沙箱;什么都没给算看不出来(权限可能在它们的配置文件里,
  本工具不读),整机记为 `unknown`。`permission` 为 `full`(全带完全访问)、`sandboxed`
  (都不带)、`mixed`、`none`(没有内环)、`unknown`(读不到进程表或 herdr,或有看不出来
  的内环)。`receive` 只接受这五个值,其余或
  缺失记为 `unknown`;`backends` 逐项清洗,未知字段丢弃。`status` 的文本与 JSON 都带这一
  项。它是单元自报的告知信息:不参与健康判定,不是安全边界。
- 接收:`receive` 从 stdin 读一条心跳(上限 256 KiB),单元必须已登记;带 `--unit` 时心跳
  自报的单元号必须与它一致。字段逐个清洗后落盘到 `heartbeat/<单元号>.json`,并写入 hub
  时钟的 `received_at`。`ok` 只有 JSON `true` 才算通过。
- 信任模型:信任锚是 ssh——hub 的 `authorized_keys` 给每个单元的密钥配强制命令
  `olp-unit.py receive --unit <U>`,一个单元无法冒充另一个。网卡地址与主机名的核对用来发现
  接错线、换了机器之类的问题,不是安全凭据。主机密钥指纹只登记并保证唯一,供 hub → 单元
  方向固定 known_hosts,本工具不做连接时校验。
- 状态判定(hub):`offline` 人工标记;`stale` 没有心跳或 `received_at` 距今超过
  `--stale-after`(默认 180 秒,即三次心跳);`degraded` 心跳按时但巡检有项不过,或上报的
  主机名不等于登记值,或登记的网卡地址不在上报的地址里;其余为 `healthy`。失联只按 hub 的
  时钟算。
- 派卡前检查:`check <U>` 仅在 `healthy` 时退出 0;否则退出 1 并在 stderr 给出原因。它是
  一个独立命令,不嵌进写板脚本——是否派卡仍由大外环决定。
- 退出码:0 成功;1 `check` 未通过;2 用法或输入错误;3 心跳被拒收。
- 不自动改派卡、不自动接管主审权锁、不恢复会话(写入文档)。

## Boundaries

### Allowed Changes
- scripts/olp-unit.py
- tests/olp_unit.rs
- docs/OLP_CLUSTER.md
- specs/task-olp-unit-registry.spec.md

### Forbidden
- 不改任何现有脚本、协议文档与 AGENTS.md。
- 不引入第三方 Python 依赖。
- 不在工具里发起网络连接。

## Out of Scope

- launchd 定时与保活(下一个工作包)。
- 设备租约、设备健康与三级恢复(项目仓库)。
- 全景页面上的集群视图。
- 单元内复验 agent 的打回权限(协议文档,待两个单元试点后)。

## Completion Criteria

### Rule: registry — 编组
Scenario: 登记时单元号、主机名、网卡地址、主机密钥各自唯一(critical)
  Tags: critical
  Test: unit_register_enforces_unique_identity
  Given 已登记单元 m07(主机名、网卡地址以不规范的大小写与分隔符给出)
  When 以相同的主机名、网卡地址或主机密钥登记 m08;以非法的单元号或网卡地址登记;把 m07 标记离线后重新登记
  Then m07 的登记值被规范化;三次冲突与两次非法输入都以退出码 2 拒绝且登记文件不变;重新登记更新网卡地址并保留离线标记

### Rule: health — 状态判定
Scenario: 四种状态的判定与派卡前检查(critical)
  Tags: critical
  Test: unit_status_classifies_healthy_degraded_stale_offline
  Given 四个已登记单元:m01 心跳正常,m02 巡检有项不过,m03 从未上报,m04 心跳正常但被人工标记离线
  When 在 hub 上查看状态并逐个执行 `check`
  Then 状态依次为 healthy、degraded、stale、offline,只有 m01 可派卡;`check` 对 m01 退出 0,对其余退出 1,对未登记单元退出 2;m04 恢复 online 后为 healthy

Scenario: 主机名或网卡地址与登记不符时降级
  Test: unit_status_flags_identity_mismatch
  Given m01 的心跳主机名与登记不同,m02 的心跳里没有登记的网卡地址
  When 查看状态
  Then 两者都是 degraded,原因分别指出主机名不符与网卡地址不符

Scenario: 是否失联按 hub 收到心跳的时间算
  Test: unit_stale_is_judged_by_hub_receive_time
  Given 一条 `sent_at` 在遥远未来的心跳被 hub 接收,随后把落盘的 `received_at` 改早 1000 秒
  When 以默认上限和 `--stale-after 2000` 分别查看
  Then 刚接收时为 healthy;改早后默认为 stale 且 `check` 退出 1;上限放宽到 2000 秒时为 healthy 且 `check` 退出 0

### Rule: receive — 接收与拒收
Scenario: 未登记或与强制命令不符的心跳被拒收(critical)
  Tags: critical
  Test: unit_receive_rejects_unregistered_and_mismatched_unit
  Given 只登记了 m01
  When 接收 m02 的心跳;以 `--unit m02` 接收自报 m01 的心跳;以 `--unit m01` 接收同一条心跳
  Then 前两次以退出码 3 拒收且不落盘,第三次成功落盘

Scenario: 畸形心跳不落盘
  Test: unit_receive_rejects_malformed_input
  Given 非 JSON、非对象、缺 unit、unit 含路径字符、超过 256 KiB 的输入;以及字段类型不对的心跳
  When 分别交给 `receive`
  Then 前五种以退出码 2 拒绝且心跳目录未创建;字段类型不对的心跳被清洗后接收,非布尔的 `ok` 记为未通过

### Rule: heartbeat — 单元侧巡检
Scenario: 心跳带上内置巡检与扩展检查的结果
  Test: unit_heartbeat_reports_builtin_and_plugin_checks
  Given 扩展检查目录里有一个通过的检查、一个失败的检查和一个不可执行的文件
  When 运行 `heartbeat --require disk`,并把输出交给 hub 侧的 `receive`
  Then 心跳里依次是 disk、通过的检查、失败的检查(不可执行的文件被忽略);hub 判为 degraded,原因含失败检查的名字与说明,且没有身份不符;未知的内置巡检项以退出码 2 拒绝

Scenario: 心跳报告内环后端的启动权限
  Test: unit_heartbeat_reports_launch_permission
  Given 进程表里有 TUI 包裹进程、两个带完全访问的后端(其中一个的命令行含密钥)、一个无关进程;以及只有不带完全访问的后端、两种混合、只有 TUI 的进程表
  When 分别生成心跳;把带额外字段与畸形条目的 `backends` 交给 hub;再交给 hub 一个未知的权限值和一条不带该字段的旧心跳
  Then 权限依次为 full、sandboxed、mixed、none;后端只含 pid 与两个开关,密钥不出现在心跳里;hub 的状态显示 full 且健康状态不受影响,额外字段与畸形条目被丢弃;未知值与缺失都显示为 unknown

Scenario: 心跳经 herdr 认出任何一种内环(critical)
  Tags: critical
  Test: unit_heartbeat_recognises_any_kind_of_inner_loop
  Given 一个替身 herdr,依次报告:没有内环;一个 Claude Code 和一个 Codex 内环;带各种启动参数的单个内环;读不到
  When 运行心跳,并把一条 Claude Code 单元的心跳交给 hub
  Then 没有内环时 agent 检查不过、权限为无内环;两种内环都让 agent 检查通过并列出 kind 与状态,没带完全访问参数时权限为未知;七组启动参数分别判为完全访问或沙箱,与 octos 后端混合时为混合;读不到 herdr 时检查不过并说明原因、权限为未知;hub 按默认检查把 Claude Code 单元判为正常,status 显示内环一列;伪造的 agents 项被丢弃,多余字段不落盘
