spec: task
name: "OLP v1:ACK 语法、车道模板、协议契约测试(octoscode)"
tags: [olp, protocol, octoscode]
satisfies: [REQ-OLP-PROTO]
estimate: 1d
---

## 意图

OLP v0 的 R1(ACK)与 R2(诚实验证)靠自然语言约定;分歧路径从未定形;
苦力车道机制存在但无开箱配置。本任务在 **octoscode 仓库**落地协议 v1 的
客户端侧:ACK 定式语法(含 wontdo 分歧分支)、sub_providers 车道配置
模板与搭配矩阵、以及把协议文档本身钉住的契约测试。result.md 的
`verified`/`protocol` 字段由 octos 侧写入(task-req-olp-exec-peer),
本合约只固化 schema 文档与消费侧约定。

## 已定决策

- ACK 语法定式:`ACK(done|wontdo|blocked): <说明>`,写入
  `docs/OUTER_LOOP_PROTOCOL.md`(升 v1)与 `AGENTS.md`;对 `wontdo`,
  外环只能"接受"或"升级 operator",不得对同一条目再次打回——该规则
  进 OLP 文档 R1 修订。
- result.md frontmatter v1 schema 文档化:必含 slug、outcome、
  updated_unix、turn、verified、protocol;未知字段消费方必须忽略。
- `sub_providers` 车道模板落 `docs/OUTER_LOOP_PROTOCOL.md` 附录:
  cheap/strong 两例,description 写明选道标准;附双环搭配矩阵
  (分析/验证→cheap,实施→primary,keeper→primary)。
- 新增契约测试 `tests/olp_contract.rs`(纯文档契约,不依赖 octos):
  校验黑板 ACK 行全部匹配 v1 语法、车道模板 TOML 片段可解析、OLP 文档
  声明的 schema 字段清单与本合约一致、AGENTS.md 引用的协议版本与 OLP
  文档一致。

## 边界

### Allowed Changes
- docs/OUTER_LOOP_PROTOCOL.md
- docs/OUTER_LOOP_REVIEW.md
- AGENTS.md
- tests/olp_contract.rs
- specs/**

### Forbidden
- 不改 src/**(本合约零生产代码)。
- 不改黑板历史条目的既有 ACK 内容(旧格式由测试豁免清单覆盖,不重写历史)。
- 协议版本号变更必须同步 AGENTS.md 与 OLP 文档两处(测试钉住)。

## 排除范围

- verified 字段的写入(octos 侧,task-req-olp-exec-peer)。
- octoscode 对 result.md 的运行时消费/渲染。
- steer/observability 相关文档(随各自合约更新)。

## 完成条件

场景: 黑板 ACK 全部符合 v1 语法(critical)
  测试: olp_ack_lines_match_v1_grammar
  假设 docs/OUTER_LOOP_REVIEW.md 含历史与新增 ACK 行
  当 契约测试解析所有 ACK 行
  那么 v1 生效日期后的每行匹配 ACK(done|wontdo|blocked) 语法,
       历史行命中豁免清单

场景: 非法 ACK 状态词被测试拒绝
  测试: olp_ack_rejects_unknown_status
  假设 一行状态词不在 done/wontdo/blocked 集合内的 ACK 样本
  当 语法校验函数处理它
  那么 返回不匹配

场景: 车道模板可解析且含选道说明
  测试: olp_lane_template_parses
  假设 OLP 文档附录中的 sub_providers TOML 模板片段
  当 契约测试用 toml 解析器解析
  那么 解析成功且每条 lane 均有非空 description

场景: 协议版本双处一致
  测试: olp_version_consistent_across_docs
  假设 AGENTS.md 与 docs/OUTER_LOOP_PROTOCOL.md
  当 契约测试提取两处协议版本号
  那么 两者相等且为 olp/v1

场景: schema 字段清单与合约一致
  测试: olp_result_schema_fields_documented
  假设 OLP 文档的 result.md schema 一节
  当 契约测试提取字段清单
  那么 恰为 {slug, outcome, updated_unix, turn, verified, protocol}
