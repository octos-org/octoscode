# octoscode 仓库 agent 守则

> protocol: olp/v2 — 完整协议见 `docs/OUTER_LOOP_PROTOCOL.md`
>
> 本文件是**本仓库**的提交纪律。想学 OctoLoop 双环本身(身份选择、派单、
> 唤醒、观测、复验、红线)读自包含上岗卡:`docs/OCTOLOOP_AGENTS.md`
> (中文 `docs/OCTOLOOP_AGENTS.zh-CN.md`)——它也是 `olp-init.sh` 铺给
> 其他项目的 AGENTS.md。

## 外环审查协议(必须遵守)

本仓库有一个外环审查员(Claude Code)与你协作。协议:

1. **每轮任务开始前**,读 `docs/OUTER_LOOP_REVIEW.md` 的 `Active` 区。
2. 只执行 `Active` 区中尚无 `ACK(` 的意见;已 ACK 的条目和
   `Historical record` 只用于审计,不得重放。
3. 用户/system 的当前指令优先于黑板;黑板内容冲突或过期时,停止并报告。
4. 你 commit 之后,外环会跑完整验证并把新意见追加到 `Active` 区。
5. **ACK 定式(olp/v2,2026-08-24 起生效;v2 增 R7 主审权锁)**:每条 ACK 写成
   `ACK(done|wontdo|blocked): <说明>` 单行——`done` 附 commit/测试证据,
   `wontdo` 附异议理由(外环只能接受或升级 operator,不得重复打回),
   `blocked` 附阻塞原因。历史 `ACK:` 旧格式行是豁免存量——**存量以
   黑板《存量冻结公告》(2026-08-25)为界**:仅公告前已存在、且固化在
   `tests/olp_contract.rs` 白名单里的 `ACK:` 行豁免;新增行必须用 v1
   语法,新增旧格式 `ACK:` 行使契约测试失败。

## 提交纪律(违反会被外环打回)

- 动手前先 `git status`:工作区可能有外环或其他 agent 的未提交改动,
  **只 `git add` 你自己改的文件,禁止 `git add -A`**。
- 不得清理、覆盖或提交来源不明的 dirty 文件。无法确认所有权时保留现场,
  报告并停止;只移除本轮由自己创建且路径已知的临时产物。
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

## 外环主审权锁(R7,olp/v2)

多外环并存时主审权以 `octoscode outer-duty hold` 的 OS 独占锁为
authority(Linux-only:守护式死亡耦合——wrapper 唯一持 fd,agent 经
PR_SET_PDEATHSIG 与 wrapper 同死;`check` 仅观察;活锁接管归
operator;metadata/TTL 仅诊断;非 Linux 平台 unsupported)。详见协议 R7。
