---
kind: context
id: FLAW-sample
title: "样本:三修复三预防"
repo: example/repo
layers: [Lifecycle, Execution]
status: filed
severity: S2
recurrence: 2
fingerprint: sample/fixture
req: REQ-OLP-EVO
cards: []
filed: 2026-09-05
---

## 症状

样本 FLAW:三修复三预防,req 绑定 REQ-OLP-EVO。

## 责任步

`src/foo/bar.rs` 的 `handle()` 缺少超时。

## 根因

超时逻辑从未实现。

## 锚点

- `src/foo/bar.rs` L10–L20

## 复发史

- 两次。

## 修复

- 修复项一:加 5s 超时
- 修复项二:超时可配置
- 修复项三:超时打点

## 预防

- 预防项一:契约测试钉超时
- 预防项二:clippy lint 警告
- 预防项三:文档记录超时语义

## 异议

(无)

## 结案

(待)
