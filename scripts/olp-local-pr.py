#!/usr/bin/env python3
"""OLP 本地 PR:hub 仓库里每个任务分支的状态,以及推 GitHub 前的闸门。

多机部署时,单元把 `task/<N>` 推到 hub 的本地仓库,一次推送就是一个
"本地 PR"。本工具回答:哪些本地 PR 在等大外环审核、哪些被退回了、哪些
已经采认但还没推 GitHub、哪些已经在 GitHub 上等人合并。

子命令(都在 hub 上)
  status --repo <hub.git> --board <黑板> [--remote github] [--json]
      列出仓库里每个 task/<N> 分支的状态。
  promote <N> --repo <hub.git> --board <黑板> [--remote github] [--dry-run]
      把 task/<N> 推到 GitHub 远程。只有黑板上这张卡的最新结论是采认时
      才推;否则退出码 1,什么都不推。开 PR(标题、正文)仍由大外环做。

状态(从两处算出来,不另存)
  数据来源:hub 仓库的引用(分支头、GitHub 远程的跟踪引用)和黑板上的
  三种行首定式——
      > 单元(<单元号>)·本地复验通过(#N): task/N @ <提交号> 复验 <署名>
      > 外环(<署名>)·采认(#N): …
      > 外环(<署名>)·打回(#N): …
  对同一张卡,以黑板上**最后出现**的那一行为准:

  状态          条件                                         等谁
  pending       最后一行是单元推送登记,或没有任何结论        大外环:审核
  returned      最后一行是打回                               单元:返工后重新推送
  accepted      最后一行是采认,GitHub 上还没有这个提交       大外环:promote
  submitted     最后一行是采认,且 GitHub 远程的跟踪引用      人:在 GitHub 上审核合并
                等于分支头

  采认之后单元又推送(登记行落在采认之后)→ 回到 pending:采认的是先前
  那个提交。

  分区(与黑板收件箱一致):pending、accepted 在大外环的收件箱;submitted
  在人的收件箱;returned 是进行中。

也报告对不上的地方(warnings):分支头与最近一次推送登记的提交号不符
(有人绕过钩子改了分支)、GitHub 上是旧提交、板上没有这张卡的任何记录。

"已合并"需要查 GitHub,本工具不联网,不判断。

退出码:0 成功;1 promote 被闸门拒绝或推送失败;2 用法错误、仓库或黑板不存在。
"""

import argparse
import json
import os
import re
import subprocess
import sys

PUSHED = re.compile(r"^单元\(([^)]+)\)·本地复验通过\(#(\d+)\)[:：]?\s*task/\d+\s*@\s*([0-9a-f]{7,64})")
VERDICT = re.compile(r"^外环\(([^)]+)\)·(采认|打回)\(#(\d+)\)")
TASK_BRANCH = re.compile(r"^refs/heads/task/(\d+)$")
REMOTE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")

STATE_ZH = {
    "pending": "待审核",
    "returned": "被退回",
    "accepted": "已采认,待推 GitHub",
    "submitted": "已提交 GitHub,待人合并",
}
WAITING_ON = {"pending": "outer", "returned": "unit", "accepted": "outer", "submitted": "human"}
WAITING_ZH = {"outer": "大外环", "unit": "单元", "human": "人"}
PARTITION = {"pending": "inbox", "accepted": "inbox", "submitted": "inbox", "returned": "working"}


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def git(repo, *args):
    return subprocess.run(
        ["git", "--git-dir", repo, *args], capture_output=True, text=True
    )


def strip_quote(line):
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


def board_events(board):
    """number -> [(line_no, kind, detail)], kind in pushed/accepted/returned.
    Only line-start forms outside code fences count."""
    events = {}
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
            m = PUSHED.match(signed)
            if m:
                events.setdefault(int(m.group(2)), []).append(
                    (no, "pushed", {"unit": m.group(1), "sha": m.group(3)})
                )
                continue
            m = VERDICT.match(signed)
            if m:
                kind = "accepted" if m.group(2) == "采认" else "returned"
                events.setdefault(int(m.group(3)), []).append((no, kind, {"by": m.group(1)}))
    return events


def task_branches(repo):
    r = git(repo, "for-each-ref", "--format=%(refname) %(objectname)", "refs/heads/task/")
    if r.returncode != 0:
        raise Fail(2, f"读不了仓库 {repo}: {r.stderr.strip()}")
    out = {}
    for line in r.stdout.splitlines():
        ref, _, sha = line.partition(" ")
        m = TASK_BRANCH.match(ref)
        if m:
            out[int(m.group(1))] = sha
    return out


def remote_head(repo, remote, number):
    r = git(repo, "rev-parse", "--verify", "--quiet", f"refs/remotes/{remote}/task/{number}")
    return r.stdout.strip() if r.returncode == 0 else None


