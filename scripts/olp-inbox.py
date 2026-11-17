#!/usr/bin/env python3
"""OLP 黑板收件箱:从黑板算出每张卡现在等谁。

用法:
  scripts/olp-inbox.py <board.md> [--json] [--unit <单元号>] [--all] [--today YYYY-MM-DD]
  scripts/olp-inbox.py <board.md> --units [--json] [--unit <单元号>]

黑板是按时间追加的流水账。条目多了以后,外环要从头扫一遍才知道"现在
哪些事在等我"。本工具只读黑板,把每个编号条目归到四个分区之一:

  收件箱   等外环决定:内环已 ACK、单元已推送、搁置到期
  进行中   已派出,还没有 ACK——这是内环的收件箱(执行编号最小的那条)
  搁置     外环明确搁置到某一天的条目
  已了结   已采认、已打回、被改判作废、已归档

状态不存进黑板,每次从黑板现有的行算出来;黑板仍然只追加。默认输出
前三个分区,--all 连已了结一起输出。

判定依据(都是行首定式,代码围栏里的不算)
  条目内:
    ACK(done) / ACK(wontdo) / ACK(blocked)   以行号最大的一条为准
    > 内环(<署名>)·ACK(done): …              内环的署名写法,同样算 ACK;带
    > 内环(<署名>)·#N ACK(done): …           `#N` 时归到 #N,不看它落在哪里
    unit: <单元号>                            以条目内最后一条为准
    <!-- olp-archived: … -->                  归档工具留下的桩
  全板任意位置(按 #N 归属,不看它落在哪个条目下面):
    > 外环(<署名>)·采认(#N): …               通过
    > 外环(<署名>)·打回(#N): …               不通过;整改按 R1/R5 另立新条目
    > 外环(<署名>)·改判(作废 #N): …          作废
    > 外环(<署名>)·搁置(#N) 至 YYYY-MM-DD: … 到那一天之前不进收件箱
    > 单元(<单元号>)·本地复验通过(#N): …     单元已推送到 hub 的本地仓库

  对同一个条目,以**最后发生**(行号最大)的那件事为准:
    采认 / 打回                 → 已了结
    ACK(done)                   → 收件箱:待复验
    ACK(wontdo)                 → 收件箱:待回应(接受或升级)
    ACK(blocked)                → 收件箱:待解阻塞
    单元已推送                  → 收件箱:待审核
    以上都没有                  → 进行中
  改判作废与归档桩不看先后,直接是已了结。
  搁置行只有落在最后一件事**之后**才生效;之后内环又有新的 ACK 或推送,
  搁置自动失效,条目回到收件箱。

待回答的提问
  内环的 ask_outer 在板上留一行 `MCP(ask_outer) ask: id=… question=…`(单元
  转发来的带 `> 单元(<单元号>)·` 署名)。还没有对应的 answer 或 timeout 行
  的提问列在最前面——内环只等 50 秒。用 scripts/olp-answer.py 回答。

占用视图(--units)
  多机部署时一台单元同一时间只做一张卡。--units 按单元列出还占着它的
  卡:派给它(以条目里最后一条 `unit:` 行为准)且还没有了结的条目。
  带 `unit:` 行的卡被打回后仍算占着那台单元(它要返工并重新推送),直到
  采认、改判作废或归档。

也顺带报告黑板的结构问题(warnings):同一个编号出现两次、搁置行的
日期无法解析、判词指向板上不存在的条目。

退出码:0 成功;2 用法错误或板不存在。
"""

import argparse
import datetime
import json
import os
import re
import sys

