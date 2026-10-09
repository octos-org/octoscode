---
name: olp-outer
description: 外环上岗(已收编)——请用统一入口 /octoloop 的 outer 模式;本卡仅作旧名转发
---

# /olp-outer → /octoloop(outer 模式)

本 skill 的完整上岗规程已收编进统一入口 **`/octoloop`** 的
**outer 模式**(读规程 → 发现现场 → 接管职责),权威文档不变:
`docs/OLP_OUTER_BOOT.md` + `docs/OUTER_LOOP_PROTOCOL.md`。

若项目显式采用 `schema="olp-board/v1"`，同时读
`docs/OLP_STRUCTURED_BOARD.md`：用 item/receive/ack/review 工具成对写入，
用 inbox/sentinel 的账本投影判定完成，DRIFT 走精确 recovery 或 void。没有事件的
项目仍按 legacy 流程。opt-in 后未闭合围栏以 opener 字节证据阻塞，补匹配闭合行
只恢复写入、不把围栏示例变成 recovery 对象；sentinel 的 `TIMEOUT` JSON 行以
退出码 3 终止等待。本转发卡不宣称已普遍切换。

请改用:`/octoloop`,选 outer 模式。全景能力见 `docs/OCTOLOOP_FEATURES.md`。
