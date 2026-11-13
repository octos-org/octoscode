#!/usr/bin/env bash
# OLP 黑板原子追加助手(随仓库发行版)。
#
# 外环写黑板的唯一正道:flock 互斥 + 整条目一次性追加,防多写者交错与
# 撞号。条目正文从 stdin 喂入。
#
# 用法:
#   scripts/olp-board-append.sh [--key <自然键>] <board.md>  <<'EOF'
#   ### {N}. <标题>(<日期>,<署名>)
#   ...正文...
#   EOF
#
# 三种写法:
#   1. 锁内分配编号(推荐):正文首行以 `### {N}` 起头。助手在锁内取板上
#      `### <数字>` 的最大号 +1,替换正文里所有 `{N}`,并把分配到的编号
#      打印到 stdout。多个写入方并发也不会撞号。
#   2. 幂等重发:加 `--key <自然键>`。同一个键已在板上时不再追加,stdout
#      返回原编号(该条目当初未分配编号时无输出),退出码 0。用于断网
#      重发、离线队列回放。键只允许 [A-Za-z0-9._:/@+=-],最长 200。
#   3. 旧写法(兼容):首行不是 `### {N}` 且不带 `--key` 时原样追加,不输出
#      (正文别处的 `{N}` 字样不替换)。调用方自己写编号——并发下会撞号,
#      新代码不要用。
#
# 退出码:0 成功(含幂等命中);2 用法或预检失败(此时板与锁文件均未触碰)。
#
# 本脚本自包含(olp-init.sh 会把它单文件装到 ~/.octos/outer/),不 source
# 其他文件。macOS 的 flock 来自 Homebrew;非交互 ssh 的 PATH 不含
# Homebrew 目录,所以找不到 flock 时会到 /opt/homebrew/bin 与
# /usr/local/bin 再找一次(OLP_PREFLIGHT_DISCOVER=0 可关闭)。
#
# 注:若你是常驻外环且自建了"自写登记"机制(监视器抵扣自触发),请在
# 你自己的包装脚本里做登记——本脚本保持纯追加,保证其他外环的监视器
# 能感知你的写入。
set -euo pipefail
# 键校验与逐字节处理不随调用方 locale 变化。
export LC_ALL=C

die() { echo "olp-board-append: $*" >&2; exit 2; }

KEY=""
HAS_KEY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --key)
            [ $# -ge 2 ] || die "--key 需要一个值"
            KEY=$2
            HAS_KEY=1
            shift 2
            ;;
        --) shift; break ;;
        -*) die "未知参数: $1" ;;
        *) break ;;
    esac
done
BOARD="${1:?用法: olp-board-append.sh [--key <自然键>] <board.md> (正文从 stdin 喂)}"
[ $# -eq 1 ] || die "多余的参数: $2"

if [ "$HAS_KEY" -eq 1 ]; then
    [ -n "$KEY" ] || die "--key 不能为空"
    [ "${#KEY}" -le 200 ] || die "--key 超过 200 字节"
    case "$KEY" in
        *[!A-Za-z0-9._:/@+=-]*) die "--key 只允许 [A-Za-z0-9._:/@+=-]" ;;
    esac
fi

# --- 预检:任何写入之前 --------------------------------------------------
if ! command -v flock >/dev/null 2>&1; then
    if [ "$(uname -s)" = "Darwin" ] && [ "${OLP_PREFLIGHT_DISCOVER:-1}" != "0" ]; then
        for dir in /opt/homebrew/bin /usr/local/bin; do
            if [ -x "$dir/flock" ]; then
                # 追加在末尾:只补缺失的工具,不改变其他命令的解析顺序。
                PATH="$PATH:$dir"
                export PATH
                break
            fi
        done
    fi
    if ! command -v flock >/dev/null 2>&1; then
        echo "olp-board-append: 找不到 flock,未写入任何内容。" >&2
        echo "  macOS: brew install flock    Linux: 安装 util-linux" >&2
        exit 2
    fi
fi

TMP=$(mktemp)
OUT=$(mktemp)
trap 'rm -f "$TMP" "$OUT"' EXIT
cat > "$TMP"

# 只有首行以 `### {N}` 起头才分配编号。正文别处出现的 `{N}` 字样(例如
# 采集哨转抄的原文)不算占位符,按旧写法原样追加——调用方不该因为正文
# 里碰巧有这三个字符而写板失败。
ALLOC=0
case "$(head -n 1 "$TMP")" in
    '### {N}'*) ALLOC=1 ;;
esac
if [ ! -s "$TMP" ]; then
    [ "$HAS_KEY" -eq 0 ] || die "正文为空"
    exit 0
fi

LOCK="${BOARD}.lock"
exec 9>"$LOCK"
flock -x 9

# --- 以下均在锁内 --------------------------------------------------------
if [ "$HAS_KEY" -eq 1 ] && [ -f "$BOARD" ]; then
    HIT=$(awk -v k="$KEY" '
        BEGIN { bare = "<!-- olp-key: " k " -->"; pre = "<!-- olp-key: " k " n=" }
        $0 == bare { print "-"; exit }
        index($0, pre) == 1 {
            n = substr($0, length(pre) + 1)
            sub(/ -->$/, "", n)
            if (n ~ /^[0-9]+$/) { print n; exit }
        }
    ' "$BOARD")
    if [ -n "$HIT" ]; then
        if [ "$HIT" = "-" ]; then
            echo "olp-board-append: 键 $KEY 已在板上,未重复追加" >&2
        else
            echo "olp-board-append: 键 $KEY 已在板上(#$HIT),未重复追加" >&2
            echo "$HIT"
        fi
        exit 0
    fi
fi

N=""
if [ "$ALLOC" -eq 1 ]; then
    MAX=0
    if [ -f "$BOARD" ]; then
        MAX=$(awk '
            /^### [0-9]+/ { n = $2; sub(/[^0-9].*$/, "", n); if (n + 0 > m) m = n + 0 }
            END { print m + 0 }
        ' "$BOARD")
    fi
    N=$((MAX + 1))
    awk -v n="$N" '
        {
            out = ""
            rest = $0
            while ((i = index(rest, "{N}")) > 0) {
                out = out substr(rest, 1, i - 1) n
                rest = substr(rest, i + 3)
            }
            print out rest
        }
    ' "$TMP" > "$OUT"
else
    cat "$TMP" > "$OUT"
fi

# 条目以换行收尾,键标记才能独占一行,下一条的标题也不会粘在后面。
if [ -n "$(tail -c 1 "$OUT")" ]; then
    echo >> "$OUT"
fi
if [ "$HAS_KEY" -eq 1 ]; then
    if [ -n "$N" ]; then
        echo "<!-- olp-key: $KEY n=$N -->" >> "$OUT"
    else
        echo "<!-- olp-key: $KEY -->" >> "$OUT"
    fi
fi
# 板的最后一行缺换行时先补上,保证新条目从行首开始。
if [ -s "$BOARD" ] && [ -n "$(tail -c 1 "$BOARD")" ]; then
    echo >> "$BOARD"
fi
# 分配了编号的新条目以标题开头;板的最后一行不是空行时先空一行,标题不贴着
# 上一条的末行(例如一行 `ACK:`)。旧写法(不带 `{N}`)仍然逐字追加。
if [ -n "$N" ] && [ -s "$BOARD" ] && [ -n "$(tail -n 1 "$BOARD")" ]; then
    echo >> "$BOARD"
fi
cat "$OUT" >> "$BOARD"
if [ -n "$N" ]; then
    echo "$N"
fi