HEADING = re.compile(r"^### (\d+)[.．]?\s*(.*)$")
UNIT_LINE = re.compile(r"^unit:\s*(\S+)\s*$")
ROLE_LINE = re.compile(r"^(impl|review):\s*(\S.*?)\s*$")
ACK = re.compile(r"^ACK\((done|wontdo|blocked)\)")
SIGNED = re.compile(r"^外环\(([^)]+)\)·(采认|打回|搁置)\(#(\d+)\)(.*)$")
VOID = re.compile(r"^外环\(([^)]+)\)·改判\(作废 #(\d+)\)")
# 实际黑板上内环也会把 ACK 写成署名引用行,可带条目号:
#   > 内环(<署名>)·#4 ACK(done)(日期): …     > 内环(<署名>)·ACK(done): #7(日期): …
INNER_ACK = re.compile(
    r"^内环\([^)]+\)·(?:#(\d+)\s*)?ACK\((done|wontdo|blocked)\)(?:[^:：]*[:：]\s*#(\d+))?"
)
PUSHED = re.compile(r"^单元\(([^)]+)\)·本地复验通过\(#(\d+)\)")
SNOOZE_UNTIL = re.compile(r"^\s*至\s*(\d{4}-\d{2}-\d{2})")
ARCHIVED_MARK = "<!-- olp-archived: "

INBOX, WORKING, SNOOZED, SETTLED = "inbox", "working", "snoozed", "settled"
PARTITION_ZH = {INBOX: "收件箱", WORKING: "进行中", SNOOZED: "搁置", SETTLED: "已了结"}
REASON = {
    "ack_done": "待复验:内环已 ACK(done)",
    "ack_wontdo": "待回应:内环 ACK(wontdo),接受或升级",
    "ack_blocked": "待解阻塞:内环 ACK(blocked)",
    "pushed": "待审核:单元已推送",
}


def strip_quote(line):
    """Drop leading `> `, `**` and whitespace, like the harvest does."""
    s = line.lstrip()
    while True:
        if s.startswith("> "):
            s = s[2:]
        elif s.startswith(">"):
            s = s[1:]
        elif s.startswith("**"):
            s = s[2:]
        else:
            break
        s = s.lstrip()
    return s


def scan(board):
    """One pass over the board. Returns (entries, events, warnings):
    entries: number -> {title, line, unit, archived, acks: [(line, kind)]}
    events:  number -> [(line, kind, detail)] for verdict lines found anywhere
    """
    entries, events, warnings = {}, {}, []
    current = None
    in_fence = False
    with open(board, "r", encoding="utf-8", errors="replace") as f:
        for no, raw in enumerate(f, start=1):
            line = raw.rstrip("\r\n")
            stripped = line.lstrip()
            if stripped.startswith("```") or stripped.startswith("~~~"):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            if line.startswith("### ") or line.startswith("## "):
                m = HEADING.match(line)
                if m:
                    number = int(m.group(1))
                    if number in entries:
                        warnings.append(
                            f"编号 #{number} 出现两次(第 {entries[number]['line']} 行与第 {no} 行),以第一次为准"
                        )
                        current = None
                    else:
                        current = number
                        entries[number] = {
                            "title": m.group(2).strip(),
                            "line": no,
                            "unit": None,
                            "impl": None,
                            "review": [],
                            "archived": False,
                            "acks": [],
                        }
                else:
                    current = None
                continue
            if current is not None:
                entry = entries[current]
                m = ACK.match(line)
                if m:
                    entry["acks"].append((no, "ack_" + m.group(1)))
                    continue
                m = UNIT_LINE.match(line)
                if m:
                    entry["unit"] = m.group(1)
                    continue
                m = ROLE_LINE.match(line)
                if m:
                    if m.group(1) == "impl":
                        entry["impl"] = m.group(2)
                    else:
                        entry["review"] = [r.strip() for r in m.group(2).split(",") if r.strip()]
                    continue
                if line.startswith(ARCHIVED_MARK):
                    entry["archived"] = True
                    continue
            signed = strip_quote(line)
            m = INNER_ACK.match(signed)
            if m:
                ref = m.group(1) or m.group(3)
                target = int(ref) if ref else current
                if target in entries:
                    entries[target]["acks"].append((no, "ack_" + m.group(2)))
                elif target is not None:
                    events.setdefault(target, []).append((no, "dangling", ""))
                continue
            m = SIGNED.match(signed)
            if m:
                kind = {"采认": "accepted", "打回": "returned", "搁置": "snooze"}[m.group(2)]
                events.setdefault(int(m.group(3)), []).append((no, kind, m.group(4)))
                continue
            m = VOID.match(signed)
            if m:
                events.setdefault(int(m.group(2)), []).append((no, "voided", ""))
                continue
            m = PUSHED.match(signed)
            if m:
                events.setdefault(int(m.group(2)), []).append((no, "pushed", m.group(1)))
    for number in sorted(events):
        if number not in entries:
            first = events[number][0][0]
            warnings.append(f"第 {first} 行的判词指向 #{number},但板上没有这个条目")
    return entries, events, warnings


