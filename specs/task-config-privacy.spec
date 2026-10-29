spec: task
name: "Private config publication and diagnostic display"
inherits: project
tags: [config, cli, privacy, persistence]
---

## 意图

保存配置时不截断正在使用的文件，并限制可能包含认证 token 的配置权限。
默认诊断输出遮盖认证 token；只有用户显式要求才显示原文。

## 已定决策

- 先完整写入同目录私有临时文件并同步，再原子替换配置。
- Unix 新建和替换配置均为 0600，保留用户管理的符号链接。
- 尊重已有配置的只读权限；保存失败保留旧文件且清理临时文件。
- config show 默认遮盖 auth-token 及 auth_token，保留其他 JSON 值。
- --show-secrets 显式输出原文；默认输出遇非法 JSON 报错，不回退原文。
- 不改变启动配置优先级，不新增依赖。

## 完成条件

场景: 默认展示不泄漏 token 且不改磁盘配置
  测试: config_show_masks_tokens_without_changing_saved_values
  假设 配置含规范名和旧别名的认证 token
  当 运行 config show
  那么 token 被遮盖且磁盘内容不变

场景: 用户可显式查看 token 原文
  测试: config_show_requires_explicit_opt_in_for_original_tokens
  假设 配置含认证 token
  当 用户显式传入 --show-secrets
  那么 输出原文

场景: 非法 JSON 不绕过默认遮盖
  测试: config_show_does_not_fall_back_to_raw_malformed_contents
  假设 配置 JSON 非法且含 token
  当 运行默认 config show
  那么 返回错误且输出不包含 token

场景: 原子替换保留旧 inode 与未知键
  测试: config_save_replaces_the_file_instead_of_truncating_the_old_inode
  假设 配置有旧 inode 的硬链接
  当 保存更新的 UI 设置
  那么 旧 inode 内容不变且新配置保留认证及未知键

场景: 失败保存保留旧配置
  测试: config_save_failure_keeps_read_only_original_and_no_temporary_file
  假设 配置被用户设置为只读
  当 尝试保存
  那么 返回错误且旧文件与目录内容不变

场景: Unix 新旧配置均采用私有权限
  测试: saved_config_is_private_for_new_and_existing_files
  假设 配置缺失或已有配置为 0644
  当 在 Unix 保存配置
  那么 已发布配置权限为 0600

场景: Windows 发布失败不截断旧文件
  测试: config_publish_failure_preserves_original_and_cleans_temporary_file
  假设 另一个文件句柄允许读写但禁止替换旧配置
  当 完成临时文件写入后尝试发布
  那么 保存报错且旧文件内容不变，临时文件被清理

场景: 保存不替换用户管理的符号链接
  测试: config_save_preserves_user_managed_symlink
  假设 Unix 配置路径是用户管理的符号链接
  当 保存配置
  那么 链接保留且其目标被更新

场景: 配置保护文案支持中英文
  测试: config_privacy_messages_resolve_in_both_locales
  假设 使用 en 或 zh 语言
  当 解析配置保护帮助与错误文案
  那么 返回对应译文而非 key
