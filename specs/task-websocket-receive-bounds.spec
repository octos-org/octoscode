spec: task
name: WebSocket 接收内存边界
inherits: project
tags: [transport, websocket, resource-bounds]
---

## 意图

在 WebSocket 组装消息和排队阶段执行 OUP 大小限制，避免 UI 慢消费者导致
仅按条数限制的接收队列积累大量正文。stdio 行缓冲策略保持独立。

## 已定决策

- frame 和重组后的 message 均限制到 MAX_TEXT_FRAME_BYTES。
- 接收队列正文预算为八个最大协议消息，预算随队列元素持有至消费或丢弃。
- 大小超限报告 frame_too_large 并断开，通过现有取消流程释放在途请求。
- 协议限额内消息和控制帧保持原有传递语义；JSON 关联分析前先检查文本长度。

## 完成条件

场景: 超大正文在入队前被拒绝
  测试: websocket_receive_rejects_oversized_text_before_queueing
  当 真实本地 WebSocket 对端发送超过协议限制的文本消息
  那么 客户端不向事件队列投递该正文并报告断开错误

场景: 分片重组同样遵守消息限额
  测试: websocket_receive_bounds_fragmented_binary_messages
  当 两个合法大小的分片合计超过消息限制
  那么 客户端拒绝重组消息且不向事件队列投递正文

场景: 协议限额本身可接收
  测试: websocket_receive_allows_message_at_protocol_limit
  当 对端发送恰好等于协议上限的文本
  那么 该文本原样进入事件队列

场景: 慢消费者占用的字节预算不会提前释放
  测试: websocket_receive_queue_releases_byte_budget_only_when_consumed
  当 队列字节预算耗尽但条目容量尚有余量
  那么 下一条正文等待并在已有元素释放后继续