MCP_AUDIT = re.compile(
    r"MCP\(ask_outer\) (ask|answer|timeout): id=([A-Za-z0-9_-]+)(?: question=(.*))?"
)
UNIT_SIGNATURE = re.compile(r"^单元\(([^)]+)\)·")


def pending_questions(board):
    """Inner-loop questions (ask_outer) that have reached the board and have
    neither been answered nor timed out. A unit forwards its MCP audit lines
    signed as `> 单元(<unit>)·- <time> MCP(ask_outer) ask: id=… question=…`;
    on a single machine the same lines appear unsigned."""
    asked, closed = {}, set()
    in_fence = False
    with open(board, "r", encoding="utf-8", errors="replace") as f:
        for no, raw in enumerate(f, start=1):
            line = raw.rstrip("\r\n")
            stripped = line.lstrip()
            if stripped.startswith("```") or stripped.startswith("~~~"):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            signed = strip_quote(line)
            m = MCP_AUDIT.search(signed)
            if not m:
                continue
            kind, ask_id = m.group(1), m.group(2)
            if kind == "ask":
                unit = UNIT_SIGNATURE.match(signed)
                asked[ask_id] = {
                    "id": ask_id,
                    "unit": unit.group(1) if unit else None,
                    "question": (m.group(3) or "").strip(),
                    "line": no,
                }
            else:
                closed.add(ask_id)
    return [q for ask_id, q in asked.items() if ask_id not in closed]


def classify(number, entry, events, today, warnings):
    """Return (partition, reason, extra) for one entry."""
    evs = events.get(number, [])
    if entry["archived"]:
        return SETTLED, "已归档", {}
    if any(kind == "voided" for _, kind, _ in evs):
        return SETTLED, "已被改判作废", {}

    timeline = list(entry["acks"])
    timeline += [(no, kind) for no, kind, _ in evs if kind in ("accepted", "returned", "pushed")]
    timeline.sort()
    if not timeline:
        last_line, last = 0, None
    else:
        last_line, last = timeline[-1]

    if last == "accepted":
        return SETTLED, "已采认", {}
    if last == "returned":
        if entry["unit"]:
            # 多机部署:打回沿派发路径退回这台单元,它返工后重新推送同一个
            # 任务分支,所以这张卡还占着单元。
            return WORKING, "已打回,等单元返工后重新推送", {}
        return SETTLED, "已打回(整改见后续条目)", {}

    # A snooze counts only when it is the latest thing said about the entry.
    until = None
    for no, kind, detail in evs:
        if kind != "snooze" or no < last_line:
            continue
        m = SNOOZE_UNTIL.match(detail)
        if not m:
            warnings.append(f"第 {no} 行:#{number} 的搁置行缺少可解析的日期(应为 至 YYYY-MM-DD),已忽略")
            continue
        try:
            until = datetime.date.fromisoformat(m.group(1))
        except ValueError:
            warnings.append(f"第 {no} 行:#{number} 的搁置日期 {m.group(1)} 不是合法日期,已忽略")
            continue
    if until is not None:
        if today < until:
            return SNOOZED, f"搁置至 {until.isoformat()}", {"until": until.isoformat()}
        expired = f"(搁置已于 {until.isoformat()} 到期)"
    else:
        expired = ""

    if last is None:
        if until is not None:
            return INBOX, "搁置到期,尚未 ACK" + expired, {}
        return WORKING, "已派出,等内环 ACK", {}
    return INBOX, REASON[last] + expired, {}


def unit_load(board, today):
    """Which cards each unit still holds. Returns (load, warnings), where
    load maps a unit id to its open cards — every entry dispatched to it
    (last `unit:` line) that is not settled — as dicts with number, title,
    partition and reason. A unit with no open card maps to an empty list."""
    entries, events, warnings = scan(board)
    load = {}
    for number in sorted(entries):
        entry = entries[number]
        if not entry["unit"]:
            continue
        cards = load.setdefault(entry["unit"], [])
        partition, reason, _ = classify(number, entry, events, today, warnings)
        if partition != SETTLED:
            cards.append(
                {"number": number, "title": entry["title"], "partition": partition, "reason": reason}
            )
    return load, warnings


