spec: task
name: "/octoloop 入口对所有 agent 通用、可安装、不含固定路径"
id: task-olp-skill-portable
status: implemented
tags: [olp, skill, install]
estimate: 0.5d
---

## Intent

OctoLoop 的一键入口原先是 `.claude/skills/octoloop/SKILL.md`:它在 Claude Code 专有的目录
里,只有在 octoscode 仓库内工作时才被加载;别的项目、别的 agent 都用不上,仓库也没有提供
安装办法。个人手工装的全局版本把 octoscode 的路径写死在文件里,仓库一搬家就失效。本任务把
规程正文放到对所有 agent 通用的位置,提供安装脚本,并让入口不依赖任何固定路径。

## Decisions

- 规程正文只有一份:`skills/octoloop/AGENTS.md`(由原 SKILL.md 用 `git mv` 移来,保留
  历史)。它是纯 Markdown,保留原有的 frontmatter 与全部小节;新增一段说明它对所有 agent
  通用,以及在别的项目里相对路径如何解析。
- 受保护的内容不变:描述行、init 模式、inner 模式、自主性纪律四处的哈希原样通过,
  `tests/olp_evo_retro.rs` 与 `tests/olp_contract.rs` 只把读取的路径改到新位置。
- `.claude/skills/octoloop/SKILL.md` 改为薄卡:同样的 `name`/`description`,正文只说"完整读取
  `skills/octoloop/AGENTS.md`"。在 octoscode 仓库里 Claude Code 的 `/octoloop` 行为不变。
- 安装脚本 `scripts/olp-install-skill.sh`:
  - 默认把薄卡装到 `~/.claude/skills/octoloop/SKILL.md`;`--to <文件>` 写到别的 agent 读取
    指令的位置;`--print` 只打印不写文件;`--home-only` 只记录仓库位置。
  - 每次(除 `--print`)都把本仓库的规范化路径写进 `~/.octos/outer/octoscode-home`(一行)。
    这是唯一存放路径的地方,可以手工改。
  - 目标文件已存在且不带本脚本的生成标记时拒绝(退出码 3);`--force` 先备份为 `<文件>.bak`
    再覆盖。带标记的文件直接替换。重复安装结果不变。
- **薄卡不含固定路径。** 它让 agent 按顺序取第一个含 `skills/octoloop/AGENTS.md` 的目录作为
  `OCTOSCODE_HOME`:环境变量 `OCTOSCODE_HOME` → `~/.octos/outer/octoscode-home` 的内容 →
  当前项目根(`git rev-parse --show-toplevel`)。薄卡里给出完成这一解析的一条 shell 命令;
  失效的环境变量或记录被跳过而不是被信任;三处都找不到时命令无输出且失败,薄卡要求 agent
  停下告知用户。
- 薄卡的 `description` 在安装时取自正文并加"任意项目可用。",不另行维护。
- `olp-init.sh` 不改:它铺的是协议上岗卡(`docs/OCTOLOOP_AGENTS.md` → 项目的 AGENTS.md),与
  本入口是两回事。旧名 `/olp-outer` 的转发卡不改。

## Boundaries

### Allowed Changes
- skills/octoloop/AGENTS.md(由 .claude/skills/octoloop/SKILL.md 移来)
- .claude/skills/octoloop/SKILL.md(改为薄卡)
- scripts/olp-install-skill.sh
- tests/olp_skill_install.rs
- tests/olp_evo_retro.rs、tests/olp_contract.rs(仅读取路径)
- docs/OCTOLOOP_GUIDE.md、docs/OLP_QUICKSTART.md、docs/OLP_QUICKSTART.en.md
- specs/task-olp-skill-portable.spec.md

### Forbidden
- 不改规程正文里受哈希保护的四处内容。
- 薄卡与安装脚本不得把仓库路径写进薄卡。
- 不自动覆盖不是本脚本生成的文件。

## Out of Scope

- 为某个具体 agent 的专有机制做适配(用 `--to` 指定文件即可)。
- 把 octoscode 仓库本身分发到没有克隆它的机器。
- 历史契约文件里对旧路径的引用(它们记录的是当时允许改动的文件)。

## Completion Criteria

### Rule: layout — 一份正文,薄入口
Scenario: 规程正文只有一份,仓库内的入口是不含路径的薄卡
  Test: skill_body_is_agent_neutral_and_the_claude_entry_is_thin
  Given 仓库里的 `skills/octoloop/AGENTS.md` 与 `.claude/skills/octoloop/SKILL.md`
  When 读取两者
  Then 薄卡指向正文且不重复任何模式小节;两者的 description 相同;正文说明了路径解析规则;两者都不含固定路径

### Rule: install — 安装与路径解析
Scenario: 文档地图覆盖每一份 OctoLoop 文档,指向的文件都存在
  Test: skill_document_map_reaches_every_octoloop_document
  Given 规程正文与 `docs/` 下的全部 OctoLoop 文档(`OLP_*`、`OCTOLOOP_*`、`OUTER_LOOP_PROTOCOL.md`)
  When 读正文里的"文档地图"与 init 模式
  Then 每一份文档都在地图里出现;正文反引号里的每个 `docs/….md` 与 `scripts/olp-…` 路径都存在;多机部署的工具(两个保活工具、本地仓库工具、负哨转发、行为证据互审、运行时守护)在地图里有名字;init 模式把多机接入引到 `docs/OLP_CLUSTER.md` §7,该小节存在

Scenario: 安装的薄卡不含固定路径,仓库位置单独记录(critical)
  Tags: critical
  Test: install_writes_a_path_free_entry_and_records_the_home
  Given 一个空的 HOME
  When 运行安装脚本;再运行一次;以及 `--print`、`--home-only`、`--to <文件>`
  Then 薄卡里没有仓库路径,description 取自正文;`~/.octos/outer/octoscode-home` 是仓库的规范化路径;重复安装结果不变;`--print` 不写文件;`--home-only` 不装薄卡;`--to` 写出同样的薄卡

Scenario: 薄卡里的解析命令按 环境变量 → 记录文件 → 当前项目 的顺序找到仓库(critical)
  Tags: critical
  Test: installed_entry_resolves_the_home_in_order
  Given 已安装的薄卡里的解析命令
  When 在无关目录里执行;设置指向另一份含正文的目录的环境变量;设置失效的环境变量;把记录文件改成失效路径后在仓库里执行;在无关目录里且记录失效时执行
  Then 依次得到:记录的仓库、环境变量指向的目录、记录的仓库(失效变量被跳过)、当前项目(失效记录被跳过)、无输出且失败

Scenario: 不覆盖别人写的文件,除非明确要求并留备份
  Test: install_refuses_to_overwrite_a_foreign_file_without_force
  Given 目标位置已有一份不是本脚本生成、且写死了旧路径的文件
  When 运行安装脚本;加 `--force` 再运行;再普通运行一次;以及未知参数与缺值的 `--to`
  Then 第一次以退出码 3 拒绝且文件不变;`--force` 把原文件备份为 `.bak` 后替换;本脚本生成的文件被直接替换且不再产生备份;用法错误为退出码 2
