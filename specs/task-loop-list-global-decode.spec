spec: task
name: "全局 loop 查询的回包解码与分发"
inherits: project
tags: [tui, autonomy, loop, protocol]
estimate: 0.25d
---

## 意图

`/loop list` 的请求侧已改为全局查询(无会话时省略 `session_id`),但回包方向
未跟上,链路仍然断在客户端:

1. 服务端把请求的 `session_id` **原样回显**,全局查询时回包为
   `"session_id": null`;而 `LoopListResult.session_id` 是非 Option 的
   `SessionKey`,实测解码报 `invalid type: null, expected a string` ——
   列表永远为空(与 2026-08-03 的 `session/hydrate missing field payload`
   同类:请求方向修好、响应方向没跟上)。
2. 消费端 `set_session_loops(&result.session_id, loops)` 把**全部**返回的
   loop 写进单一会话镜像;全局查询返回的 loop 可能分属多个会话,一股脑塞进
   一个桶会错配。
3. 无活跃会话时 `active_session_profile_id()` 必然返回 `None`,服务端据此
   兜底到 `main` profile,而用户的 loop 在其他 profile 下会被过滤清零。

本任务补齐这三处,让全局查询端到端可用。

## 已定决策

- `LoopListResult.session_id` 改为 `#[serde(default)] Option<SessionKey>`:
  服务端对全局查询回显 `null`,解码必须容忍;`default` 让缺省字段同样安全。
- 分发策略按查询种类区分:
  - **限定查询**(回包 `session_id` 为 `Some`):沿用既有语义,该会话的 loop
    集合被整体替换(服务端对该会话是权威的)。
  - **全局查询**(回包 `session_id` 为 `None`):以服务端回传的
    `profile_id` 为权威边界 —— 先清空**该 profile 内所有会话**的镜像,
    再按每条记录自身的 `record.session_id` 分组写入。其他 profile 的会话
    不受影响。
    (初版决策为"不清空未出现的会话",但实测导致矛盾态:服务端已删除全部
    loop 时,状态栏显示 `0 loop(s)`、转录显示"没有循环",而指示行仍挂着
    过期 loop。回包中的 `profile_id` 恰好给出了精确的权威范围,故改为
    profile 内清空。)
- profile 回退:`active_session_profile_id()` 为 `None` 时改用启动 profile
  (`onboarding.launch_profile_id`),避免服务端兜底到 `main` 而查不到用户
  实际 profile 下的 loop。
- 转录清单渲染:**全局查询**在每行追加该 loop 所属会话(取 `record.session_id`
  的 base key,去掉 profile 前缀以免每行重复同一 profile);**限定查询**不加
  该列(所有 loop 同属一个已知会话,重复显示是噪音)。状态栏计数行为不变;
  全局查询的计数为各组保留数之和。

## 边界

### Allowed Changes
- src/model.rs
- src/store.rs
- src/app.rs
- tests/m15_autonomy_dispatch_contract.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不改变限定查询(有活跃会话)的既有分发语义。
- 不改变 `/loop list` 的请求侧契约(已正确)。
- 不清空回包 `profile_id` 之外的会话镜像。
- 不新增 crate 依赖。

## 排除范围

- 服务端回包结构调整(客户端单方面容忍即可)。
- loop 记录跨 profile 的展示策略。

## 完成条件

场景: 全局查询回包可以解码
  测试: loop_list_result_decodes_null_session_id
  假设 服务端对全局查询回包 session_id 为 null
  当 解码 LoopListResult
  那么 解码成功
  并且 session_id 字段为 None

场景: 全局查询按记录自身会话分发
  测试: global_loop_list_distributes_by_record_session
  假设 全局查询回包含两条分属不同会话的 loop
  当 应用该结果
  那么 每条 loop 进入其自身会话的镜像
  并且 两个会话各持有一条 loop

场景: 限定查询沿用整体替换语义
  测试: scoped_loop_list_replaces_that_session_set
  假设 限定查询回包 session_id 为该会话且含一条 loop
  当 应用该结果
  那么 该会话镜像恰好持有这一条 loop

场景: 全局查询清空同 profile 内已消失的 loop
  测试: global_loop_list_clears_vanished_loops_in_scope
  假设 会话 A 已有一条 loop,全局查询回包 profile 为同一 profile 且只含会话 B 的 loop
  当 应用该结果
  那么 会话 A 的 loop 被清除
  并且 会话 B 持有回包中的 loop

场景: 全局查询不触碰其他 profile 的会话
  测试: global_loop_list_leaves_other_profiles_untouched
  假设 会话 A 属于 other profile 且已有一条 loop,全局查询回包 profile 为 kimi
  当 应用该结果
  那么 会话 A 的 loop 仍然保留

场景: 全局查询返回空时镜像与计数自洽
  测试: empty_global_response_clears_scope_so_ui_agrees
  假设 会话 A 在 kimi profile 下已有一条 loop
  当 应用 profile 为 kimi 的空全局回包
  那么 会话 A 的 loop 被清除
  并且 状态栏计数与镜像一致为零

场景: 全局清单每行标注所属会话
  测试: global_loop_list_block_labels_each_session
  假设 全局查询回包含两条分属不同会话的 loop
  当 渲染转录清单
  那么 清单包含第一条 loop 所属会话的标识
  并且 包含第二条 loop 所属会话的标识

场景: 限定清单不重复会话标注
  测试: scoped_loop_list_block_omits_session_column
  假设 限定查询回包含一条 loop
  当 渲染转录清单
  那么 清单不包含会话标识列

场景: 无会话时回退到启动 profile
  测试: loop_list_falls_back_to_launch_profile_without_session
  假设 没有活跃会话且启动 profile 为 kimi
  当 派发 /loop list
  那么 请求的 profile_id 为 kimi
