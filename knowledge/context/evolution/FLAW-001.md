---
kind: context
id: FLAW-001
title: "围栏 peer 是无构建缓存的裸克隆,Rust peer 冷编译耗尽 50 迭代"
repo: octos-org/octos
layers: [Lifecycle, Execution]
status: closed
severity: S2
recurrence: 1
fingerprint: lifecycle/fenced-peer-clone-no-build-cache
issue: https://github.com/octos-org/octos/issues/2236
cards: []
pr: https://github.com/octos-org/octos/pull/2240
merged: 2026-09-04 (e08c2715)
filed: 2026-09-04
---

## 症状

`peer_handoff` 围栏(显式 `worktree=true` 或 #20a 智能围栏自动命中)时,peer 的
工作树是 `peers/<slug>/wt` 下的全新 `git clone --no-hardlinks`,没有 `target/`。
peer 的第一次 `cargo build`/`cargo test` 从零编译整个 workspace;单次 cargo 观测到
147 s 与 500 s;50 迭代预算耗尽于等编译,`result-1.md` 写
"did not complete within 50 iterations",产出未 commit 并残留一个 E0596。

## 责任步

peer 阶段的第一条 `cargo test -p octos-cli --features api …`:该步在冷树里形成了
一个 8.2 GB 的独立 `wt/target/`,把后续每一步都拖成分钟级。

## 根因

`crates/octos-cli/src/peers/mod.rs` L1596–L1626:围栏是 `git clone --no-hardlinks`,
克隆天然排除 gitignored 的 `target/`;`crates/octos-cli/src` 里没有任何代码路径引用
`CARGO_TARGET_DIR` 或其它构建缓存提示。克隆隔离 `.git` 的理由成立(沙箱内 git 可用、
peer 间 refs 隔离),代价是 Rust peer 丢掉了最需要的热缓存。

## 锚点

- `crates/octos-cli/src/peers/mod.rs` L1596–L1626(围栏创建)
- `crates/octos-cli/src/peers/mod.rs` L1941–L2086(#20a 智能围栏默认)

## 复发史

- 2026-09-04,octos 活板 #45(issue #2234 修复),goal 未建成(见 FLAW-002),
  peer `fix-2234-linux-keystore`,事件 `peer_staged` 2026-09-03T17:58:49Z。

## 保护门

- 围栏的 `.git` 隔离语义不变(克隆理由在 `peers/mod.rs` 注释里成立)。
- 非 Cargo workspace 不注入任何环境变量。
- 沙箱只对共享 target 目录放行写权限,不放行 `wt/` 之外的其它路径。

## 异议

(第二外环署名批注,只追加)

## 结案

- 2026-09-04:issue 经维护者 triage 接受,按 SDD 契约修复并合并(https://github.com/octos-org/octos/pull/2240,main e08c2715);验收=契约 6/6 lifecycle + 主审隔离复验 + CI 矩阵。观察窗:后续两个完成的 goal 内同指纹卡片应为 0。
