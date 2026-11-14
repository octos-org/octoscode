spec: task
name: "olp/v3:集群里复验落在单元,大外环只规划、派发、汇总;内环免沙箱"
id: task-olp-v3-cluster-roles
status: implemented
tags: [olp, cluster, protocol, v3]
estimate: 1.5d
---

## Intent

operator 定调(2026-11-14):集群里大外环逐卡复验会成为瓶颈——单元是集群,外环只有一个。
大外环的工作改为:从用户指定的项目全景台读目标 → 规划成卡、派给单元 → 汇总集群进展回报
全景台 → 循环到目标完成。复验落在单元:单元用多个并行内环对抗复验,实现方与复验方用哪些
agent 由大外环按卡指定,复验通过才交回。单元上的内环一律免沙箱,agent 可以代为启动。

## Decisions

- 协议升到 olp/v3(R6:信道语义变更必须升版本)。R2 增补"集群里复验落在单元";新增 R8
  内环免沙箱;附录 C 改写三道关:单元内对抗复验 → hub 收单(只核对手续:复验记录、提交号、
  `ACK(done)` 的证据、改动范围;不跑测试、不审代码)→ 人审核。单机非集群的双环不变。
- 派卡:`olp-dispatch.py --impl <种类> --review <种类>[,<种类>…]`,卡上写
  `impl: <种类>` 与 `review: <种类>, <种类>` 两行(在 `unit:` 行之后)。种类是 herdr 的
  agent 种类名,`[a-z][a-z0-9-]{0,31}`,不写死在工具里。给了 `--impl` 就必须给 `--review`;
  复验方不重复。角色是派发内容的一部分,参与幂等键;没有角色的卡键不变。
- 卡上带角色时,`--wake` 发出的话告诉被唤醒的 agent:它是单元主持、不要自己实现,按 skill
  的「模式 unit」组织,并给出实现方与复验方。
- 单元复验板 `.octos/UNIT_BOARD.md`(不入库,`olp-init.sh` 加进 `.gitignore`)记轮次
  `#N-rK`:`实现(<种类>)·提交`、`复验(<种类>)·通过|打回`。
- hub 的 pre-receive 钩子拒收"带复验记录却没有单元号"的推送:那是单元的克隆没经 `serve`
  (单机部署漏了 `connect-local`),照收会绕开推送规则、黑板也不登记——实测里单元主持
  自建的克隆正是这样把分支推上去的。hub 本机不带记录的推送照旧不受限。
- 复验署名可以写出多个复验方:`olp-git-hub.py` 与 `olp-task-push.sh` 的署名字符集加 `+`
  (如 `m07-codex+claude`)。
- 归档的桩保留 `impl:`、`review:`、`verify-of:` 行;收件箱显示角色,`--json` 带
  `impl`、`review`。
- 心跳新增内置巡检 `full`(每个内环都以完全访问启动),默认巡检改为 `agent,disk,full`:
  不满足 R8 的单元判为降级、不派卡。
- 汇总:`olp-summary.py --board <板> [--repo <hub 仓库>] [--name <项目名>] [--json]`,只读,
  收卡(含角色与所处分区)、各区计数、单元状态、本地 PR、待答提问、未了结卡上的负哨;某个
  数据源读不到时该项记 `unavailable`,其余照常。全景台不在 OctoLoop 里,由用户指定。
- skill 新增 unit 模式(单元主持)与 outer 模式的集群循环;inner 模式写明三种内环的免沙箱
  启动参数;删除"免沙箱启动永远留给 operator 亲手";四处受保护小节的哈希同步更新。上岗卡的
  红线为集群单元开出例外。

## Boundaries

### Allowed Changes
- docs/OUTER_LOOP_PROTOCOL.md、AGENTS.md、.octos/loop.md(版本号)
- docs/OCTOLOOP_AGENTS.md、docs/OCTOLOOP_AGENTS.zh-CN.md、docs/OLP_CLUSTER.md、docs/OCTOLOOP_FEATURES.md
- skills/octoloop/AGENTS.md、.claude/skills/octoloop/SKILL.md
- scripts/olp-dispatch.py、scripts/olp-inbox.py、scripts/olp-board-archive.py、scripts/olp-unit.py
- scripts/olp-git-hub.py、scripts/olp-task-push.sh、scripts/olp-init.sh、scripts/olp-summary.py
- tests/olp_v3_cluster.rs、tests/olp_unit.rs、tests/olp_contract.rs、tests/olp_evo_retro.rs
- specs/task-olp-v3-cluster-roles.spec.md

### Forbidden
- 不改 hub 仓库的推送规则(只能推 task/N、必须带复验记录、记录的提交号等于分支头)。
- 不改 docs/OLP_OUTER_BOOT.md 的 §0–§6(受哈希保护)。
- 单元复验的组织由 skill 规定、由 agent 执行;本任务不写调度单元内环的程序。

## Out of Scope

- 全景台本身(由用户指定的工具)。
- 一台机器上并行多张互不相干的卡(单元下的车道)。

## Completion Criteria

Scenario: 大外环按卡指定实现方与复验方(critical)
  Tags: critical
  Test: v3_dispatch_names_the_implementer_and_reviewers_per_card
  Given 一块空板
  When 派一张带 `--impl octoscode --review codex,claude` 的卡;同标题同正文但换复验方再派;原样重派第一张;派一张不带角色的卡;再用四组非法角色参数派
  Then 卡上紧跟 `unit:` 有 `impl:`、`review:` 两行;带角色的唤醒告诉对方它是单元主持并指向模式 unit;换复验方得到新编号;原样重派返回原编号;不带角色的卡格式不变;收件箱显示角色,JSON 带 impl 与 review;非法参数以退出码 2 拒绝且板不变

Scenario: 复验记录可以写出多个复验方,归档后角色行留在桩里(critical)
  Tags: critical
  Test: v3_verification_record_names_several_reviewers_and_archive_keeps_roles
  Given 一张指定了实现方与两个复验方的卡,本机的 hub 仓库与接上的工作克隆
  When 以 `--verified-by m07-codex+octoscode` 推送,写采认,归档
  Then 推送被接受,黑板登记的复验署名是 `m07-codex+octoscode`;另一个直接以路径为远程的克隆带复验记录推送被拒并提示 connect-local,不带记录的 hub 自身推送照常;归档后的桩保留 unit、impl、review 三行而正文进入归档文件

Scenario: 大外环汇总项目进展,交给用户指定的全景台(critical)
  Tags: critical
  Test: v3_summary_collects_cards_units_prs_and_alerts
  Given 一张进行中带角色的卡、一张已采认的卡、一条负哨、一台登记了但没有心跳的单元
  When 以 `--json` 与文本两种方式汇总;给一个读不到的仓库再汇总;给一块不存在的板
  Then JSON 带项目名、各区计数、卡的角色、单元状态与负哨,没有 --repo 时不带本地 PR;文本列出未了结的卡与角色、单元、负哨,不列已了结的卡;读不到的仓库记为 unavailable 而其余照常;板不存在时退出码 2

Scenario: 内环没有全部免沙箱的单元不派卡(critical)
  Tags: critical
  Test: unit_heartbeat_recognises_any_kind_of_inner_loop
  Given 一个以默认巡检发心跳的 Claude Code 单元
  When 内环不带完全访问开关;再带 `--dangerously-skip-permissions`
  Then 前者被判为降级、原因写明 full 巡检与免沙箱;后者为正常
