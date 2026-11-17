spec: task
name: "进化环采集多机部署的信号:远程 ACK、打回、单元负哨"
id: task-olp-evo-cluster-signals
status: implemented
tags: [olp, evolution, harvest, cluster]
satisfies: [REQ-OLP-EVO]
estimate: 0.5d
---

## Intent

进化环靠采集哨从黑板上收集内环摩擦的证据。多机部署改变了这些证据的写法:单元不能在条目
下面原地写 ACK,只能把带条目号的署名行追加到板尾;大外环的打回、单元运行时守护的负哨也都
是追加在板尾、按 `#N` 归属的行。采集哨原先只认行首的 `ACK(blocked):`/`ACK(wontdo):` 和
改判、R2 记档,集群里最有价值的实测信号(哪类阻塞反复出现、哪些活被打回、哪些单元在报错
或空转)一条也收不到。本任务让采集哨识别这三种行,并让 retro 按症状而不是按单元分组。

## Decisions

- `scripts/olp-evo-harvest.sh` 的活板扫描新增三种行首定式(剥掉行首 `> `、`**` 与空白后
  匹配,正文提及不算),只在既有定式都不命中时判断:
  - `内环(<署名>)·[#N ]ACK(blocked|wontdo)…`,条目号也可以写在冒号后
    (`…ACK(blocked): #N…`)→ 既有的 `ack_blocked` / `ack_wontdo`。`ACK(done)` 不触发。
  - `外环(<署名>)·打回(#N)` → 新 trigger `returned`。
  - `单元(<署名>)·负哨(#N)` 或 `负哨(<任务名>)` → 新 trigger `unit_alert`。
- 归属:卡的 identity 用行里写明的条目号(`board:<路径>#<N>#<kind>#<行哈希>`),不用它物理上
  所在的条目。行里没有条目号时(署名 ACK 不带 `#N`、负哨的任务名不是数字)退回所在条目。
  identity 的 kind 段:blocked、wontdo、returned、alert。同一行重跑不重复落卡。
- 既有行为不变:行首的 `ACK(blocked):`/`ACK(wontdo):`、改判、R2 记档照旧;带引用前缀的
  普通 `> ACK(blocked):` 仍不触发;采认、搁置、单元的推送登记不触发。
- `scripts/olp-evo-lib.py`:`returned`、`unit_alert` 加入板类 trigger;层提示
  `returned → Verification`、`unit_alert → Execution`。分组文本剥掉三种署名前缀
  (`外环(…)·打回(#N):`、`内环(…)·[#N ]ACK(…)[: #N…]:`、`单元(…)·负哨(…):`);
  `unit_alert` 另外去掉 `turn=`、`seq=`、`pid=`、`age=` 这些每次都不同的字段——告警种类与
  状态才是症状。于是两台单元、两张卡上的同一种症状归为一个候选。
- 这是对 task-req-olp-evo-p0(trigger 清单)与 task-req-olp-evo-p1(层提示表、分组文本)的
  增补;两份契约里既有的场景与断言都不动。
- 采集仍然只在 hub 上运行(主机守卫)。回放基线夹具不含新定式,基线不变。

## Boundaries

### Allowed Changes
- scripts/olp-evo-harvest.sh(仅活板扫描里新增三种定式与归属)
- scripts/olp-evo-lib.py(trigger 清单、层提示表、分组文本)
- tests/olp_evo_harvest.rs、tests/olp_evo_retro.rs(各新增一个测试,既有测试不动)
- docs/OLP_OUTER_BOOT.md(仅 §7)
- specs/task-olp-evo-cluster-signals.spec.md
- specs/task-olp-board-inbox.spec.md(一句说明)

### Forbidden
- 不改既有定式的判定、卡片格式、游标与提交协议。
- 不改回放夹具与既有断言。
- 不改 docs/OLP_OUTER_BOOT.md 的 §0–§6。

## Out of Scope

- 从单元的 events.jsonl 直接采集(单元的事件经负哨行到达 hub)。
- 把 `tool_failed` 收成卡(转发器默认不转发它)。
- metrics 脚本的展示调整(它按 trigger 通用统计,新 trigger 自动计入)。

## Completion Criteria

### Rule: harvest — 识别与归属
Scenario: 多机部署的行首定式落卡并按 #N 归属(critical)
  Tags: critical
  Test: olp_evo_harvest_cluster_forms_attach_by_entry_number
  Given 一块板:#212、#213 两个条目之后,板尾依次是带条目号的署名 ACK(blocked)、条目号在冒号后的署名 ACK(wontdo)、署名 ACK(done)、署名 checkpoint、打回、采认、搁置、负哨、推送登记,以及一行在正文里提到这些字样的文字
  When 运行采集,再运行一次
  Then 恰好四张卡,trigger 依次为 ack_blocked、ack_wontdo、returned、unit_alert;identity 分别归到 #212、#213、#212、#212,而不是它们物理所在的 #213;第二次运行不新增卡

### Rule: retro — 按症状分组
Scenario: 多机部署的卡按症状分组,不按单元和卡号分组
  Test: olp_evo_retro_groups_cluster_cards_across_units
  Given 五张卡:两台单元在两张卡上各报一次同样的停滞(turn 与时长不同),两台单元各有一条同样原因的署名 ACK(blocked)(两种写法),以及一张打回
  When 运行 retro
  Then 两次停滞归为一个候选,两条阻塞归为一个候选;unit_alert 的层为 Execution,returned 的层为 Verification;候选的键里没有单元、内环或外环的署名
