spec: task
name: "外环观测面:只读状态 CLI、事件流、inbox 寻址(octos)"
tags: [olp, observability, octos, upstream]
satisfies: [REQ-OLP-OBS]
estimate: 2d
---

## 意图

外环模型今天靠"翻 per-instance 目录 + 自算跨版本不稳定的 session hash +
grep 人类日志"观测内环,三件套全部脆弱(实测:日志按进程启动日期滚动、
DefaultHasher 上游自认不稳定、ugrep 缓冲吞事件)。本任务给 octos 增加
机器可读的只读观测面:三个 `--json` 子命令、结构化事件流、inbox 路径
查询。实施仓库为 **octos**(本合约随 workstream 提交上游)。

## 已定决策

- 新增只读子命令 `octos goal status [--goal <id>] --json`、
  `octos peer list --json`、`octos ledger tail <goal_id> --json`:直读
  数据目录(redb/文件),serve 进程不存活时同样可用;`--json` 缺省时输出
  人类可读表格,两种模式共享同一数据装配层。
- serve 追加写 `<data_dir>/events.jsonl`,每行
  `{ts, kind, goal_id?, slug?, session?, model_lane?, detail}`;kind 至少
  覆盖 `peer_staged`、`finding_recorded`、`escalation`、`goal_transition`、
  `steer_consumed`、`turn_error`;只追加、单行 JSON、写失败不影响主流程
  (best-effort,与 goal ledger 的 durable 定位区分)。
- `peer_staged` 事件携带解析后的 `model_lane`(未指定 lane 时为
  `"primary"`)。
- 新增 `octos inbox path --session <key>`:输出该 session 的 notes 文件
  绝对路径;哈希算法保持内部实现细节(operator 拍板:零迁移),外部
  消费者一律经此命令寻址。

## 边界

### Allowed Changes(octos 仓库)
- crates/octos-cli/src/commands/**
- crates/octos-cli/src/autonomy/**
- crates/octos-cli/src/api/ui_protocol_transport.rs
- crates/octos-cli/tests/**
- crates/octos-cli/src/lib.rs

### Forbidden
- 不改任何既有 AppUI 协议方法的语义与 wire 形状。
- 不改 serve 排他锁、goal ledger、inbox notes 的既有读写协议。
- events.jsonl 只追加;不得为它引入轮转/删除逻辑(留给后续提案)。
- 不新增网络端点。

## 排除范围

- steer 子命令(task-req-olp-ctrl-steer)。
- 人类日志(tracing)的滚动策略修复——事件流落地后外环不再依赖它。
- events.jsonl 的消费端工具。

## 完成条件

场景: serve 停止时仍可读 goal 状态(critical)
  测试:
    包: octos-cli
    过滤: olp_obs_goal_status_json_without_serve
  假设 一个含已完成 goal 账本的数据目录且 serve 未运行
  当 执行 octos goal status --goal <id> --json
  那么 输出合法 JSON 且 status 字段为 "complete"

场景: 未知 goal id 报结构化错误
  测试:
    包: octos-cli
    过滤: olp_obs_goal_status_unknown_id_errors
  假设 数据目录中不存在 goal_nonexistent
  当 执行 octos goal status --goal goal_nonexistent --json
  那么 进程以非零退出且 stderr 输出含 error 字段的 JSON

场景: peer 交付追加结构化事件且带 model_lane
  测试:
    包: octos-cli
    过滤: olp_obs_finding_appends_event_with_lane
  假设 一个 goal-scoped peer(未指定 lane)完成一个 turn
  当 finding 写入账本
  那么 events.jsonl 追加一行 kind=finding_recorded 且 model_lane 为 "primary"

场景: 事件写失败不影响主流程
  测试:
    包: octos-cli
    过滤: olp_obs_event_write_failure_is_nonfatal
  假设 events.jsonl 所在目录不可写
  当 一个 goal-scoped peer 完成 turn
  那么 finding 仍写入账本且 turn 正常结束

场景: 外部进程查询 inbox 路径
  测试:
    包: octos-cli
    过滤: olp_obs_inbox_path_matches_serve
  假设 会话 key octos:local:tui#coding
  当 执行 octos inbox path --session octos:local:tui#coding
  那么 输出路径与 serve 写入 wake note 的实际路径一致

场景: ledger tail 对空账本输出空数组
  测试:
    包: octos-cli
    过滤: olp_obs_ledger_tail_empty_goal
  假设 goal 存在但账本无 finding
  当 执行 octos ledger tail <goal_id> --json
  那么 输出为合法 JSON 空数组且退出码为 0
