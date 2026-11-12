#!/usr/bin/env bash
# For verified byte receipts and olp-board/v1 events, use the adjacent Python tools.
# OLP 黑板原子追加助手(随仓库发行版)。
#
# 外环写黑板的唯一正道:flock 互斥 + 整条目一次性追加,防多写者交错与
# 撞号。条目正文从 stdin 喂入。
#
# 用法:
#   scripts/olp-board-append.sh <board.md>  <<'EOF'
#   ### <编号>. <标题>(<日期>,<署名>)
#   ...正文...
#   EOF
#
# 注:若你是常驻外环且自建了"自写登记"机制(监视器抵扣自触发),请在
# 你自己的包装脚本里做登记——本脚本保持纯追加,保证其他外环的监视器
# 能感知你的写入。
set -euo pipefail
BOARD="${1:?用法: olp-board-append.sh <board.md> (正文从 stdin 喂)}"
# find and grep below would read a relative name starting with `-`, `!` or `(`
# as an option or an expression and silently skip every check; keep it a path.
case $BOARD in /*) ;; *) BOARD="./$BOARD" ;; esac
LOCK="${BOARD}.lock"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT
cat > "$TMP"
OPTED_IN='^(> OLP-EVENT |<!-- olp-board/v1 -->$)'
# The Python tools lock the real board; through a symlink this helper would lock
# <link>.lock instead and split the lock domain, so refuse before locking.
if [ -L "$BOARD" ] && LC_ALL=C grep -qE "$OPTED_IN" "$BOARD" 2>/dev/null; then
    echo "error: $BOARD is a symlink to an olp-board/v1 board; use the real path so every tool shares one lock" >&2
    exit 2
fi
# Each name of a hard-linked board would lock its own <name>.lock as well. The
# check prints a fixed mark, never the name, which command substitution could
# strip (a name may be just a newline).
has_extra_links() { [ "$(find "$1" -prune -links +1 -exec printf x \; 2>/dev/null)" = x ]; }
refuse_extra_links() {
    echo "error: $BOARD is an olp-board/v1 board with more than one hard link; keep exactly one link so every tool shares one lock" >&2
    exit 2
}
if has_extra_links "$BOARD" && LC_ALL=C grep -qE "$OPTED_IN" "$BOARD" 2>/dev/null; then
    refuse_extra_links
fi
exec 9>"$LOCK"
flock -x 9
# A board that opted in to olp-board/v1 reserves `> OLP-EVENT ` lines and
# standalone `ts=` lines for the event CLI; a hand-appended copy would only be
# quarantined as malformed DRIFT, or could re-pair a damaged event. A ts= line
# ends with an optional CR only, exactly as olp-board-append.py judges it.
TS_LINE='ts=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z'$'\r''?$'
if LC_ALL=C grep -qE "$OPTED_IN" "$BOARD" 2>/dev/null; then
    # A second name may have appeared while the lock was awaited.
    has_extra_links "$BOARD" && refuse_extra_links
    if LC_ALL=C grep -qE "^(> OLP-EVENT |${TS_LINE})" "$TMP"; then
        echo "error: $BOARD opted in to olp-board/v1; record events with olp-board-event.py" >&2
        exit 2
    fi
    # Like olp-board-append.py, keep every line whole: never leave a partial
    # line and never continue one, or two appends could splice an event line.
    if [ -s "$TMP" ] && [ "$(tail -c 1 "$TMP" | od -An -tuC | tr -d ' ')" != 10 ]; then
        echo "error: $BOARD opted in to olp-board/v1; the entry must end with a newline" >&2
        exit 2
    fi
    if [ -s "$BOARD" ] && [ "$(tail -c 1 "$BOARD" | od -An -tuC | tr -d ' ')" != 10 ]; then
        echo "error: $BOARD opted in to olp-board/v1 and ends with a partial line; void it with olp-board-event.py first" >&2
        exit 2
    fi
fi
cat "$TMP" >> "$BOARD"
