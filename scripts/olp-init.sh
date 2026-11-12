#!/usr/bin/env bash
# olp-init.sh — 在当前项目目录一键铺设 OLP(Outer-Loop Protocol)双环脚手架。
#
# 做的事(全部幂等,存在即跳过,绝不覆盖已有内容):
#   1. 依赖体检:octoscode / octos / git / API key 环境,缺什么打印怎么装;
#   2. 生成 .octos/loop.md(内环维护循环)与 .octos/OUTER_LOOP_REVIEW.md
#      (外环审查黑板,含 v1 ACK 定式说明);
#   3. 黑板加入 .gitignore(分支无关,避免跨分支裂脑);
#   4. 铺设 AGENTS.md 上岗卡(自包含,外来 agent 读它即可上岗;
#      OLP_INIT_LANG=zh 取中文版,缺省英文);
#   5. 安装旧 shell 工具,并在可用时安装四个结构化 Python 工具;
#   6. 仅在 OLP_BOARD_MODE=structured 且文件为新建时生成结构化模板与独立锁;
#   7. 打印标准启动命令与后续检查清单。
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
INIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || true)"
STRUCTURED_TOOLS="olp-board-append.py olp-board-event.py olp-board-inbox.py olp-board-sentinel.py"

BOARD_MODE="${OLP_BOARD_MODE:-legacy}"
case "$BOARD_MODE" in
  legacy|structured) ;;
  *)
    say "  [!!] OLP_BOARD_MODE must be legacy or structured"
    exit 2
    ;;
esac

STRUCTURED_ENABLED=0
if [ "$BOARD_MODE" = structured ]; then
  STRUCTURED_ENABLED=1
  if ! command -v python3 >/dev/null 2>&1; then
    say "  [--] structured board tools unavailable: python3 was not found; continuing with the legacy scaffold"
    STRUCTURED_ENABLED=0
  fi
  for tool in $STRUCTURED_TOOLS; do
    if [ ! -f "$INIT_DIR/$tool" ]; then
      say "  [--] structured board tools unavailable: $tool was not found beside olp-init.sh; continuing with the legacy scaffold"
      STRUCTURED_ENABLED=0
    fi
  done
  if [ "$STRUCTURED_ENABLED" = 0 ]; then
    BOARD_MODE=legacy
  fi
fi

STRUCTURED_MIGRATION_ONLY=0
if [ "$STRUCTURED_ENABLED" = 1 ] \
  && { [ -e .octos/loop.md ] || [ -e .octos/OUTER_LOOP_REVIEW.md ]; }; then
  STRUCTURED_MIGRATION_ONLY=1
fi

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
  if [ "$STRUCTURED_ENABLED" = 1 ]; then
    say "  [--] Structured migration: existing .octos/loop.md was not overwritten; update it manually from docs/OLP_STRUCTURED_BOARD.md"
  fi
elif [ "$STRUCTURED_MIGRATION_ONLY" = 1 ]; then
  say "  [--] Structured migration: .octos/loop.md was not generated beside existing OLP files; migrate the project manually"
elif [ "$STRUCTURED_ENABLED" = 1 ]; then
  cat > .octos/loop.md <<'LOOP'
# Structured maintenance loop (opt-in olp-board/v1)

Each maintenance turn must complete these steps in order:

1. Query the canonical board with
   `python3 -B "$HOME/.octos/outer/olp-board-inbox.py" --board .octos/OUTER_LOOP_REVIEW.md --for runtime --actor runtime`.
2. For the earliest `unreceived` item in ledger event order, record consumption before work:
   `python3 -B "$HOME/.octos/outer/olp-board-event.py" receive --board .octos/OUTER_LOOP_REVIEW.md --actor runtime --item <event-id>`.
3. Execute the item, run the full tests plus format and lint for code changes,
   then make one atomic commit that stages only files changed for this item.
