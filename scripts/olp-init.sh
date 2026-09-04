#!/usr/bin/env bash
# olp-init.sh — 在当前项目目录一键铺设 OLP(Outer-Loop Protocol)双环脚手架。
#
# 做的事(全部幂等,存在即跳过,绝不覆盖已有内容):
#   1. 依赖体检:octoscode / octos / git / API key 环境,缺什么打印怎么装;
#   2. 生成 .octos/loop.md(内环维护循环)与 .octos/OUTER_LOOP_REVIEW.md
#      (外环审查黑板,含 v1 ACK 定式说明);
#   3. 黑板加入 .gitignore(分支无关,避免跨分支裂脑);
#   4. 打印标准启动命令与下一步清单。
#
# 刻意 **不做** 的事(操作者显式决策,脚本不代办):
#   - 不写任何 API key;
#   - 不给 serve 加 --danger-full-access(免沙箱是安全决策,见文档 0b 节);
#   - 不覆盖已有的 loop.md / 黑板 / AGENTS.md。
set -euo pipefail

say()  { printf '%s\n' "$*"; }
ok()   { printf '  [ok] %s\n' "$*"; }
todo() { printf '  [!!] %s\n' "$*"; MISSING=1; }
MISSING=0

say "== OLP init: 依赖体检 =="
command -v git >/dev/null 2>&1 && ok "git" || todo "git 未安装——请先安装 git"
if command -v octoscode >/dev/null 2>&1; then
  ok "octoscode ($(command -v octoscode))"
else
  todo "octoscode 未安装:npm install -g @octos-org/octoscode(或 brew / shell installer,见 README)"
fi
if command -v octos >/dev/null 2>&1 || [ -x "$HOME/.octos/bin/octos" ]; then
  ok "octos server(已装或已自动拉起过)"
else
  say "  [--] octos server 未见——首次运行 octoscode 会自动下载到 ~/.octos/bin(需网络);离线环境请手装:npm i -g @octos-org/octos"
fi
[ -n "${MOONSHOT_API_KEY:-}${OPENAI_API_KEY:-}${ANTHROPIC_API_KEY:-}" ] \
  && ok "检测到模型 API key 环境变量" \
  || say "  [--] 未检测到常见 API key 环境变量——onboarding 向导里粘贴亦可"

say ""
say "== 铺设 .octos/ 脚手架 =="
mkdir -p .octos

if [ -f .octos/loop.md ]; then
  ok ".octos/loop.md 已存在,跳过"
else
  cat > .octos/loop.md <<'LOOP'
# 维护循环(内环 master 每轮唤醒执行)

每次维护唤醒依次执行,全部完成才结束本轮:

1. 读 `.octos/OUTER_LOOP_REVIEW.md`(外环审查黑板)。
2. 若存在**未 ACK 的条目**(无 `ACK(` 定式行):取编号最小的一条,按其内容
   执行到完成(代码改动跑全量测试 + fmt + clippy 后原子 commit,只 add
   自己改的文件),然后在该条目下补一行 v1 定式 ACK:
   `ACK(done|wontdo|blocked): <说明>`。
3. 无未 ACK 条目:检查在途 goal 与测试基线,如实记录状态后结束本轮。

纪律:内环只 commit、不 push(推送权在外环,独立复验后代推);
黑板只追加、不改写既有行。
LOOP
  ok "生成 .octos/loop.md"
fi

if [ -f .octos/OUTER_LOOP_REVIEW.md ]; then
  ok ".octos/OUTER_LOOP_REVIEW.md 已存在,跳过"
else
  cat > .octos/OUTER_LOOP_REVIEW.md <<'BOARD'
# 外环审查通道(Outer-Loop Review)

> 外环审查员(强模型 agent)与内环(octos master 及其 peers)的持久黑板。
> **Master:每轮任务开始前读本文件;执行完每条意见后,在对应条目下追加
> v1 定式 ACK 行:`ACK(done|wontdo|blocked): <说明>`**——done 附
> commit/测试证据,wontdo 附理由(外环只能接受或升级 operator,不得重复
> 打回),blocked 附阻塞原因。
> 外环只追加带日期的条目,不删除历史;多外环时批注必须署名(如
> `外环(claude)` / `外环(codex)`),分歧升级 operator 裁决。

---

### 1. 黑板启用(由 olp-init.sh 生成)

本条无需执行,ACK 后即完成首次读写闭环验证。

ACK:
BOARD
  ok "生成 .octos/OUTER_LOOP_REVIEW.md"
fi

if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  if ! git check-ignore -q .octos/OUTER_LOOP_REVIEW.md 2>/dev/null; then
    printf '\n# OLP 黑板:分支无关的工作文件,不入库(防跨分支裂脑)\n.octos/OUTER_LOOP_REVIEW.md\n' >> .gitignore
    ok "黑板已加入 .gitignore"
  else
    ok "黑板已在 .gitignore 中"
  fi
  # 进化黑板(EVOLUTION.md)同样分支无关:仅在尚未被忽略时追加一行;
  # 整目录 .octos 已忽略的项目不重复追加(git check-ignore 判定)。
  if ! git check-ignore -q .octos/EVOLUTION.md 2>/dev/null; then
    # 41-r5b ⑨: if the last byte of .gitignore is not a newline, the
    # appended line would glue onto the existing last line — pad first.
    if [ -s .gitignore ] && [ "$(tail -c 1 .gitignore | od -An -tuC | tr -d ' ')" != "10" ]; then
      printf '\n' >> .gitignore
    fi
    printf '.octos/EVOLUTION.md\n' >> .gitignore
    ok "进化黑板已加入 .gitignore"
  else
    ok "进化黑板已被忽略"
  fi
fi

say ""
say "== 外环侦听哨(~/.octos/outer/watch-board.sh) =="
OUTER_DIR="$HOME/.octos/outer"
SENTINEL_SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/olp-watch-board.sh"
if [ -f "$SENTINEL_SRC" ]; then
  mkdir -p "$OUTER_DIR"
  if [ -f "$OUTER_DIR/watch-board.sh" ]; then
    ok "watch-board.sh 已存在,跳过(如需更新: cp $SENTINEL_SRC $OUTER_DIR/watch-board.sh)"
  else
    cp "$SENTINEL_SRC" "$OUTER_DIR/watch-board.sh" && chmod +x "$OUTER_DIR/watch-board.sh"
    ok "已安装 watch-board.sh → $OUTER_DIR/"
  fi
else
  say "  [--] 未找到 olp-watch-board.sh(curl 单文件运行时不装;从仓库运行 scripts/olp-init.sh 会安装)"
fi

say ""
say "== 下一步(按序) =="
say "  1. 启动内环(标准命令;--solo 是单人盒子安全门):"
say "       octoscode --stdio-command 'octos serve --stdio --solo'"
say "     需要跑构建/工具链时,操作者显式追加 --danger-full-access"
say "     (权限档 1-4 是 bwrap 沙箱,~/.cargo 不可见——见 QUICKSTART 0b 节)。"
say "  2. 首次进入 TUI 完成 onboarding(选 provider、贴 key)。"
say "  3. 外环接入:读 docs/OLP_QUICKSTART.md 的『外环最小接入』三步。"
[ "$MISSING" = 1 ] && { say ""; say "  ⚠ 存在缺失依赖(上方 [!!] 行),先补齐再启动。"; exit 2; }
exit 0
