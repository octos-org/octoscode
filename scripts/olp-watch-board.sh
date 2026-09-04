#!/usr/bin/env bash
# OLP 板面侦听哨(随仓库发行版;`scripts/olp-init.sh` 会把它安装到
# ~/.octos/outer/watch-board.sh 供外环运行态调用)。
#
# 唯一合法配方(四次误报/漏报事故后固化,2026-08-30,见 skill 卡"自主性纪律"第 2 条):
#   基线行数裁剪判定域(只看挂哨后新增的行)+ 域内宽松子串匹配(grep -F,不猜格式)。
#   误报免疫:旧内容(任务书自述/引用/历史同号 ACK)永不进入判定域。
#   漏报免疫:任意前缀(### / > / 裸行/列表符)一视同仁。
#
# 已知盲区与对策:外环自己落板的批注若引用了 token(如判词里写"落 ACK(45a done|blocked)"),
# 也会进入判定域触发——两种对策任选其一:
#   (a) 先落板、后挂哨(基线取在批注之后);
#   (b) 用 --skip-signature '<署名>' 排除含该署名的行(署名批注一律带"·外环(<名>)"或"外环(<名>)")。
#
# 用法:
#   olp-watch-board.sh <板路径> <子串token> [--interval 秒(默认 20)] [--skip-signature <署名>]...
#   退出码:0 = 命中(stdout 打印 BOARD-SIGNAL 与最多 3 行命中);2 = 参数错误;板缺失时等待其出现。
set -euo pipefail

board=""; token=""; interval=20; skips=()
while [ $# -gt 0 ]; do
  case "$1" in
    --interval) interval="${2:?--interval 需要秒数}"; shift 2 ;;
    --skip-signature) skips+=("${2:?--skip-signature 需要署名}"); shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    --*) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    *) if [ -z "$board" ]; then board="$1"; elif [ -z "$token" ]; then token="$1"; else interval="$1"; fi; shift ;;
  esac
done
[ -n "$board" ] && [ -n "$token" ] || { printf '用法: olp-watch-board.sh <板路径> <子串token> [--interval N] [--skip-signature 署名]...\n' >&2; exit 2; }
case "$interval" in ''|*[!0-9]*) printf 'interval 必须是整数秒: %s\n' "$interval" >&2; exit 2 ;; esac

# 判定域过滤:先裁掉带排除署名的行,再做子串匹配。
filter_skips() {
  if [ ${#skips[@]} -eq 0 ]; then cat; return; fi
  local args=()
  for s in "${skips[@]}"; do args+=(-e "$s"); done
  grep -vF "${args[@]}" || true
}

until [ -f "$board" ]; do sleep "$interval"; done
base=$(wc -l < "$board")
while :; do
  cur=$(wc -l < "$board")
  if [ "$cur" -gt "$base" ]; then
    hits=$(tail -n +"$((base + 1))" "$board" | filter_skips | grep -F -- "$token" || true)
    if [ -n "$hits" ]; then
      echo "BOARD-SIGNAL: $token"
      printf '%s\n' "$hits" | head -3
      exit 0
    fi
  elif [ "$cur" -lt "$base" ]; then
    # 板被截断/重写:基线失效,重新取基线(不回溯旧内容)。
    base=$cur
  fi
  sleep "$interval"
done
