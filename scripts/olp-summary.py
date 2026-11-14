#!/usr/bin/env python3
"""OLP 进展汇总:大外环交给项目全景台的一份项目状态。

用法(在 hub 上):
  scripts/olp-summary.py --board <板> [--repo <hub 仓库>] [--name <项目名>] [--json]

全景台不在 OctoLoop 里——用哪个工具、放在哪里,由用户指定。大外环的循环是:
从全景台读项目目标 → 规划成卡、派给单元 → 用本命令汇总集群的进展 → 回报给
全景台 → 再规划,直到目标完成。本命令只负责"汇总"这一步:把散在黑板、单元
登记、本地仓库里的状态收成一份,文本给人看,`--json` 给程序读。它只读,不写
任何文件,不联网。

汇总的内容
  cards      每张卡:编号、标题、派给哪台单元、实现方与复验方、处在哪一区
             (收件箱 / 进行中 / 搁置 / 已了结)及原因。
  counts     各区的卡数。
  units      每台登记的单元:状态、心跳间隔、在跑的内环、启动权限。
  local_prs  给了 --repo 时:每个本地 PR 的状态(待审核 / 已退回 / 已收单 /
             已提交 GitHub)。
  alerts     还没了结的卡上的负哨行(最近 20 条)。
  questions  还在等回答的内环提问。

退出码:0 成功;2 用法错误(黑板不存在等)。数据源之一读不到时,对应一项
记为 `{"unavailable": "<原因>"}`,其余照常输出。
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ALERT = re.compile(r"^>?\s*单元\(([^)]+)\)·负哨\(#?(\d+)\):\s*(.*)$")


def run_json(args):
    """Run one of the sibling tools with --json; (value, error)."""
    try:
        r = subprocess.run(
            [sys.executable, "-B", *args], capture_output=True, text=True, timeout=60
        )
    except (OSError, subprocess.SubprocessError) as e:
        return None, str(e)
    if r.returncode != 0:
        return None, (r.stderr.strip() or f"退出码 {r.returncode}").splitlines()[-1][:200]
    try:
        return json.loads(r.stdout), ""
    except ValueError:
        return None, "输出不是 JSON"


def board_alerts(board, open_numbers):
    out = []
    with open(board, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            m = ALERT.match(line.strip())
            if m and int(m.group(2)) in open_numbers:
                out.append({"unit": m.group(1), "card": int(m.group(2)), "alert": m.group(3)[:160]})
    return out[-20:]


def collect(args):
    board = os.path.abspath(args.board)
    inbox, err = run_json([os.path.join(HERE, "olp-inbox.py"), board, "--json", "--all"])
    summary = {
        "project": args.name or os.path.basename(os.path.dirname(os.path.dirname(board))) or board,
        "board": board,
        "generated_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }
    if inbox is None:
        summary["cards"] = {"unavailable": err}
        cards, open_numbers = [], set()
    else:
        cards = [
            {k: c.get(k) for k in ("number", "title", "unit", "impl", "review", "partition", "reason")}
            for c in inbox.get("items", [])
        ]
        summary["cards"] = cards
        summary["counts"] = inbox.get("counts", {})
        summary["questions"] = inbox.get("questions", [])
        open_numbers = {c["number"] for c in cards if c["partition"] != "settled"}

    units, err = run_json([os.path.join(HERE, "olp-unit.py"), "status", "--json"])
    if units is None:
        summary["units"] = {"unavailable": err}
    else:
        summary["units"] = [
            {
                "unit": u.get("unit"),
                "state": u.get("state"),
                "heartbeat_age_seconds": u.get("heartbeat_age_seconds"),
                "agents": u.get("agents", []),
                "permission": u.get("permission"),
                "reasons": u.get("reasons", []),
            }
            for u in units.get("units", [])
        ]

    if args.repo:
        prs, err = run_json(
            [os.path.join(HERE, "olp-local-pr.py"), "status", "--repo", args.repo, "--board", board, "--json"]
        )
        summary["local_prs"] = prs if prs is not None else {"unavailable": err}

    summary["alerts"] = board_alerts(board, open_numbers)
    return summary


PARTITION_ZH = {"inbox": "收件箱", "working": "进行中", "snoozed": "搁置", "settled": "已了结"}


def render(s):
    print(f"项目 {s['project']} · {s['generated_at']}")
    counts = s.get("counts") or {}
    if counts:
        print("卡:" + "  ".join(f"{PARTITION_ZH.get(k, k)} {v}" for k, v in counts.items()))
    cards = s.get("cards")
    if isinstance(cards, dict):
        print(f"卡:读不到({cards['unavailable']})")
    else:
        for c in cards:
            if c["partition"] == "settled":
                continue
            roles = ""
            if c.get("impl") or c.get("review"):
                roles = f" 实现 {c.get('impl') or '-'} / 复验 {'、'.join(c.get('review') or []) or '-'}"
            unit = f" [{c['unit']}]" if c.get("unit") else ""
            print(f"  #{c['number']}{unit}{roles} {c['title']} — {PARTITION_ZH.get(c['partition'], c['partition'])}:{c['reason']}")
    units = s.get("units")
    if isinstance(units, dict):
        print(f"单元:读不到({units['unavailable']})")
    elif units:
        print("单元:")
        for u in units:
            age = "-" if u["heartbeat_age_seconds"] is None else f"{u['heartbeat_age_seconds']}s"
            kinds = ",".join(sorted({a.get("kind", "?") for a in u["agents"]})) or "-"
            why = ";".join(u["reasons"])
            state = {"healthy": "正常", "degraded": "降级", "stale": "失联", "offline": "离线"}.get(u["state"], u["state"])
            print(f"  {u['unit']} {state} 心跳 {age} 内环 {kinds} 权限 {u['permission']}" + (f" — {why}" if why else ""))
    prs = s.get("local_prs")
    if isinstance(prs, dict) and "unavailable" in prs:
        print(f"本地 PR:读不到({prs['unavailable']})")
    elif isinstance(prs, dict):
        rows = prs.get("items") or []
        if rows:
            print("本地 PR:")
            for p in rows:
                label = p.get("branch") or f"task/{p.get('number', '?')}"
                print(f"  {label} {p.get('state', '?')}")
    for q in s.get("questions") or []:
        print(f"提问 [{q.get('unit')}] {q.get('question')}")
    for a in s.get("alerts") or []:
        print(f"负哨 #{a['card']} [{a['unit']}] {a['alert']}")


def main():
    ap = argparse.ArgumentParser(prog="olp-summary.py", description="OLP 进展汇总(交给项目全景台)")
    ap.add_argument("--board", required=True)
    ap.add_argument("--repo")
    ap.add_argument("--name")
    ap.add_argument("--json", action="store_true")
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    if not os.path.isfile(args.board):
        print(f"olp-summary: 黑板不存在: {args.board}", file=sys.stderr)
        return 2
    s = collect(args)
    if args.json:
        json.dump(s, sys.stdout, ensure_ascii=False, indent=2)
        sys.stdout.write("\n")
    else:
        render(s)
    return 0


if __name__ == "__main__":
    sys.exit(main())