def main():
    ap = argparse.ArgumentParser(prog="olp-inbox.py", description="OLP 黑板收件箱")
    ap.add_argument("--units", action="store_true", help="按单元列出还占着它的卡(占用视图)")
    ap.add_argument("board")
    ap.add_argument("--json", action="store_true", help="输出 JSON")
    ap.add_argument("--unit", help="只看派给这个单元的条目")
    ap.add_argument("--all", action="store_true", help="连已了结一起输出")
    ap.add_argument("--today", help="把今天当作这一天(YYYY-MM-DD),用于判断搁置是否到期")
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    if not os.path.isfile(args.board):
        print(f"olp-inbox: 板不存在: {args.board}", file=sys.stderr)
        return 2
    if args.today:
        try:
            today = datetime.date.fromisoformat(args.today)
        except ValueError:
            print(f"olp-inbox: --today 不是合法日期: {args.today}", file=sys.stderr)
            return 2
    else:
        today = datetime.date.today()

    if args.units:
        load, warnings = unit_load(args.board, today)
        units = [
            {"unit": u, "busy": bool(load[u]), "open": load[u]}
            for u in sorted(load)
            if not args.unit or u == args.unit
        ]
        if args.json:
            json.dump(
                {"today": today.isoformat(), "units": units, "warnings": warnings},
                sys.stdout,
                ensure_ascii=False,
                indent=2,
            )
            sys.stdout.write("\n")
            return 0
        if not units:
            print("板上没有派给单元的条目")
        for u in units:
            if not u["open"]:
                print(f"{u['unit']}\t空闲")
                continue
            cards = ";".join(f"#{c['number']} {c['reason']}" for c in u["open"])
            print(f"{u['unit']}\t忙\t{cards}")
        for w in warnings:
            print(f"warning: {w}", file=sys.stderr)
        return 0

    entries, events, warnings = scan(args.board)
    items = []
    for number in sorted(entries):
        entry = entries[number]
        if args.unit and entry["unit"] != args.unit:
            continue
        partition, reason, extra = classify(number, entry, events, today, warnings)
        item = {
            "number": number,
            "title": entry["title"],
            "unit": entry["unit"],
            "impl": entry["impl"],
            "review": entry["review"],
            "partition": partition,
            "reason": reason,
        }
        item.update(extra)
        items.append(item)

    order = [INBOX, WORKING, SNOOZED] + ([SETTLED] if args.all else [])
    counts = {p: sum(1 for i in items if i["partition"] == p) for p in PARTITION_ZH}
    questions = [
        q for q in pending_questions(args.board) if not args.unit or q["unit"] == args.unit
    ]
    if args.json:
        json.dump(
            {
                "today": today.isoformat(),
                "counts": counts,
                "items": [i for i in items if i["partition"] in order],
                "questions": questions,
                "warnings": warnings,
            },
            sys.stdout,
            ensure_ascii=False,
            indent=2,
        )
        sys.stdout.write("\n")
        return 0

    if questions:
        # 内环只等 50 秒:提问排在最前面。
        print(f"== 待回答的提问({len(questions)})")
        for q in questions:
            unit = f" [{q['unit']}]" if q["unit"] else ""
            print(f"id={q['id']}{unit} {q['question']}")
            if q["unit"]:
                print(f"    olp-answer.py answer {q['unit']} {q['id']}  (回答从 stdin 喂)")
    for partition in order:
        rows = [i for i in items if i["partition"] == partition]
        print(f"== {PARTITION_ZH[partition]}({len(rows)})")
        for i in rows:
            unit = f" [{i['unit']}]" if i["unit"] else ""
            roles = ""
            if i["impl"] or i["review"]:
                roles = f" (实现 {i['impl'] or '-'},复验 {'、'.join(i['review']) or '-'})"
            print(f"#{i['number']}{unit}{roles} {i['title']}")
            print(f"    {i['reason']}")
    if not args.all:
        print(f"== 已了结({counts[SETTLED]}),--all 查看")
    for w in warnings:
        print(f"warning: {w}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
