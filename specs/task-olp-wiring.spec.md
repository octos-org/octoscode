spec: task
name: "写进系统里的 OctoLoop 命令调用 octoscode olp,并由 olp doctor 检查"
id: task-olp-wiring
status: implemented
tags: [olp, cli, upgrade, doctor]
estimate: 1d
---

## Intent

`octoscode olp` 从按内容命名的工具目录(`~/.octos/outer/olp-bundle/<哈希>/`)运行脚本。
脚本写下的、以后才执行的命令——git 钩子、单元工作仓库的 receivepack/uploadpack、保活任务、
skill 记下的规程位置——都用了自己的绝对路径,于是落在某一版本的工具目录里:升级 octoscode
后仍跑旧代码,旧目录被清掉后直接失败(Mac server 预检,2026-11-14)。本任务让这些地方写
`<octoscode 绝对路径> olp …`,并提供 `octoscode olp doctor` 找出旧写法、给出修复命令。

## Decisions

- `octoscode olp` 运行脚本时设置 `OCTOSCODE_BIN`(正在运行的二进制)。新库
  `scripts/olp-cli-lib.py`(无命令)据此给出要写下的命令:有 `OCTOSCODE_BIN` 时写
  `<octoscode> olp <组> <动作> …`;PATH 上的 `octoscode` 与它是同一个文件时写 PATH 上的名字;
  没有时照旧写本脚本的路径(直接从仓库运行,随 `git pull` 更新)。
- 写法改动的地方:`olp repo init` 的两个钩子(`olp repo hook <名字>`)、`olp repo connect-local`
  (`olp repo local-pack …`)、`olp keepalive heartbeat`(`olp unit heartbeat … | ssh … receive`
  或单机的 `| … olp unit receive …`)。`--scripts <目录>` 仍明确使用该目录的脚本。
- post-receive 钩子从自己所在的目录找写板助手,不再读仓库配置 `olp.scripts`;`repo init`
  删除这个旧配置。
- `olp-bundle/current` 链接指向最近运行的版本的目录(每次运行时更新,先建新链接再改名覆盖)。
  `olp skill install` 在工具目录里运行时把这个链接记进 `~/.octos/outer/octoscode-home`。
- `octoscode olp doctor [--repo R]… [--work W]… [--remote hub] [--json]` 检查:
  `~/.ssh/authorized_keys` 里调用 OctoLoop 的强制命令;hub 裸仓库的钩子(`--repo` 与强制
  命令里 serve 的 `--repo`);工作仓库 hub 远程的本机推送设置(`--work` 与当前目录);
  launchd 与 systemd 的保活任务;`octoscode-home`。判定:调用 octoscode 绝对路径或仓库脚本
  为 ok;位于某一版本工具目录(`current` 除外)或按 PATH 找 octoscode 为 warn;写下的路径
  不存在、钩子缺失或不可执行、命令无法解析为 fail。warn 与 fail 附修复命令(强制命令给出
  改写后的整行 `command="…"`)。有 fail 时退出码 1。
- ssh 强制命令仍由人写进 `authorized_keys`;文档示例改为
  `command="/path/bin/octoscode olp … "`,并说明写绝对路径的原因(sshd 的 PATH 很短)。

## Boundaries

### Allowed Changes
- src/cmd/olp.rs
- scripts/olp-cli-lib.py、scripts/olp-doctor.py
- scripts/olp-git-hub.py、scripts/olp-launchd.py、scripts/olp-systemd.py、scripts/olp-install-skill.sh
- scripts/olp-answer.py、scripts/olp-board-remote.py(仅帮助文字)
- tests/olp_cli.rs
- docs/OLP_CLI.md、docs/OLP_CLUSTER.md、docs/OLP_LOCAL_GIT.md、docs/OLP_REMOTE_BOARD.md
- skills/octoloop/AGENTS.md(文档地图与命令对照各一行)
- specs/task-olp-wiring.spec.md

### Forbidden
- 不改直接运行脚本时写下的命令(现有契约测试照旧通过)。
- 不改推送规则、心跳内容、强制命令的校验逻辑。
- 不自动改写 `authorized_keys`(只给出改写后的行)。

## Out of Scope

- 清理旧版本的工具目录。
- 把脚本换成 Rust。

## Completion Criteria

Scenario: 经 octoscode olp 写下的钩子、本机推送设置、心跳任务调用 octoscode,推送照常经钩子(critical)
  Tags: critical
  Test: olp_wiring_calls_octoscode_not_the_bundle
  Given 一个沙箱 HOME 与独立的工具目录根
  When 经 `octoscode olp` 执行 `repo init`、`repo connect-local`、`keepalive heartbeat --hub local`,再推送一个带复验记录的任务分支
  Then 钩子是 `exec <octoscode> olp repo hook <名字>`,receivepack 以 `<octoscode> olp repo local-pack --unit m01` 开头,心跳任务调用 `<octoscode> olp unit heartbeat` 与 `unit receive`,都不含工具目录;推送被接受(有 flock 时黑板登记了这次推送)

Scenario: 直接运行脚本时仍写脚本路径
  Test: launchd_heartbeat_job_pipes_the_unit_heartbeat_to_the_hub
  Given 没有 `OCTOSCODE_BIN`
  When 直接运行 `scripts/olp-launchd.py heartbeat`
  Then 任务里是 `python3 -B '<仓库>/scripts/olp-unit.py' heartbeat …`

Scenario: skill install 记录 current 链接,它指向本版本的工具目录
  Test: olp_skill_install_records_the_current_link
  Given 一个沙箱 HOME
  When 执行 `octoscode olp skill install --home-only`
  Then `octoscode-home` 记的是工具目录根下的 `current`,它解析到本二进制的工具目录,其中有规程正文

Scenario: doctor 把固定在某一版本工具目录的命令报 warn,路径消失后报 fail 并给出修复命令(critical)
  Tags: critical
  Test: olp_doctor_flags_pinned_then_missing_wiring
  Given hub 裸仓库的两个钩子与一条强制命令指向另一版本工具目录里的脚本
  When 先运行 `octoscode olp doctor --repo <裸仓库> --json`,再删掉那个工具目录后运行,再按建议修复后运行
  Then 第一次 3 项 warn、退出码 0;第二次 3 项 fail、退出码 1,修复命令含 `octoscode olp repo init` 与改写后的 `command="<octoscode> olp unit receive --unit m02"`;修复后 3 项 ok、退出码 0
