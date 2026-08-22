# octoscode 仓库 agent 守则

> protocol: olp/v0 — 完整协议见 `docs/OUTER_LOOP_PROTOCOL.md`

## 外环审查协议(必须遵守)

本仓库有一个外环审查员(Claude Code)与你协作。协议:

1. **每轮任务开始前**,读 `docs/OUTER_LOOP_REVIEW.md`(外环审查黑板)。
2. 执行其中带日期的最新审查意见;每条意见下方的 `ACK:` 行由你补写:
   做了什么,或为什么不做。
3. 你 commit 之后,外环会跑完整验证并把新意见追加到同一文件。

## 提交纪律(违反会被外环打回)

- 动手前先 `git status`:工作区可能有外环或其他 agent 的未提交改动,
  **只 `git add` 你自己改的文件,禁止 `git add -A`**。
- 验证必须跑 `cargo test --all-targets`——只跑 `--lib` 看不到 `tests/`
  目录的编译破损(本仓库已两次因此损坏 CI)。
- commit 前:`cargo fmt` + `cargo clippy --all-targets -- -D warnings`。
- 改了公开签名(如构造函数加参数),同一个 commit 里更新所有调用点,
  包括 `tests/*.rs` 集成测试。
- 新代码不要放在 `mod tests` 之后(clippy items-after-test-module)。
- 不在仓库根目录留验证/草稿文件(`FINAL_VERIFICATION.md` 之类);
  结论写进 commit message 或黑板的 ACK。

## 行为契约

- 行为改动同步更新 `specs/` 下对应的 `.spec` 合约(场景绑定真实测试名)。
- 无法运行工具链时(例如 shell 没有 cargo),在交付物里如实声明
  "未验证",不要声称已验证。