def evaluate(number, head, events, tracked):
    """Return the status record for one task branch."""
    evs = events.get(number, [])
    warnings = []
    last = evs[-1] if evs else None
    pushes = [e for e in evs if e[1] == "pushed"]
    unit = pushes[-1][2]["unit"] if pushes else None

    if last is None:
        state = "pending"
        warnings.append(f"#{number}:板上没有这个分支的推送登记或结论")
    elif last[1] == "pushed":
        state = "pending"
    elif last[1] == "returned":
        state = "returned"
    else:
        state = "submitted" if tracked == head else "accepted"
        if tracked and tracked != head:
            warnings.append(f"#{number}:GitHub 上是旧提交 {tracked[:12]},分支头是 {head[:12]}")

    if pushes and not head.startswith(pushes[-1][2]["sha"]):
        warnings.append(
            f"#{number}:分支头 {head[:12]} 与最近一次推送登记的 {pushes[-1][2]['sha'][:12]} 不符"
        )
    return {
        "number": number,
        "branch": f"task/{number}",
        "head": head,
        "unit": unit,
        "state": state,
        "waiting_on": WAITING_ON[state],
        "partition": PARTITION[state],
        "pushes": len(pushes),
    }, warnings


def check_inputs(args):
    if not os.path.isdir(args.repo) or git(args.repo, "rev-parse", "--git-dir").returncode != 0:
        raise Fail(2, f"不是 git 仓库: {args.repo}")
    if not os.path.isfile(args.board):
        raise Fail(2, f"黑板不存在: {args.board}")
    if not REMOTE_RE.match(args.remote):
        raise Fail(2, f"远程名不合法: {args.remote}")


def collect(args):
    events = board_events(args.board)
    items, warnings = [], []
    for number, head in sorted(task_branches(args.repo).items()):
        item, w = evaluate(number, head, events, remote_head(args.repo, args.remote, number))
        items.append(item)
        warnings.extend(w)
    return items, warnings


def cmd_status(args):
    check_inputs(args)
    items, warnings = collect(args)
    counts = {s: sum(1 for i in items if i["state"] == s) for s in STATE_ZH}
    if args.json:
        json.dump(
            {"counts": counts, "items": items, "warnings": warnings},
            sys.stdout,
            ensure_ascii=False,
            indent=2,
        )
        sys.stdout.write("\n")
        return 0
    if not items:
        print("仓库里没有 task/<N> 分支")
    for state in ("pending", "accepted", "submitted", "returned"):
        rows = [i for i in items if i["state"] == state]
        if not rows:
            continue
        print(f"== {STATE_ZH[state]}({len(rows)})· 等{WAITING_ZH[WAITING_ON[state]]}")
        for i in rows:
            unit = f" [{i['unit']}]" if i["unit"] else ""
            print(f"{i['branch']}{unit} @ {i['head'][:12]}  推送 {i['pushes']} 次")
    for w in warnings:
        print(f"warning: {w}", file=sys.stderr)
    return 0


def cmd_promote(args):
    check_inputs(args)
    head = task_branches(args.repo).get(args.number)
    if head is None:
        raise Fail(1, f"仓库里没有分支 task/{args.number},未推送")
    if git(args.repo, "remote", "get-url", args.remote).returncode != 0:
        raise Fail(2, f"仓库里没有名为 {args.remote} 的远程")
    item, _ = evaluate(
        args.number, head, board_events(args.board), remote_head(args.repo, args.remote, args.number)
    )
    if item["state"] == "pending":
        raise Fail(
            1,
            f"#{args.number} 还没有采认:板上这张卡的最新一行不是采认"
            "(单元在采认之后又推送过,也要重新审核)。未推送",
        )
    if item["state"] == "returned":
        raise Fail(1, f"#{args.number} 已被打回,未推送")
    if item["state"] == "submitted":
        print(f"task/{args.number} @ {head[:12]} 已在 {args.remote} 上,无需再推")
        return 0
    ref = f"refs/heads/task/{args.number}"
    if args.dry_run:
        print(f"dry-run:会把 task/{args.number} @ {head[:12]} 推到 {args.remote}")
        return 0
    r = git(args.repo, "push", args.remote, f"{ref}:{ref}")
    if r.returncode != 0:
        raise Fail(1, f"推送到 {args.remote} 失败: {r.stderr.strip()}")
    # A bare repo does not always move the tracking ref on push; record it so
    # `status` can tell the branch is on the remote without going online.
    git(args.repo, "update-ref", f"refs/remotes/{args.remote}/task/{args.number}", head)
    print(f"已把 task/{args.number} @ {head[:12]} 推到 {args.remote};接下来由大外环开 PR")
    return 0


def main():
    ap = argparse.ArgumentParser(prog="olp-local-pr.py", description="OLP 本地 PR 状态与 GitHub 闸门")
    sub = ap.add_subparsers(dest="cmd")
    for name, fn in (("status", cmd_status), ("promote", cmd_promote)):
        p = sub.add_parser(name)
        if name == "promote":
            p.add_argument("number", type=int)
            p.add_argument("--dry-run", action="store_true")
        else:
            p.add_argument("--json", action="store_true")
        p.add_argument("--repo", required=True)
        p.add_argument("--board", required=True)
        p.add_argument("--remote", default="github")
        p.set_defaults(fn=fn)
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    if not getattr(args, "fn", None):
        ap.print_usage(sys.stderr)
        return 2
    try:
        return args.fn(args)
    except Fail as e:
        print(f"olp-local-pr: {e}", file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main())
