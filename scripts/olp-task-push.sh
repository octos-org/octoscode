#!/usr/bin/env bash
# OLP 单元侧:本地复验通过后,把任务分支推到 hub 的本地仓库。
#
# 用法(在任务所在的 git 仓库里,由**复验方**执行):
#   scripts/olp-task-push.sh <N> --verified-by <复验署名> [--remote hub] [--force]
#
# 推的是本地分支 task/<N> 的当前分支头,并带上本地复验记录
# (push option `olp-verified=<sha>:<署名>`)。hub 的钩子核对:分支名、
# 这张卡是否派给本单元、记录里的 sha 是否等于推上来的分支头。没有这条
# 记录 hub 会拒收——所以内环自己直接 `git push` 是推不上去的。
#
# --force:返工或 rebase 到新种子之后覆盖自己先前推过的分支。
#
# 退出码:0 推送成功;2 用法错误或本地检查不通过;其余为 git push 的退出码。
set -euo pipefail

die() { echo "olp-task-push: $*" >&2; exit 2; }

N=""
SIGNER=""
REMOTE="hub"
FORCE=0
while [ $# -gt 0 ]; do
    case "$1" in
        --verified-by)
            [ $# -ge 2 ] || die "--verified-by 需要复验署名"
            SIGNER=$2
            shift 2
            ;;
        --remote)
            [ $# -ge 2 ] || die "--remote 需要远程名"
            REMOTE=$2
            shift 2
            ;;
        --force) FORCE=1; shift ;;
        -*) die "未知参数: $1" ;;
        *)
            [ -z "$N" ] || die "多余的参数: $1"
            N=$1
            shift
            ;;
    esac
done
[ -n "$N" ] || die "用法: olp-task-push.sh <N> --verified-by <复验署名> [--remote hub] [--force]"
case "$N" in
    *[!0-9]*) die "任务号必须是数字: $N" ;;
esac
[ -n "$SIGNER" ] || die "缺少 --verified-by:本地复验通过后由复验方推送"
case "$SIGNER" in
    *[!A-Za-z0-9._@-]* | [!A-Za-z0-9]*) die "复验署名只允许 [A-Za-z0-9._@-],且以字母或数字开头" ;;
esac

BRANCH="task/$N"
SHA=$(git rev-parse --verify --quiet "refs/heads/$BRANCH^{commit}") \
    || die "本地没有分支 $BRANCH"
git remote get-url "$REMOTE" >/dev/null 2>&1 || die "没有名为 $REMOTE 的远程"

ARGS=(push -o "olp-verified=$SHA:$SIGNER")
[ "$FORCE" -eq 1 ] && ARGS+=(--force)
exec git "${ARGS[@]}" "$REMOTE" "refs/heads/$BRANCH:refs/heads/$BRANCH"
