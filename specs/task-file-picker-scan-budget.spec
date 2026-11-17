spec: task
name: "Bound workspace file picker enumeration"
inherits: project
tags: [tui, composer, performance, files]
---

## 意图

打开 @ 文件选择器时，限制同步枚举、排序及待访问目录的总工作量。
条目预算在过滤之前计算，不能让大量被忽略的条目绕过限制。

## 已定决策

- 保留 breadth-first、目录内排序、文件数、深度和排除规则。
- 全扫描最多处理 16000 个原始目录条目，允许一次 lookahead 判断截断。
- 根目录包含在最多 2000 个已准入目录中，在入队时预留预算。
- 预算截断设置 truncated；恰好遍历完预算内目录不误报截断。
- 本次保持同步扫描，不承诺单次文件系统 I/O 的延迟上限。

## 完成条件

场景: 单个巨大目录不能无界枚举
  测试: directory_entry_budget_stops_before_enumerating_the_remainder
  假设 目录迭代器还包含任意多条目
  当 条目预算耗尽
  那么 只允许额外读取一个 lookahead 条目并标记截断

场景: 多个目录共享全局条目预算
  测试: directory_entry_budget_is_shared_across_directories
  假设 根目录下存在多个子目录
  当 前面的目录消耗全部条目预算
  那么 后续目录不继续无界扫描

场景: 宽目录树的待处理工作有界
  测试: wide_directory_tree_bounds_admitted_work
  假设 根目录包含多于准入预算的子目录
  当 扫描并入队子目录
  那么 只准入预算允许的目录并标记截断

场景: 精确预算边界保留完整结果
  测试: entry_budget_exactly_filled_does_not_claim_truncation
  假设 目录条目数恰好等于预算
  当 所有条目被列出
  那么 保留排序且不标记截断

场景: 保留现有排序
  测试: scan_lists_relative_paths_breadth_first_and_sorted
  假设 工作区不触及扫描预算
  当 打开文件选择器
  那么 路径顺序不变

场景: 保留现有排除规则
  测试: scan_skips_git_target_and_node_modules
  假设 工作区不触及扫描预算
  当 打开文件选择器
  那么 排除规则不变
