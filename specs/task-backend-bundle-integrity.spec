spec: task
name: 后端 bundle 安装完整性
inherits: project
tags: [installer, integrity]
---

## 意图

所有受支持平台的 pinned 后端 bundle 必须取得并通过发布的 SHA256 校验，才可
解包发布。历史 release override 同样遵循该要求，无 checksum 时应手动安装。

## 完成条件

场景: checksum 获取故障不覆盖原安装
  测试: bundle_checksum_fetch_failures_preserve_existing_installation
  当 checksum 返回 404 或 500 或响应头之前断开
  那么 自动安装失败且原二进制逐字节保持不变

场景: 摘要不匹配不创建安装目录
  测试: bundle_checksum_mismatch_never_creates_installation
  当 下载内容与 checksum 不一致
  那么 不解包或创建安装目录

场景: 校验成功才发布二进制
  测试: bundle_checksum_verified_download_installs_binary
  当 真实本地 HTTP 返回有效 checksum 和 bundle
  那么 仅校验通过的内容进入安装目录
