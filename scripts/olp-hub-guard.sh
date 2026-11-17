#!/usr/bin/env bash
# OLP 进化环的主机守卫(由 olp-evo-harvest.sh / olp-evo-retro.sh source,
# 不单独执行)。
#
#   . "$(dirname "$0")/olp-hub-guard.sh"
#   olp_hub_guard <脚本名> || exit 3
#
# 多机部署时,进化环的采集与 retro 只能在 hub 上运行:它们各自维护一个
# 本机计数器(EVO-NNNN、FLAW-NNN)和本机游标,在别的机器上对着黑板镜像
# 跑一遍,会分配出与 hub 重复的编号,并把同一条信号再记一次。
#
# hub 主机名的来源(按顺序):
#   1. 环境变量 OLP_HUB_HOST;
#   2. 文件 ${OLP_HUB_HOST_FILE:-~/.octos/outer/hub-host} 的第一个非空、
#      非 # 开头的行。
# 两处都没有配置时视为单机部署,守卫直接放行——现有行为不变。
#
# 比较时两边都取第一个标签(去掉域名后缀)并转小写,所以 `Hub-01`、
# `hub-01.local`、`hub-01.lan` 是同一台机器。本机名取 `hostname` 的输出。
# macOS 的主机名会随网络环境变化,hub 上建议固定:
#   sudo scutil --set HostName hub-01
#
# 不是 hub 时向 stderr 说明并返回 3。调用方必须在任何写入(建状态目录、
# 开锁文件、写进化板)之前调用。dry-run 不写任何东西,由调用方决定是否
# 跳过守卫。

# 取第一个标签并转小写。
olp_hub_guard_normalize() {
    local name=$1
    name=${name%%.*}
    printf '%s' "$name" | tr '[:upper:]' '[:lower:]'
}

olp_hub_guard() {
    local who=$1
    local hub="${OLP_HUB_HOST:-}"
    local file="${OLP_HUB_HOST_FILE:-$HOME/.octos/outer/hub-host}"
    local line here
    if [ -z "$hub" ] && [ -f "$file" ]; then
        while IFS= read -r line || [ -n "$line" ]; do
            # 去掉首尾空白(含 CR)。
            line="${line#"${line%%[![:space:]]*}"}"
            line="${line%"${line##*[![:space:]]}"}"
            case "$line" in
                "" | "#"*) continue ;;
            esac
            hub=$line
            break
        done < "$file"
    fi
    [ -z "$hub" ] && return 0

    here=$(hostname 2>/dev/null || uname -n 2>/dev/null || true)
    if [ -n "$here" ] \
        && [ "$(olp_hub_guard_normalize "$hub")" = "$(olp_hub_guard_normalize "$here")" ]; then
        return 0
    fi
    echo "error: $who: 本机(${here:-未知})不是 hub($hub),未写入任何内容。" >&2
    echo "  进化环的采集与 retro 只在 hub 上运行;在 hub 上执行,或用 --dry-run 只读查看。" >&2
    return 3
}