4. Write the terminal result through `olp-board-event.py ack` with the original
   item id, an honest R2 value, and an explanation body file. Never hand-write
   a replacement ACK line.

`received_pending` is reconciliation evidence after a restart, not permission
to execute the item again. Stop on DRIFT or a replay error and ask the outer
loop to reconcile the board. Commit only; never push.
LOOP
  ok "generated .octos/loop.md (structured opt-in)"
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
  if [ "$STRUCTURED_ENABLED" = 1 ]; then
    say "  [--] Structured migration: existing board was not overwritten; reconcile legacy text before the first structured item"
  fi
elif [ "$STRUCTURED_MIGRATION_ONLY" = 1 ]; then
  say "  [--] Structured migration: the board was not generated beside existing OLP files; migrate the project manually"
elif [ "$STRUCTURED_ENABLED" = 1 ]; then
  cat > .octos/OUTER_LOOP_REVIEW.md <<'BOARD'
# Outer-Loop Review Channel

<!-- olp-board/v1 -->
> This board opted in to the experimental `olp-board/v1` extension.
> Record items, receipts, ACKs, reviews and reconciliation only with the
> installed event CLI. The text is for humans; completion is decided by
> ledger replay. Do not hand-write item headings or ACK lines, and keep
> examples inside closed code fences.

---
BOARD
  ok "generated .octos/OUTER_LOOP_REVIEW.md (structured opt-in)"
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

if [ "$STRUCTURED_ENABLED" = 1 ] && [ "$STRUCTURED_MIGRATION_ONLY" = 0 ]; then
  if [ -e .octos/OUTER_LOOP_REVIEW.md.lock ]; then
    ok ".octos/OUTER_LOOP_REVIEW.md.lock already exists; skipped"
  else
    (umask 077 && : > .octos/OUTER_LOOP_REVIEW.md.lock)
    ok "generated independent regular lock .octos/OUTER_LOOP_REVIEW.md.lock"
  fi
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
  # The structured lock is a per-checkout runtime file like the board: a
  # tracked copy would be rewritten on checkout and change its inode.
  if [ "$STRUCTURED_ENABLED" = 1 ] && [ "$STRUCTURED_MIGRATION_ONLY" = 0 ] \
    && ! git check-ignore -q .octos/OUTER_LOOP_REVIEW.md.lock 2>/dev/null; then
    if [ -s .gitignore ] && [ "$(tail -c 1 .gitignore | od -An -tuC | tr -d ' ')" != "10" ]; then
      printf '\n' >> .gitignore
    fi
    printf '.octos/OUTER_LOOP_REVIEW.md.lock\n' >> .gitignore
    ok "structured board lock added to .gitignore"
  fi
fi

say ""
say "== agent 上岗卡(AGENTS.md) =="
# AGENTS.md 是 OLP 的常驻约束信道(R6):octos prompt_layer 每个 session 自动
# 注入,外来 agent(Claude Code / Codex / 任意 CLI agent)读它即可自学双环。
# 取卡三路:仓库内复制 → curl 拉取 → 如实报缺(绝不写半张卡)。
case "${OLP_INIT_LANG:-en}" in
  zh|zh-CN|zh_CN) CARD_NAME="OCTOLOOP_AGENTS.zh-CN.md" ;;
  *)              CARD_NAME="OCTOLOOP_AGENTS.md" ;;
esac
CARD_URL="https://raw.githubusercontent.com/octos-org/octoscode/main/docs/$CARD_NAME"
SELF_DIR="$INIT_DIR"
CARD_SRC=""
# 认仓库布局才复制:同目录须有 olp-init.sh 自身,避免 curl|bash 时 dirname 落在
# 无关 cwd 上误取别的 docs/。
if [ -n "$SELF_DIR" ] && [ -f "$SELF_DIR/olp-init.sh" ] && [ -f "$SELF_DIR/../docs/$CARD_NAME" ]; then
  CARD_SRC="$SELF_DIR/../docs/$CARD_NAME"
