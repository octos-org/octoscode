---
slug: pr630-k3
outcome: completed
updated_unix: 1788922186
turn: 3
---

外层新增反例裁决完成，**outcome 修订为 fail**（推翻 turn 2 的 pass-with-observations）。

**四项反例裁决（动态证据引自外层 adversarial.log，非我运行；成立性为我独立静态复核）：**

1. **反例 1 — 采纳，新增回归（高）**：核实链确凿 — hello grace 到期路径（2494-2508）只 `take()` barrier 并 flush，**不清** `stdio_child_served_frame`；该 flag 唯一置位点是 `note_connection_served_frame`（2458-2468），而 `stdio_child_startup_pending`（2474-2479）只看这个 flag。因此对零输出的 stdio child，2527 的 `startup_pending` 抑制**永久生效**，attempts 停在 1（外层实测 `left:1 right:2`）。这正是 PR 注释自己点名的头号场景——**修复承诺未兑现**，我和 GLM 初审都把"请求是延迟而非被无视"误当成覆盖了"grace 已到期但首帧仍无"的中间态。

2. **反例 2 — 采纳，新增回归（中）**：执行顺序确凿 — `next_event` 先跑超时检查（3133）后 poll driver（3137）；retry 的 retain（2540-2542）删掉旧 pending id 后，已在 driver 通道排队但未 decode 的成功帧在 3810-3812 被**静默丢弃**（无事件、probe 不清除），且 `sent_at` 已重置多盲 5s。较旧代码（pending 永不被删、迟到成功总能被接受）是严格回归。我 turn 2 对 O2"幂等无害"的裁决被推翻。

3. **反例 3 — 采纳，修订双方测试证明力**：测试 1 收到成功即 break，不证明终止；`count==2` 靠 fixture 截断间接排除重连而非直接证明；测试 2 直接构造 attempts=4，不证明 0→4 链；两测试都从反例 1 的盲区（fixture 回了 hello error → 首帧已 serve）旁边绕过。我初审"'恰好 2 次'隐式证明终止"结论已修订降级。

4. **反例 4 — 采纳，语义澄清**：stdio `send_text` 仅是 mpsc `try_send` 入队（1334-1341），真正写 stdin 在 writer task（1184-1208）；"wire clock" 实为 "channel-enqueue clock"，writer 写失败还会白烧一份预算（有界）。

cross-k3.md 已追加"五、外层新增反例裁决"小节（保留 turn 2 内容为历史记录并标注推翻），frontmatter outcome=fail、turn=3；result.md 已更新。修复方向建议已附。等待 master 下一步指令。
