spec: task
name: "多机部署:单元保活——用 systemd 用户单元自动拉起常驻任务(Linux)"
id: task-olp-systemd-keepalive
status: implemented
tags: [olp, cluster, unit, systemd, linux]
estimate: 0.5d
---

## Intent

`scripts/olp-launchd.py` 只支持 macOS。在 Linux 集群的试点里(容器里的真实 sshd、真实内环),
单元上的心跳只能手工启动:机器重启后没有任何东西把它拉起来,hub 三分钟后把这台单元判为
失联。本任务提供 Linux 上的对应工具,子命令与含义和 launchd 版相同,让同一份操作规程在
两个平台上都成立。

## Decisions

- 工具 `scripts/olp-systemd.py`(纯 Python 标准库),子命令 `heartbeat`、`install`、
  `uninstall`、`list`、`render`、`doctor`,参数与 `olp-launchd.py` 一致;任务自己的命令写在
  `--` 之后,在交给 argparse 之前切开。
- 用 systemd **用户**单元(`~/.config/systemd/user`,`systemctl --user`),不写系统单元、
  不需要 root。文件名前缀 `olp-`,首行是本工具的生成标记;`list` 与 `uninstall` 只认带标记
  的文件。
- 任务形态两种,必须且只能选一种:
  - `--interval <秒>`(10–86400):一对文件,`olp-<名字>.service`(`Type=oneshot`)与
    `olp-<名字>.timer`(`OnBootSec=10`、`OnActiveSec=10`、`OnUnitActiveSec=<秒>`、
    `AccuracySec=1`,`WantedBy=timers.target`);启用的是 timer。
  - `--keepalive`:只有 `olp-<名字>.service`(`Type=simple`、`Restart=always`、
    `RestartSec=10`,`WantedBy=default.target`)。
- `ExecStart` 的每个参数单独加双引号;`\`、`"` 转义,`%` 与 `$` 写成两个,systemd 不会把
  它们当成说明符或变量展开。参数里不允许换行。命令的第一个词必须是绝对路径(systemd 不按
  PATH 找 `ExecStart`)。原始命令另存一行 `# olp-command: <JSON>`,供 `list` 显示。
- 环境变量:默认写入 `PATH=~/.local/bin:~/.cargo/bin:/usr/local/bin:/usr/bin:/bin`(用户
  单元的默认 PATH 很短),`--env K=V` 追加或覆盖。日志追加到
  `~/.octos/outer/logs/<名字>.log`。
- 加载:`install`/`heartbeat` 先停掉并禁用同名任务的 timer 与 service,删掉新形态不再需要
  的那个文件,写入新文件,`daemon-reload`,再 `enable --now`。**timer 与 service 各自一条
  `systemctl` 命令**:一条命令里只要有一个单元不存在,`systemctl` 整条失败,常驻任务的
  service 就不会被停掉(真实 systemd 上测出来的)。`uninstall` 同样逐个停掉再删文件。
  `--no-load` 只写或删文件。不带 `--no-load` 在非 Linux 上退出码 2。
- 调用 `systemctl --user` 时,环境里没有 `XDG_RUNTIME_DIR` 就补上 `/run/user/<uid>`:经
  `su`、`sudo -u` 或一条 ssh 命令进来的 shell 常常没有它。
- linger:用户单元默认只在该用户登录期间运行。`doctor` 读
  `loginctl show-user <用户> --property=Linger`,输出 ok / FAIL / ??,给出
  `sudo loginctl enable-linger <用户>`,并提醒关闭自动休眠;只报告不修改,有问题时退出码 1。
  `--linger-output <文件>` 读文件代替运行 `loginctl`。没有 linger 时 `daemon-reload` 会因为
  连不上用户总线而失败,报错里直接给出这条修正命令。
- `heartbeat` 的命令与 launchd 版完全相同。
- 有意不做,与 launchd 版相同:不恢复内环会话;不把 `outer-duty hold` 做成常驻任务;不
  单独保活 `octos serve`;不改系统设置。
- 退出码:0 成功;1 `systemctl` 失败或 `doctor` 发现问题;2 用法错误,或在非 Linux 上要求
  加载任务。

## Boundaries

### Allowed Changes
- scripts/olp-systemd.py
- tests/olp_systemd.rs
- docs/OLP_KEEPALIVE.md
- docs/OLP_CLUSTER.md(心跳一节提到 systemd)
- specs/task-olp-systemd-keepalive.spec.md

### Forbidden
- 不改 scripts/olp-launchd.py 与其他现有脚本。
- 测试不得触碰真实的 systemd 用户管理器(一律 `--no-load` 加临时目录)。
- 不调用 sudo,不修改系统设置。

## Out of Scope

- 没有 systemd 的 Linux(Alpine 的 OpenRC 等)。
- 把 macOS 与 Linux 两个工具合成一个入口。
- 负哨转发器本身(用 `install` 注册)。

## Completion Criteria

### Rule: jobs — 任务的生成与管理
Scenario: 定时任务是 service 加 timer,常驻任务只有 service(critical)
  Tags: critical
  Test: systemd_render_builds_interval_and_keepalive_units
  Given 一个带自定义环境变量、命令里含 `--flag`、带空格的参数、以及含 `%`、`$`、双引号、反斜杠的参数的定时任务,和一个常驻任务
  When 分别 render
  Then 定时任务输出 oneshot 的 service 与带 OnUnitActiveSec 的 timer,每个参数单独加引号且 `%`、`$` 被写成两个,环境变量含自定义值与默认 PATH,日志路径在 `~/.octos/outer/logs/` 下;常驻任务只有带 Restart=always 的 service;render 不写任何文件

Scenario: 安装、换形态、列出、卸载(critical)
  Tags: critical
  Test: systemd_install_list_uninstall_roundtrip
  Given 一个任务目录(`--no-load`),里面有一个前缀相同但不带生成标记的单元和一个无关的单元
  When 安装一个定时任务,再以同名安装为常驻任务,列出,卸载,再卸载一次
  Then 换形态后 timer 文件被删除、service 被原地替换;列表只含本工具的任务;卸载不动别人的文件;重复卸载不报错

Scenario: 心跳任务把巡检结果经 ssh 送到 hub(critical)
  Tags: critical
  Test: systemd_heartbeat_job_pipes_the_unit_heartbeat_to_the_hub
  Given 脚本目录的路径含空格
  When 注册心跳任务;以不存在的脚本目录再注册;以非法单元号、含命令注入的 hub、以 `-` 开头的 hub 注册
  Then 任务每 60 秒运行一条管道,脚本路径被正确引用,ssh 带 BatchMode 与超时;脚本不存在时成功但给出提醒;三种非法输入都以退出码 2 拒绝

### Rule: doctor — linger 检查
Scenario: linger 检查只报告不修改
  Test: systemd_doctor_reports_linger
  Given 三份 loginctl 输出:Linger=yes、Linger=no、读不出设置
  When 分别交给 doctor
  Then 第一份 ok、退出码 0;第二份 FAIL 并给出 `sudo loginctl enable-linger` 与休眠提醒,退出码 1;第三份 ??,退出码 1

### Rule: usage — 输入检查
Scenario: 用法错误不写任何文件
  Test: systemd_rejects_bad_usage_without_writing
  Given 非法名字、既无 interval 也无 keepalive、两者都给、interval 过小、缺命令、非法 env、命令不是绝对路径、参数含换行、uninstall 带命令、未知子命令、无子命令
  When 分别运行
  Then 都以退出码 2 结束,任务目录没有被创建
