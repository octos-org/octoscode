# 证据索引与复验边界

本轮于 2026-09-09 在四个固定 PR HEAD 上做独立静态审查，再在独立合成树运行全量测试与负向反例。准确 SHA、各 PR 比较基线、合成树文件 SHA256、运行模型和原生 goal 状态见上级 [MANIFEST.json](../MANIFEST.json)。#628 自身增量相对 #627，不能把重复包含的代码算作另一项独立变更。

反例源码是外层 Codex 编写并执行的诊断测试；GLM/k3 在初审冻结后核实其生产调用链。两车道未运行 cargo，报告中的动态证据不能算作第三次独立实验。最终综合以外层复核后的 [ASSESSMENT.md](../ASSESSMENT.md) 为准；保留的初审和改判前文件包含已被后续证据推翻的意见。

| PR | 实际执行的测试 | 证明的缺口 |
|---|---|---|
| 627 | `outer_review_627_failed_archive_must_not_replay_on_new_goal` | 失败的旧 Archive 意图意外归档新 goal |
| 627 | `outer_review_627_cancelled_archive_must_not_replay_after_reconnect` | 取消、重连后旧意图被普通查询重放 |
| 628 | `outer_review_628_idle_rejection_must_render_beyond_status_bar` | 有聊天历史的 idle 场景看不到拒绝原因 |
| 628 | `outer_review_628_active_turn_rejection_must_render` | active turn 的过滤排除新 Warning |
| 628 | `outer_review_628_repeat_rejection_must_keep_specific_reason` | 第二次相同拒绝的具体原因退化 |
| 629 | `outer_review_629_runtime_main_list_must_not_enable_invalid_save` | list 回包 seed `_main` 后仍派发无效持久化请求（遗留路径） |
| 630 | `outer_review_630_no_first_frame_must_retry_after_grace` | grace 过期、无首帧仍不重试 |
| 630 | `outer_review_630_queued_success_must_survive_retry_deadline` | 重试删除旧请求关联，丢弃已到达成功响应 |

[adversarial.log](adversarial.log) 记录 **0 passed / 8 failed**，即八条诊断断言均按预期暴露缺口；没有实施修复，不能把它们写成回归测试通过。[store-repros.rs](store-repros.rs) 与 [transport-repros.rs](transport-repros.rs) 为完整诊断源码。[reproduce.sh](reproduce.sh) 在新的 disposable clone 重建合成树并挂入反例；脚本通过 `bash -n`，脚本本身未再次执行，实际运行证据来自同等步骤的上述日志。

全量验证保留三类结果：[初轮](all-targets.log)、[GNU 工具兼容复跑](platform-retest.log)、[GNU 环境完整轮](all-targets-gnu.log)。最后完整轮为 2383 passed / 1 failed / 4 ignored，唯一计时失败的 [隔离复跑](timing-retest.log) 通过。Clippy 的既有布尔表达式错误见 [clippy.log](clippy.log)，未修改四 PR 之外代码来隐藏该问题。工具链版本见 [toolchain.json](toolchain.json)，最新 PR 状态见 [pr-state.json](pr-state.json)。

原生报告真实性由各 `pr-N/native/<slug>/result-N.md`、`turns.txt` 和 `runtime-evidence.json` 交叉确认。最高编号 result 的 outcome 对应已结束轮次，运行中的下一轮以 thread 状态为准；四个失败的原 GLM review peer 仍保留 `errored`，不能计入有效互审。
