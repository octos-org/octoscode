spec: task
name: "控制面:带回执的 steer、审查者通道、升级通知(octos)"
tags: [olp, steer, escalation, octos, upstream]
depends: [task-req-olp-obs-cli]
satisfies: [REQ-OLP-CTRL]
estimate: 2d
---

## 意图

外环对 idle 内环的唯一唤起方式是"inbox 门铃 + operator 说一句话"
(实测:goal complete 后 master 完全 idle,黑板新条目无人读)。本任务
给 octos 增加可编程的 steer 通道(CLI 落盘 + 唤醒 + 机器可读回执)与
escalation 外部通知,消除无人值守长程的最后一个人工环节。实施仓库为
**octos**。operator 已拍板:CLI 落盘唤醒,不新增网络端点。

## 已定决策

- 新增 `octos steer --session <key> --text <指令>`:写入持久队列。**注入层级必须等同用户消息**(实测两次:system prompt 附加段的指令会被模型当作背景信息而不执行——外环 2026-08-22 门铃实验)
  (inbox `.reviewer-notes` sidecar,与 `.monitor-notes` 同构的
  flock+append 协议),随后复用 `enqueue_goal_progress_wake` 的唤醒机制
  触发目标 session 的 continuation。
- steer 注入下一 turn 的 prompt 时带 `### External reviewer` 头与
  `[external-reviewer]` 来源标记;信任级别与 peer brief 相同(数据,
  非系统指令);单 turn 注入总量沿用 notes 的 64KiB 读取上限,超限的
  单条 steer 在入队时即拒绝。
- steer 被消费后向 events.jsonl 写 `steer_consumed`
  (含入队时间戳与消费 turn id)——机器可读回执(依赖
  task-req-olp-obs-cli 的事件流)。
- goal-scoped escalation 记录时,若 profile 配置了通知通道(复用 cron
  notify mode 的发送器),向 operator 发送含 slug 与 goal_id 的通知;
  未配置时静默跳过(不失败、不告警刷屏)。

## 边界

### Allowed Changes(octos 仓库)
- crates/octos-cli/src/commands/**
- crates/octos-cli/src/autonomy/**
- crates/octos-cli/src/api/ui_protocol_transport.rs
- crates/octos-cli/src/lib.rs
- crates/octos-cli/tests/**

### Forbidden
- steer 不得注入为 system 指令层级(只能是 prompt 数据段)。
- steer 不得触达审批决定(peer_respond/approval 不在本通道能力内)。
- 不新增网络端点(operator 拍板)。
- 不改 goal-progress notes 与 monitor-notes 的既有语义。

## 排除范围

- 远程(跨主机)steer——依赖网络端点,另行提案。
- steer 的权限模型(同 UID 即可,v1 不做多用户)。
- 外环消费 steer_consumed 的工具。

## 完成条件

场景: steer 唤醒 idle master 并留下回执(critical)
  测试:
    包: octos-cli
    过滤: olp_ctrl_steer_wakes_and_receipts
  假设 master 无 active goal 且处于 idle
  当 执行 octos steer --session <master> --text "读黑板第 7 条"
  那么 下一 turn 的 prompt 含该指令与 External reviewer 头,且
       events.jsonl 出现 steer_consumed

场景: steer 目标 session 不存在时报错
  测试:
    包: octos-cli
    过滤: olp_ctrl_steer_unknown_session_errors
  假设 给定的 session key 无任何持久状态
  当 执行 octos steer --session unknown --text hi
  那么 进程非零退出且不产生队列文件

场景: 超限 steer 在入队时被拒绝
  测试:
    包: octos-cli
    过滤: olp_ctrl_steer_oversize_rejected
  假设 一条超过 64KiB 的 steer 文本
  当 执行 octos steer
  那么 进程非零退出且队列未追加任何内容

场景: steer 不越权改配置
  测试:
    包: octos-cli
    过滤: olp_ctrl_steer_cannot_mutate_config
  假设 一条要求修改 verify_command 的 steer 被消费
  当 检查 profile 配置文件
  那么 配置保持不变(steer 是数据不是配置写入通道)

场景: steer 与审批隔离
  测试:
    包: octos-cli
    过滤: olp_ctrl_steer_never_answers_approvals
  假设 目标 session 有一个 park 中的 approval
  当 一条 steer 被消费
  那么 approval 仍处于 park 状态(steer 不构成审批答复)

场景: escalation 触发外部通知
  测试:
    包: octos-cli
    过滤: olp_ctrl_escalation_notifies_operator
  假设 profile 配置了 notify 通道且一个 goal-scoped peer park 在 approval
  当 escalation 写入账本
  那么 通知发送器收到一条含 slug 与 goal_id 的消息

场景: 未配置通知通道时静默跳过
  测试:
    包: octos-cli
    过滤: olp_ctrl_escalation_no_channel_noop
  假设 profile 未配置 notify 通道
  当 escalation 写入账本
  那么 记账成功且不产生错误或告警
