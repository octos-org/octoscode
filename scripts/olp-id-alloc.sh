#!/usr/bin/env bash
# OLP 编号分配助手:在 flock 下按自然键分配顺序编号。
#
# 给黑板条目号以外的编号用(例如 API-SS-00123、APP-00042、W00037):
# 同一个自然键再次请求时返回原编号,不重复分配。登记文件是唯一的事实源。
#
# 用法:
#   scripts/olp-id-alloc.sh <登记文件.tsv> <前缀> <位数> <自然键>
#
#   scripts/olp-id-alloc.sh ids.tsv API-SS- 5 'ohos.ability.startAbility'
#   → API-SS-00001          (再次以同一个键请求,仍是 API-SS-00001)
#
# stdout 只输出编号。登记文件每行四列,制表符分隔:
#   <编号>\t<前缀>\t<自然键>\t<UTC 时间>
# 各前缀独立计数,新编号 = 该前缀已登记的最大序号 + 1。
#
# 退出码:0 成功(含已存在);2 用法或预检失败(此时登记文件与锁文件均未触碰)。
#
# 多机使用时,在登记文件所在的机器上执行本脚本(例如经 ssh);不要把登记
# 文件放在 NFS/SMB 上,网络文件系统上的 flock 不可靠。
set -euo pipefail
export LC_ALL=C

die() { echo "olp-id-alloc: $*" >&2; exit 2; }

[ $# -eq 4 ] || die "用法: olp-id-alloc.sh <登记文件.tsv> <前缀> <位数> <自然键>"
REGISTRY=$1
PREFIX=$2
WIDTH=$3
KEY=$4

case "$PREFIX" in
    "" | [!A-Za-z]* | *[!A-Za-z0-9-]*) die "前缀须以字母开头,只含 [A-Za-z0-9-]" ;;
esac
[ "${#PREFIX}" -le 32 ] || die "前缀超过 32 字节"
case "$WIDTH" in
    "" | *[!0-9]*) die "位数须为 1-12 的整数" ;;
esac
[ "${#WIDTH}" -le 2 ] && [ "$((10#$WIDTH))" -ge 1 ] && [ "$((10#$WIDTH))" -le 12 ] \
    || die "位数须为 1-12 的整数"
WIDTH=$((10#$WIDTH))
[ -n "$KEY" ] || die "自然键不能为空"
[ "${#KEY}" -le 500 ] || die "自然键超过 500 字节"
case "$KEY" in
    *[[:cntrl:]]*) die "自然键不能含制表符、换行等控制字符" ;;
esac

# --- 预检:任何写入之前 --------------------------------------------------
. "$(dirname "${BASH_SOURCE[0]}")/olp-preflight.sh"
olp_preflight olp-id-alloc flock || exit 2

LOCK="${REGISTRY}.lock"
exec 9>"$LOCK"
flock -x 9

# --- 以下均在锁内 --------------------------------------------------------
# 键经环境变量传入:awk -v 会解释反斜杠转义,自然键必须逐字比较。
FOUND=""
if [ -f "$REGISTRY" ]; then
    FOUND=$(OLP_ID_PREFIX="$PREFIX" OLP_ID_KEY="$KEY" awk -F '\t' '
        BEGIN { p = ENVIRON["OLP_ID_PREFIX"]; k = ENVIRON["OLP_ID_KEY"]; max = 0 }
        NF >= 3 && $2 == p {
            if ($3 == k) { print "HIT\t" $1; hit = 1; exit }
            s = substr($1, length(p) + 1)
            if (index($1, p) == 1 && s ~ /^[0-9]+$/ && s + 0 > max) max = s + 0
        }
        END { if (!hit) print "MAX\t" max }
    ' "$REGISTRY")
fi
case "$FOUND" in
    HIT*)
        printf '%s\n' "${FOUND#HIT	}"
        exit 0
        ;;
    MAX*) MAX=${FOUND#MAX	} ;;
    *) MAX=0 ;;
esac

ID=$(printf '%s%0*d' "$PREFIX" "$WIDTH" "$((MAX + 1))")
if [ -s "$REGISTRY" ] && [ -n "$(tail -c 1 "$REGISTRY")" ]; then
    echo >> "$REGISTRY"
fi
printf '%s\t%s\t%s\t%s\n' "$ID" "$PREFIX" "$KEY" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$REGISTRY"
printf '%s\n' "$ID"
