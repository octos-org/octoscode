spec: task
name: "OctoLoop 板面侦听哨的 Rust 实现"
id: task-olp-board-sentinel-rust
status: implemented
tags: [olp, board, sentinel, rust]
estimate: 0.5d
---

## Intent

`olp board watch` 原先是 `scripts/olp-watch-board.sh`。按"脚本迁移到 Rust"的顺序把它迁过来,
规则不变:只看挂哨后新增的行,域内做子串匹配。

## Decisions

- `octoscode olp board watch <板> <token> [--interval 秒] [--skip-signature 署名]… [--harvest <repo>]`
  由 `src/olp/sentinel.rs` 实现;`OCTOSCODE_OLP_LEGACY=1` 改跑原脚本。
- 基线是挂哨时板的行数;之后新增的行里,不含任何排除署名、且含 token 的行算命中。板变短时
  重取基线,不回看旧内容。板不存在时等它出现。
- 不带 `--harvest`:命中即打印 `BOARD-SIGNAL: <token>` 与最多 3 行命中,退出码 0。
- 带 `--harvest <repo>`:命中时先跑本程序自带的采集(`olp evo harvest <repo>`,2026-11-23 起
  也是 Rust;不再需要板旁边有一份仓库检出),失败时在 stderr 报 `harvest: failed (exit N)`,然后报信号、
  推进基线、继续监视。
- 脚本保留:`olp init` 仍把它单文件装成 `~/.octos/outer/watch-board.sh`,它原有的测试照旧对
  脚本运行。

## Boundaries

### Allowed Changes
- src/olp/sentinel.rs、src/olp/mod.rs、src/cmd/olp.rs
- tests/olp_watch_board.rs
- docs/OLP_CLI.md、docs/ARCHITECTURE.md

### Forbidden
- 不改 `scripts/olp-watch-board.sh`。

## Out of Scope

- `--harvest` 常驻模式的另外四个场景(与手跑并发去重、采集失败后继续、失败后恢复、从安装
  副本运行)目前只对脚本运行。

## Completion Criteria

Scenario: Rust 版侦听哨只对挂哨后新增的行触发,并排除带署名的行
  Test: native_watch_fires_only_on_new_lines_and_skips_signed_ones
  Given 一块已含 token 字样的板,用 octoscode olp board watch 挂哨并排除外环的署名
  When 等一会儿;追加一行带外环署名且含 token 的批注;再追加一行不带署名、含 token 的行;另以缺参数、非数字间隔调用
  Then 旧内容不触发;带署名的批注不触发;新行触发,输出 BOARD-SIGNAL 与该行,退出码 0;参数错误退出码 2

Scenario: Rust 版侦听哨的 --harvest 命中时落卡、推进基线并继续监视
  Test: native_watch_harvest_cards_each_hit_and_keeps_watching
  Given 用 octoscode olp board watch 带 --harvest 挂哨
  When 追加一行命中;追加一行不含 token 的行;再追加一行不同的命中
  Then 第一次命中落一张卡且哨兵仍在运行;无关行不落卡;第二次命中再落一张卡
