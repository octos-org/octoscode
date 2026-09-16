---
kind: context
id: EVO-OPERATORS
title: "进化环算子表与固定禁改段"
tags: [olp, evolution, harness]
---

# 进化环算子表与固定禁改段

来源:LEP-003 §Decision 第 4 项;借鉴 HarnessFix(arXiv 2606.06324)表二,按本项目裁剪。
每份修复规格只能从下表选算子;固定禁改段随每份规格附带,agent 不得删改。

## 算子表(按 ETCLOVG 层)

| 层 | 算子 | 对应实案与剩余缺口 | 仓库 |
|---|---|---|---|
| Lifecycle | 验证门控收工 | 预算耗尽已 wip commit 加 checkpoint,剩余缺口是审查黑板无 ACK | octoscode 规程 + octos |
| Lifecycle | 重试与超时上界 | writer 停摆;恒定时长即固定成本或超时 | octoscode |
| Lifecycle | 状态检查点与写序 | 离线 goal archive 被 live cache 反盖(#43 在线化后观察复发) | octos |
| Lifecycle | 委派输出校验 | peer 交付双写竞争,result.md 单写者已落地 | octos |
| Lifecycle | 环境快照复用 | 围栏 peer 克隆无构建缓存,Rust peer 冷编译耗尽迭代(octos #2236) | octos |
| Lifecycle | 准入判定修正 | goal_create 把 archived 当未完成拒绝(octos #2237) | octos |
| Observability | 新增 producer | malformed 耗尽、fallback 切道无事件;kind 集合归 REQ-OLP-OBS 修订 | octos |
| Observability | 错误与状态差日志 | serve stderr 进 ring buffer 探针不可见;裸线程 tracing 丢失 | octoscode |
| Tooling | 报错信息修复 | 权限档 1 到 4 下 cargo "command not found" 无沙箱提示;档位不动只改文案 | octos |
| Tooling | 参数校验 | malformed 自纠上限 | octos |
| Context | idle 读板唤醒 | loop paused 或哨兵失效时 idle master 不消费新板项;不把 steer 升为用户消息层级 | octos |
| Context | 上下文预算检查 | 沉淀章节越长越吃上下文 | octoscode 文档 |
| Verification | 效果证据收工守卫 | verified 声称被复验证伪 | octoscode 规程 |
| Verification | 期望与实际状态对比 | 自检翻数据目录而 TUI 状态栏明示 paused;自检绑定权威面 | octoscode 规程 |
| Verification | 回归夹具 | 单测全绿但真管道字节流损坏;夹具必须用真 OS 原语 | 两仓库 |
| Governance | 只加门 | 只能加更严的围栏;树主权谓词与审批分级属协议修订,不走进化派单 | 不派单 |

## 固定禁改段(每份修复规格自带)

- R1 到 R7 的语义与 `protocol: olp/vN` 版本号(R6 只由人升版)。
- 推送权只在外环。
- 免沙箱与 `--danger-full-access`。
- harness 自身权限配置。
- `tests/olp_contract.rs` 白名单与黑板存量冻结。
- MCP 工具面与运行态路径(`~/.octos/outer/mcp/`,规格已禁第三工具)。
- 树主权谓词(R4b)。
- 一切 operator-tier 动作。

## 规格三段(accepted 后写进缺陷记录)

```
## 修复规格
target:     FLAW-NNN · issue #NNNN
repo:       octoscode | octos
operators:  primary = <算子>; aux = <算子>
allowed:    <路径列表>
forbidden:  (固定禁改段) + 本次追加
required:   <可 grep 的行为断言>
worktree:   ~/.octos/outer/evo/<repo>/FLAW-NNN/   # 围栏,主树禁止 checkout
acceptance: <回放夹具> + <观察窗与期限> + 四门 + 主审复验 + 异议处置
```
