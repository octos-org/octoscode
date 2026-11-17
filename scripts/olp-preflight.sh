#!/usr/bin/env bash
# OLP 脚本共用的工具预检(由其他 olp-*.sh source,不单独执行)。
#
#   . "$(dirname "${BASH_SOURCE[0]}")/olp-preflight.sh"
#   olp_preflight <脚本名> <能力>... || exit 2
#
# 能力名:
#   flock        flock(1)
#   realpath     realpath <已存在路径>
#   realpath-m   GNU realpath -m(路径可以不存在)
#   stat-c       GNU stat -c
#   sha256sum    sha256sum
#   python3      python3(UTF-8 模式可用)
#
# 全部可用时不改动环境。有缺失且在 macOS 上时,按缺失项补 PATH 后重新
# 探测:
#   - coreutils 类(realpath-m / stat-c / sha256sum / realpath):把 Homebrew
#     coreutils 的 gnubin 目录放到 PATH 最前(与 scripts/verify.sh 一致);
#   - flock / python3:把所在的 Homebrew bin 目录追加到 PATH 末尾——只补
#     缺失的工具,不改变其他命令的解析顺序。非交互 ssh 的 PATH 不含
#     Homebrew 目录,这一步让远程调用不依赖调用方的 shell 配置。
# Linux 等其他平台不做发现,PATH 原样使用。不安装任何东西。
# OLP_PREFLIGHT_DISCOVER=0 关闭发现(只会让预检更严格)。
#
# 仍有缺失时向 stderr 列出缺失项并返回 2。调用方必须在任何写入(建目录、
# 开锁文件、写板)之前调用。

# 输出缺失的能力名(空格分隔);全部可用时无输出。
olp_preflight_missing() {
    local cap missing=""
    for cap in "$@"; do
        case "$cap" in
            flock) command -v flock >/dev/null 2>&1 ;;
            realpath) command -v realpath >/dev/null 2>&1 && realpath / >/dev/null 2>&1 ;;
            realpath-m) command -v realpath >/dev/null 2>&1 && realpath -m /dev/null >/dev/null 2>&1 ;;
            stat-c) command -v stat >/dev/null 2>&1 && stat -c '%s' /dev/null >/dev/null 2>&1 ;;
            sha256sum) command -v sha256sum >/dev/null 2>&1 && sha256sum /dev/null >/dev/null 2>&1 ;;
            python3) python3 -X utf8 -c 'import sys' >/dev/null 2>&1 ;;
            *) false ;;
        esac || missing="$missing $cap"
    done
    printf '%s' "${missing# }"
}

olp_preflight() {
    local who=$1
    shift
    local missing cap dir brew_prefix="" added_gnubin=0
    missing=$(olp_preflight_missing "$@")
    [ -z "$missing" ] && return 0

    if [ "$(uname -s)" = "Darwin" ] && [ "${OLP_PREFLIGHT_DISCOVER:-1}" != "0" ]; then
        if command -v brew >/dev/null 2>&1; then
            brew_prefix=$(brew --prefix 2>/dev/null || true)
        fi
        for cap in $missing; do
            case "$cap" in
                flock | python3)
                    for dir in ${brew_prefix:+"$brew_prefix/bin"} /opt/homebrew/bin /usr/local/bin; do
                        if [ -x "$dir/$cap" ]; then
                            PATH="$PATH:$dir"
                            break
                        fi
                    done
                    ;;
                *)
                    [ "$added_gnubin" -eq 1 ] && continue
                    for dir in ${brew_prefix:+"$brew_prefix/opt/coreutils/libexec/gnubin"} \
                        /opt/homebrew/opt/coreutils/libexec/gnubin \
                        /usr/local/opt/coreutils/libexec/gnubin; do
                        if [ -d "$dir" ]; then
                            PATH="$dir:$PATH"
                            added_gnubin=1
                            break
                        fi
                    done
                    ;;
            esac
        done
        export PATH
        # 补完 PATH 之后再探测一次:此时才判定是否可用。
        missing=$(olp_preflight_missing "$@")
        [ -z "$missing" ] && return 0
    fi

    echo "error: $who: 所需工具不可用: $missing" >&2
    echo "  macOS: brew install coreutils flock    Linux: 安装 coreutils / util-linux / python3" >&2
    return 2
}
