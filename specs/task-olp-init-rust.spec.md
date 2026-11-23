spec: task
name: "OctoLoop init 与 skill 安装的 Rust 实现"
id: task-olp-init-rust
status: implemented
tags: [olp, init, skill, rust]
estimate: 0.5d
---

## Intent

`olp init` 与 `olp skill install` 原先是 `scripts/olp-init.sh`、`scripts/olp-install-skill.sh`。
按"脚本迁移到 Rust"的顺序迁过来,铺下去的文件与装上的薄卡不变。

## Decisions

- `octoscode olp init [目录]` 由 `src/olp/init.rs` 实现:依赖体检(git、octoscode 缺失时退出码 2)、
  `.octos/loop.md`、`.octos/OUTER_LOOP_REVIEW.md`、把三个工作文件加入 `.gitignore`(已被忽略的
  不重复加;`.gitignore` 末行没有换行时先补换行)、`AGENTS.md` 上岗卡(`OLP_INIT_LANG=zh` 取
  中文版)、`~/.octos/outer/` 下的 `watch-board.sh` 与 `board-append.sh`。已存在的一律不动。
- 上岗卡与两个单文件助手编进二进制(`include_str!`),不再从工具目录复制,也不再用 curl 兜底。
- `octoscode olp skill install [--to F | --print | --home-only] [--force]`:薄卡的文字与脚本生成的
  逐字节相同;记下的位置是本版本工具目录旁的 `current` 链接(升级后仍有效)。目标已存在且不是
  本工具生成的:退出码 3,加 `--force` 先备份为 `<文件>.bak` 再覆盖。
- `OCTOSCODE_OLP_LEGACY=1` 改跑原脚本。
- 两个脚本保留:从仓库检出直接运行 `scripts/olp-install-skill.sh` 时记下的是检出目录(正文随
  `git pull` 更新),这是二进制做不到的另一种用法;它们原有的测试照旧对脚本运行。

## Boundaries

### Allowed Changes
- src/olp/init.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_init_native.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md

### Forbidden
- 不写任何 API key;不给 serve 加免沙箱参数;不覆盖已有的 loop.md、黑板、AGENTS.md。

## Out of Scope

- 退役两个脚本,以及"从检出运行时记下检出目录"这种用法的去留:留给 operator 决定。

## Completion Criteria

Scenario: Rust 版 init 铺设脚手架,重跑不改动任何已有内容(critical)
  Tags: critical
  Test: native_init_lays_the_scaffold_once
  Given 一个 git 项目,`.gitignore` 的末行没有换行
  When 以 OLP_INIT_LANG=zh 执行 octoscode olp init <目录>;用户改掉 AGENTS.md 与 loop.md 后再执行一次;对不存在的目录执行
  Then 生成 loop.md、以 `ACK:` 结尾的黑板、中文上岗卡(与仓库里的逐字相同);`.gitignore` 原有一行完好,三个工作文件各占一行且只出现一次;两个助手与仓库脚本逐字节相同且可执行;重跑后用户的文件与 `.gitignore` 都没变;不存在的目录退出码 2

Scenario: Rust 版 skill install 的薄卡与脚本生成的一字不差
  Test: native_skill_card_is_the_scripts_card
  Given 一个空的 HOME
  When 比较 octoscode olp skill install --print 与 scripts/olp-install-skill.sh --print;再 --to 装到文件;把该文件换成别人的内容后再装,再加 --force
  Then 两份输出逐字节相同且 --print 不写任何文件;装到的文件就是这张卡,记下的位置以 /current 结尾且其中有规程正文;别人的文件不被覆盖(退出码 3);--force 先备份为 .bak 再覆盖
