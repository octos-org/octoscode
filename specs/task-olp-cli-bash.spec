spec: task
name: "OLP Bash 工具统一 CLI 入口"
inherits: project
tags: [cli, olp, bash]
---

## 行为

- 提供 `octoscode olp init [directory]`（默认当前目录）、`watch`、
  `board-append`、`evo harvest`、`evo index`、`evo metrics`。
- 继续调用现有 Bash 脚本；内嵌脚本与 Python 同伴，不依赖源码目录。
- init 只在指定的已存在目录运行，保留原脚本的幂等行为和依赖检查退出码。
- stdin/stdout/stderr 继承，参数独立传递、不拼接 shell 命令；返回脚本退出码。
- `--bash` 优先于 `OCTOSCODE_BASH`；Windows 优先查找 Git Bash，最后回退 PATH bash。
- 不增加 PowerShell 实现，不承诺 Windows 已提供 flock/Python 或 Linux 主审锁。
- 用户指南与 OctoLoop skill 的 init 节使用 `octoscode olp init .`。
  此次文档变更有意更新阶段 1 契约中 init 节的 golden；description、inner、
  自主性纪律等其他章节继续使用原基准，不能随本次更新而放宽校验。

## 场景与真实测试

- 帮助无需 Bash，未知命令/非法参数退出 2：
  `olp_help_and_invalid_arguments_never_launch_bash`
- Bash 缺失时提示安装/选择解释器，项目不变：
  `olp_missing_bash_has_actionable_error_without_project_changes`
- 从非源码目录初始化含空格/中文的目录，重复运行保留用户文件与忽略规则：
  `olp_init_uses_target_directory_and_preserves_existing_files`
- Unix 上黑板追加继承 stdin，正文不做 shell 展开，错误退出码透传：
  `olp_board_append_inherits_stdin_and_propagates_script_failure`
- 从非源码目录运行 index/metrics，加载内嵌 Python 同伴；harvest 缺板返回 2：
  `olp_evo_uses_bundled_python_companions_and_returns_harvest_status`
- watch 对新增字面子串命中，旧匹配行不出现在结果中：
  `olp_watch_matches_new_literal_lines`
- init 节采用 CLI 说明的新 golden，其余受保护章节保持原基准：
  `olp_evo_retro_skill_step5_and_protected_sections_golden`