fi
if [ -f AGENTS.md ]; then
  ok "AGENTS.md 已存在,跳过(如需上岗卡另存: curl -fsSL $CARD_URL -o OCTOLOOP_AGENTS.md)"
elif [ -n "$CARD_SRC" ]; then
  cp "$CARD_SRC" AGENTS.md && ok "生成 AGENTS.md($CARD_NAME)"
elif command -v curl >/dev/null 2>&1 && curl -fsSL "$CARD_URL" -o AGENTS.md 2>/dev/null && [ -s AGENTS.md ]; then
  ok "生成 AGENTS.md(自 $CARD_URL 拉取)"
else
  rm -f AGENTS.md
  say "  [--] AGENTS.md 未铺设(离线或拉取失败)——手动: curl -fsSL $CARD_URL -o AGENTS.md"
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

# 原子追加助手:AGENTS.md 上岗卡指名要它(黑板唯一正道写入),外来项目
# 没有本仓库 scripts/,所以随哨兵一并装到 ~/.octos/outer/。
APPEND_SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/olp-board-append.sh"
if [ -f "$APPEND_SRC" ]; then
  mkdir -p "$OUTER_DIR"
  if [ -f "$OUTER_DIR/board-append.sh" ]; then
    ok "board-append.sh 已存在,跳过(如需更新: cp $APPEND_SRC $OUTER_DIR/board-append.sh)"
  else
    cp "$APPEND_SRC" "$OUTER_DIR/board-append.sh" && chmod +x "$OUTER_DIR/board-append.sh"
    ok "已安装 board-append.sh → $OUTER_DIR/"
  fi
else
  say "  [--] 未找到 olp-board-append.sh(curl 单文件运行时不装;从仓库运行 scripts/olp-init.sh 会安装)"
fi

say ""
say "== Structured board tools (optional) =="
if command -v python3 >/dev/null 2>&1; then
  STRUCTURED_READY=1
  for tool in $STRUCTURED_TOOLS; do
    src="$INIT_DIR/$tool"
    dst="$OUTER_DIR/$tool"
    if [ ! -f "$src" ]; then
      say "  [--] $tool was not found beside olp-init.sh; structured board tools unavailable"
      STRUCTURED_READY=0
    elif [ -e "$dst" ]; then
      ok "$tool already exists; skipped without overwriting the local copy"
    else
      cp "$src" "$dst" && chmod +x "$dst"
      ok "installed $tool → $OUTER_DIR/"
    fi
  done
  if [ "$STRUCTURED_READY" = 1 ]; then
    ok "structured board tools ready (Python 3.9+)"
  fi
else
  say "  [--] structured board tools unavailable: python3 was not found; legacy shell tools remain available"
fi

say ""
say "== 下一步(按序) =="
say "  1. 启动内环(标准命令;--solo 是单人盒子安全门):"
say "       octoscode --stdio-command 'octos serve --stdio --solo'"
say "     需要跑构建/工具链时,操作者显式追加 --danger-full-access"
say "     (权限档 1-4 是 bwrap 沙箱,~/.cargo 不可见——见 QUICKSTART 0b 节)。"
say "  2. 首次进入 TUI 完成 onboarding(选 provider、贴 key)。"
say "  3. 外环接入:把本项目的 AGENTS.md 交给外环 agent(自包含上岗卡:"
say "     身份选择、ACK 定式、派单/唤醒/观测/复验、红线全在卡内)。"
if [ "$STRUCTURED_ENABLED" = 1 ]; then
  say "  4. Structured opt-in is active for newly generated files; read docs/OLP_STRUCTURED_BOARD.md before migration."
else
  say "  4. Legacy board mode remains active. To opt in for a new project, run with OLP_BOARD_MODE=structured."
fi
[ "$MISSING" = 1 ] && { say ""; say "  ⚠ 存在缺失依赖(上方 [!!] 行),先补齐再启动。"; exit 2; }
exit 0
