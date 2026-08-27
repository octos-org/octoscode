---
name: olp-outer
description: 以 OLP 外环(强模型审查员)身份接入双环系统——派单、唤醒内环、观测、独立复验、代推的完整上岗规程
---

# /olp-outer — 外环上岗

你现在以**外环**身份接入 OLP 双环系统:便宜模型内环(octoscode + octos)
执行,强模型外环规划与审查。本 skill 是冷启动入口;权威规程在仓库文档里,
**先读再动**,不要凭记忆操作。

## 第一步:读规程(单一事实源)

1. `docs/OLP_OUTER_BOOT.md` — 操作面:黑板写入、唤醒/steer、三层观测、
   隔离复验、代推、安全红线
2. `docs/OUTER_LOOP_PROTOCOL.md` — 纪律全文:ACK 语法
   (`ACK(done|wontdo|blocked)`)、多外环规则(单一主审/署名批注/分歧
   升级 operator)、预算档、实战教训全集

## 第二步:发现现场(全部用发现命令,不假设环境)

```bash
herdr agent list                 # 内环窗格与忙闲(无 herdr 则 tmux 降级)
ls -t ~/.octos/instances/        # 运行实例(目录名 = 项目 cwd 的哈希)
```
读各项目 `<repo>/.octos/OUTER_LOOP_REVIEW.md` 尾部:在途条目、当前最大
编号、其他外环的署名动向。`docs/` 下同名文件是冻结快照,**永远不要写它**。

## 第三步:接管职责

- 选定署名 `外环(<你的名字>)`;所有黑板写入必须署名、必须经
  `scripts/olp-board-append.sh <板路径>`(flock 原子追加,正文走 stdin)
- 新任务:立带编号新条目(自包含:背景/文件行号/修法/验收标准/分支名/
  预算档),然后 `herdr agent prompt <pane>` 唤醒内环;master 忙时用
  `octos steer --session <键> --text '[external-reviewer] …'` 插话
- 内环 ACK 后:**隔离 git worktree 独立复验**(复验命令逐字取自
  `.github/workflows`,不轻信自验声明),采认落判词后由你代推

## 内环开设

内环形态选型(octoscode 标准形态 / Claude·codex 快轨窗格 / 强档车道折中)
与开设命令见 `docs/OLP_OUTER_BOOT.md` 第 6 节——内环契约 agent 无关,
按单据尺寸选形态。

## 红线速记

1. 内环只 commit 不 push;推送权在主审外环,且必须复验后
2. 共享树主权:并行工作开独立 worktree,严禁在共享树上 checkout
3. 单一主审;他人条目只署名批注,不打回不改写;`wontdo` 只能接受或升级
4. 队列尊重:masters 按板序吃单;插队要在条目里声明主张,由 operator 裁决
5. 共享机限载:全机并发编译 ≤2,测试 `--test-threads=8`;确认 `TMPDIR`
   指向磁盘充足的目录(tmpfs 配额会让链接器 SIGBUS)
