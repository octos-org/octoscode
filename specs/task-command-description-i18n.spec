spec: task
name: "斜杠命令说明完整本地化"
inherits: project
tags: [tui, menu, i18n, ux]
estimate: 0.1d
---

## 意图

中文界面的斜杠命令弹窗中，少数命令仍把英文说明直接写在注册表里，导致
本地化查询把整句英文当作缺失的翻译键并原样显示。进一步审计还发现恢复/
回退菜单、活动面板标题和若干运行时状态提示绕过了翻译目录。本任务让这些
应用自有文案统一引用翻译键，并补齐中英文文案。

## 已定决策

- 核心命令注册表只保存翻译键，不保存面向用户的说明原文。
- 中英文均必须解析为非空文案；新增核心命令也受同一契约约束。
- `/btw`、`/resume`、`/rewind` 的命令行为、别名和可用性规则保持不变。
- 服务器返回的内容、协议标识符、模型回复与用户输入保持原样，不对动态数据
  做客户端机器翻译；仅本地化应用自己拼接的标签和提示。

## 边界

### Allowed Changes
- src/menu/registry.rs
- src/menu/providers.rs
- src/app/render.rs
- src/app/activity_nav.rs
- src/app/tests.rs
- src/event_loop.rs
- src/model.rs
- src/store.rs
- src/lib.rs
- locales/en.yml
- locales/zh.yml
- specs/**

### Forbidden
- 不改变斜杠命令派发或能力门控。
- 不修改后端生成的助手消息内容。

## 排除范围

- 已写入会话记录的模型回复语言；该内容由后端模型生成，不属于 TUI 静态文案。

## 完成条件

场景: 所有核心命令说明均可本地化
  测试: core_command_descriptions_resolve_in_en_and_zh
  假设 核心斜杠命令注册表已构建
  当 逐项检查命令说明并以 en 与 zh 解析
  那么 每项说明均为 command.*.desc 翻译键
  并且 两种语言均解析出非空文案

场景: 审计发现的应用自有文案均有双语翻译
  测试: audited_user_facing_strings_resolve_in_en_and_zh
  假设 命令菜单、恢复/回退菜单、活动面板和运行时状态提示的翻译键集合
  当 分别以 en 与 zh 解析每个键
  那么 两种语言均解析出非空且不同的文案

场景: 本地化不改写英文协议值
  测试: activity_navigator_localizes_known_values_without_rewriting_english_protocol_values
  假设 活动导航器收到已知操作词或服务器自定义值
  当 在英文和中文界面分别生成显示值
  那么 英文界面保持原始协议值
  并且 中文界面仅翻译已知的应用标签，未知值保持原样
