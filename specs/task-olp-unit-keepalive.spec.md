spec: task
name: "多机部署:单元保活——用 launchd 自动拉起常驻任务(macOS)"
id: task-olp-unit-keepalive
status: implemented
tags: [olp, cluster, unit, launchd, macos]
estimate: 0.5d
---

## Intent

单元(Mac mini)上的心跳、负哨转发等任务必须一直在跑:机器重启或进程崩溃后要自己回来,
不能靠人登录上去重新启动。本任务提供把命令注册成 launchd 用户任务的工具、心跳任务的预设,
以及电源设置的检查。同时明确哪些东西不自动恢复:会话与主审权锁属于权威,自动恢复会制造
两个主审。

## Decisions

- 工具 `scripts/olp-launchd.py`(纯 Python 标准库,兼容 Python 3.9),子命令 `heartbeat`、
  `install`、`uninstall`、`list`、`render`、`doctor`。
- 任务形态两种,必须且只能选一种:`--interval <秒>`(10–86400,`StartInterval`)或
  `--keepalive`(`KeepAlive` + `ThrottleInterval 10`)。都带 `RunAtLoad`。
- 任务自己的命令写在 `--` 之后,在 argparse 之前切开,原样成为 `ProgramArguments`——命令里的
  参数不会被当成本工具的参数,含空格的参数保持为一个参数。
- 标签 `org.octos.olp.<名字>`,名字 `^[a-z0-9][a-z0-9-]{0,40}$`;plist 写到
  `--dir`(默认 `~/Library/LaunchAgents`),经临时文件替换;日志
  `~/.octos/outer/logs/<名字>.log`。
- 环境变量:默认写入 `PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`
  (launchd 的默认 PATH 不含 Homebrew),`--env K=V` 追加或覆盖。
- 加载:`install`/`heartbeat` 先 `launchctl bootout gui/<uid>/<标签>`(忽略失败)再
  `bootstrap`;`uninstall` 先 `bootout` 再删文件。`--no-load` 只写或删文件。不带 `--no-load`
  在非 macOS 上退出码 2。
- `heartbeat --unit U --hub H [--interval 60] [--scripts DIR]`:注册名为 `heartbeat` 的定时
  任务,命令是 `/bin/sh -c "python3 -B <DIR>/olp-unit.py heartbeat --unit U | ssh -o
  BatchMode=yes -o ConnectTimeout=10 H receive"`。路径与参数经 shell 引用;`H` 必须以字母或
  数字开头且只含 `[A-Za-z0-9._@-]`(防止被当成 ssh 选项或注入命令)。`olp-unit.py` 来自另一个
  未合并的工作包,本工具只在运行时调用它:文件不存在时给提醒而不失败。
- `list`:只列本工具前缀的 plist;在 macOS 上且未加 `--no-load` 时用 `launchctl print` 标出
  是否已加载。
- `doctor`:解析 `pmset -g` 的"Currently in use"一节,检查 `sleep = 0` 与
  `autorestart = 1`,逐项输出 ok / FAIL / ??(未列出)和修正命令,并提醒开机自动登录。
  只报告不修改;有问题时退出码 1。`--pmset-output <文件>` 读文件代替运行 `pmset`,供测试与
  离线检查。
- 有意不做(写入文档):不恢复内环会话;不把 `outer-duty hold` 做成常驻任务(R7:活锁接管
  归 operator);不单独保活 `octos serve`(标准用法里它是 TUI 的 stdio 子进程);不改系统设置。
- "卡了结时清理工作树"需要单元能读到卡的状态,依赖远程读板,留给后续工作包。
- 退出码:0 成功;1 `launchctl` 失败或 `doctor` 发现问题;2 用法错误,或在非 macOS 上要求
  加载任务。

## Boundaries

### Allowed Changes
- scripts/olp-launchd.py
- tests/olp_launchd.rs
- docs/OLP_KEEPALIVE.md
- specs/task-olp-unit-keepalive.spec.md

### Forbidden
- 不改任何现有脚本与协议文档。
- 测试不得触碰真实的 launchd 域(一律 `--no-load` 加临时目录)。
- 不调用 sudo,不修改电源设置。

## Out of Scope

- Linux 上的 systemd 单元。
- 了结时清理工作树。
- 负哨转发器本身(另一个工作包;做完后用 `install` 注册)。
- 装机时的权限档与模型配置。

## Completion Criteria

### Rule: jobs — 任务的生成与管理
Scenario: 定时任务与常驻任务的 plist(critical)
  Tags: critical
  Test: launchd_render_builds_interval_and_keepalive_jobs
  Given 一个带自定义环境变量、命令里含 `--flag` 和带空格参数的定时任务,以及一个常驻任务
  When 分别 render
  Then 定时任务有 StartInterval 而无 KeepAlive,命令的三个参数各自独立,环境变量含自定义值与含 Homebrew 的默认 PATH,日志路径在 `~/.octos/outer/logs/` 下;常驻任务有 KeepAlive 而无 StartInterval;render 不写任何文件

Scenario: 安装、列出、卸载
  Test: launchd_install_list_uninstall_roundtrip
  Given 一个空的任务目录(`--no-load`)
  When 安装一个定时任务,再以同名安装为常驻任务,列出,卸载,再卸载一次
  Then plist 被原地替换;列表只含本工具的任务而不含目录里别人的 plist;卸载删除文件且不动别人的文件;重复卸载不报错

Scenario: 心跳任务把巡检结果经 ssh 送到 hub(critical)
  Tags: critical
  Test: launchd_heartbeat_job_pipes_the_unit_heartbeat_to_the_hub
  Given 脚本目录的路径含空格
  When 注册心跳任务;以不存在的脚本目录再注册;以非法单元号、含命令注入的 hub、以 `-` 开头的 hub 注册
  Then 任务每 60 秒运行一条管道;`--require` 原样传给心跳命令,非法值以退出码 2 拒绝;herdr 在 PATH 上时任务记下它的真实路径(`OLP_HERDR`),不在时成功但给出提醒;脚本路径被正确引用,ssh 带 BatchMode 与超时;脚本不存在时成功但给出提醒;三种非法输入都以退出码 2 拒绝

### Rule: doctor — 电源设置检查
Scenario: 电源设置检查只报告不修改
  Test: launchd_doctor_reports_power_settings
  Given 一份设置正确的 pmset 输出(电池一节的值不同),和一份会休眠且没有 autorestart 的输出
  When 分别交给 doctor
  Then 第一份两项都 ok、退出码 0(电池一节不参与判断);第二份报告 FAIL 与 ??、给出两条 sudo pmset 命令并提醒自动登录,退出码 1

### Rule: usage — 输入检查
Scenario: 用法错误不写任何文件
  Test: launchd_rejects_bad_usage_without_writing
  Given 非法名字、既无 interval 也无 keepalive、两者都给、interval 过小、缺命令、非法 env、uninstall 带命令、未知子命令、无子命令
  When 分别运行
  Then 都以退出码 2 结束,任务目录没有被创建
