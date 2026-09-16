spec: task
name: "删除后重新添加同一模型"
inherits: project
tags: [tui, model, provider, server-truth, regression]
estimate: 0.125d
---

## 意图

用户从 `/model` 删除已保存的模型后，应能立即重新添加相同的模型与线路。
删除结果必须清除客户端的旧主模型标记，并以服务器返回的配置列表为准，不能让
model-config 把新草稿误判为已保存而反复折叠回“Add a model”。

## 已定决策

- `profile/llm/delete` 的成功结果是该 profile 的权威模型列表；即使列表为空也覆盖缓存。
- delete 响应不得进入兼容旧 save 响应的“无 pending 标记”分支。
- mutation 事件显式携带 upsert/delete/test 类型，不能靠可能残留的菜单状态猜测。
- 一旦已有当前 profile 的服务器模型快照，是否为已保存主模型必须按完整的
  family/model/route 判断；本地旧 label 不得覆盖服务器真相。
- 与服务端一致，将缺失的 route_id 归一化为默认线路 `official`。
- 清除本地已保存主模型标记，但不改变 test/upsert 的既有状态机。

## 边界

### Allowed Changes
- src/store.rs
- src/menu/providers.rs
- src/client_event.rs
- src/transport.rs
- specs/task-model-delete-readd.spec

### Forbidden
- 不修改 AppUI wire schema。
- 不改变普通添加、测试、保存或 fallback 的语义。
- 不新增依赖。

## 完成条件

场景: 删除主模型后可以重新添加相同模型
  测试: deleting_primary_then_readding_same_model_expands_model_config
  假设 当前 profile 已保存一个主模型且本地保留其 saved label
  当 删除成功返回空模型列表，再选择相同 family/model/route
  那么 删除结果覆盖旧主模型缓存并清除本地 saved 标记
  并且 model-config 展开 family/key/test/save 行而不是折叠回 Add a model

场景: 服务器仍有其他主模型时旧 label 不能复活已删除模型
  测试: server_primary_outweighs_stale_saved_label_when_readding_removed_model
  假设 本地 label 仍指向已删除模型，而服务器快照的主模型是另一个模型
  当 用户重新选择已删除模型的相同 family/model/route
  那么 model-config 以服务器快照为准并展开新草稿

场景: 删除 fallback 不清除仍存在的主模型
  测试: deleting_fallback_preserves_remaining_primary_state
  假设 当前 profile 有主模型与 fallback
  当 用户删除 fallback 且服务器结果仍返回主模型
  那么 客户端保留主模型已保存状态并采用服务器返回的模型列表

场景: 旧配置的缺省线路等价于 official
  测试: synthetic_official_route_matches_server_default_primary
  假设 服务器主模型没有显式 route_id，而当前草稿线路为 official
  当 model-config 判断草稿是否为已保存主模型
  那么 二者按相同默认地址处理并保持已保存状态的折叠界面

场景: 残留删除标记不能误分类保存响应
  测试: stale_removal_marker_cannot_reclassify_upsert_response
  假设 用户曾进入删除确认，客户端同时收到明确标记为 upsert 的成功响应
  当 reducer 应用该 mutation 事件
  那么 保存 pending 正常完成且响应不会走 delete 分支

场景: 传输层保留删除操作类型
  测试: protocol_preserves_profile_llm_delete_kind
  假设 客户端发出 profile/llm/delete 并收到成功响应
  当 传输层把 RPC 响应转换为客户端事件
  那么 mutation 事件类型明确为 Delete
