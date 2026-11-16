#!/usr/bin/env python3
"""OLP 派卡:一条命令完成"查单元、写板、唤醒"。

用法(在 hub 上,由持锁的大外环执行):
  scripts/olp-dispatch.py <单元号> --board <黑板> --title <标题> [选项]  <<'EOF'
  背景、精确文件与行号、修法方向、验收标准、分支 task/{N} …
  EOF

派一张卡原本是四步:查这台单元健不健康、查它是不是正忙、写板、唤醒。
每一步都靠外环记得去做,漏一步卡就悬着。本命令依次完成:

  1. 健康:`olp-unit.py check <单元号>` 必须通过(--skip-health 跳过)。
  2. 空闲:一台单元同一时间只做一张卡。板上还有派给它、没了结的卡时
     拒绝(--allow-busy 放行)。
  3. 回测不派给原作者:--verify-of <M> 表示这是 #M 的回测卡,目标单元
     不能是 #M 所派的那台。
  4. 写板:经 olp-board-append.sh 在锁内分配编号,条目带 `unit:` 行
     (回测卡另带 `verify-of: #M`)。正文里的 `{N}` 会被替换成编号。
  5. 唤醒:给了 --wake <窗格> 时执行
     `herdr --machine <单元号> agent prompt <窗格> '<一句话,指向条目号>'`。
     单元就是本机时(单机部署,登记的主机名等于本机主机名)不带 --machine。

stdout 最后一行是分配到的编号。大外环随后对 `本地复验通过(#N)` 挂正哨。

可以安全重试
  每次派发带一个键(--key;不给时由单元号、标题、正文算出)。写板成功但
  唤醒失败时,原样再跑一遍:板上已有这个键,不会再写第二张卡,也不会因为
  "单元正忙"(忙的正是这张卡)被拒,只补做唤醒。

选项
  --title T          条目标题(必填)
  --signature S      署名,默认 外环(hub)
  --verify-of M      这是 #M 的回测卡
  --impl AGENT       单元里由哪种内环实现(herdr 的种类,如 octoscode、claude、codex)
  --review A[,B…]    单元里由哪些内环对抗复验(至少一个;复验通过才交回 hub)
  --key K            幂等键,[A-Za-z0-9._:/@+=-]
  --wake PANE        写板后唤醒这台单元上的窗格
  --skip-health      不查单元健康(登记工具没部署时)
  --allow-busy       单元正忙也派
  --date YYYY-MM-DD  标题里的日期(默认今天)
  --json             以 JSON 输出结果

退出码:0 成功;1 被规则拒绝(不健康、正忙、回测派给了原作者),未写板;
2 用法错误;4 写板失败;5 已写板但唤醒失败(原样重跑即可)。
"""

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys

UNIT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
AGENT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
KEY_RE = re.compile(r"^[A-Za-z0-9._:/@+=-]{1,200}$")
KEY_LINE = re.compile(r"^<!-- olp-key: (\S+)(?: n=(\d+))? -->$")
HERE = os.path.dirname(os.path.abspath(__file__))


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def load_inbox():
    spec = importlib.util.spec_from_file_location("olp_inbox", os.path.join(HERE, "olp-inbox.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def existing_number(board, key):
    """The entry number recorded for `key` on the board, or None."""
    with open(board, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            m = KEY_LINE.match(line.rstrip("\r\n"))
            if m and m.group(1) == key and m.group(2):
                return int(m.group(2))
    return None


def check_health(unit):
    r = subprocess.run(
        [sys.executable, "-B", os.path.join(HERE, "olp-unit.py"), "check", unit],
        capture_output=True,
        text=True,
    )
    if r.returncode == 0:
        return
    detail = (r.stderr.strip() or r.stdout.strip()).splitlines()
    raise Fail(1, f"单元 {unit} 现在不能接卡:{detail[-1] if detail else '健康检查未通过'}")


def unit_is_local(unit):
    """Single machine: the unit is registered with this machine's hostname,
    so its panes are in the local herdr, not on a saved herdr machine."""
    r = subprocess.run(
        [sys.executable, "-B", os.path.join(HERE, "olp-unit.py"), "where", unit],
        capture_output=True,
        text=True,
    )
    return r.returncode == 0 and r.stdout.strip() == "local"


def wake(unit, pane, number, impl="", reviewers=()):
    herdr = os.environ.get("OLP_HERDR") or "herdr"
    # The unit reads a mirror of the board. It was last synced before this
    # card was written, so the wake names the sync as the first step — as a
    # command that runs as written from any directory (a real inner loop once
    # spent minutes searching the disk for a tool given only by name), with
    # the script path as the fallback for an octoscode without `olp`.
    sync = (
        "octoscode olp remote mirror"
        '(不可用时:python3 "$(cat ~/.octos/outer/octoscode-home)/scripts/olp-board-remote.py" mirror)'
    )
    message = f"黑板新条目 #{number} 派给本单元。先同步本机的黑板镜像:{sync},再读 #{number} 执行"
    if impl or reviewers:
        # olp/v3: the pane woken for a card with roles is the unit host, who
        # organises the work rather than doing it. Say so, and where the
        # rules are, or it will start implementing.
        message = (
            f"黑板新条目 #{number} 派给本单元,指定实现方 {impl or '(由你定)'}、"
            f"复验方 {'、'.join(reviewers) or '(由你定)'}。你是单元主持,不要自己实现:"
            "按 $(cat ~/.octos/outer/octoscode-home)/skills/octoloop/AGENTS.md 的"
            f"「模式 unit」组织实现与对抗复验,全部复验通过才推送并向 hub 回执。第一步同步镜像:{sync}"
        )
    try:
        r = subprocess.run(
            [herdr, *([] if unit_is_local(unit) else ["--machine", unit]), "agent", "prompt", pane, message],
            capture_output=True,
            text=True,
            timeout=60,
        )
    except (OSError, subprocess.SubprocessError) as e:
        return False, str(e)
    return r.returncode == 0, (r.stderr.strip() or r.stdout.strip())


def main():
    ap = argparse.ArgumentParser(prog="olp-dispatch.py", description="OLP 派卡")
    ap.add_argument("unit")
    ap.add_argument("--board", required=True)
    ap.add_argument("--title", required=True)
    ap.add_argument("--signature", default="外环(hub)")
    ap.add_argument("--verify-of", type=int)
    ap.add_argument("--impl")
    ap.add_argument("--review")
    ap.add_argument("--key")
    ap.add_argument("--wake")
    ap.add_argument("--skip-health", action="store_true")
    ap.add_argument("--allow-busy", action="store_true")
    ap.add_argument("--date")
    ap.add_argument("--json", action="store_true")
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    try:
        return run(args)
    except Fail as e:
        print(f"olp-dispatch: {e}", file=sys.stderr)
        return e.code


def run(args):
    unit = args.unit
    if not UNIT_RE.match(unit):
        raise Fail(2, f"单元号不合法: {unit}")
    # Roles in the unit, chosen per card by the outer loop: one implementer
    # kind and the kinds that review it adversarially. Free names, so any
    # inner loop herdr knows can take part.
    impl = (args.impl or "").strip()
    reviewers = [r.strip() for r in (args.review or "").split(",") if r.strip()]
    for name in [impl] * bool(impl) + reviewers:
        if not AGENT_RE.match(name):
            raise Fail(2, f"内环种类不合法: {name}(小写字母、数字、连字符)")
    if len(set(reviewers)) != len(reviewers):
        raise Fail(2, "--review 里有重复的内环")
    if impl and not reviewers:
        raise Fail(2, "指定了 --impl 就要用 --review 指定至少一个复验方:单元复验通过才交回")
    if not os.path.isfile(args.board):
        raise Fail(2, f"黑板不存在: {args.board}")
    title = args.title.strip()
    if not title or "\n" in title or "\r" in title:
        raise Fail(2, "--title 必须是非空的单行")
    if "\n" in args.signature or not args.signature.strip():
        raise Fail(2, "--signature 必须是非空的单行")
    if args.date:
        try:
            date = datetime.date.fromisoformat(args.date)
        except ValueError:
            raise Fail(2, f"--date 不是合法日期: {args.date}")
    else:
        date = datetime.date.today()
    body = sys.stdin.read()
    if not body.strip():
        raise Fail(2, "正文为空(从 stdin 喂)")
    if args.key:
        if not KEY_RE.match(args.key):
            raise Fail(2, "--key 只允许 [A-Za-z0-9._:/@+=-],最长 200")
        key = args.key
    else:
        # The roles are part of what is dispatched: the same card with other
        # reviewers is a different dispatch. Cards without roles keep the
        # key they always had.
        roles = f"\nimpl={impl}\nreview={','.join(reviewers)}" if impl or reviewers else ""
        digest = hashlib.sha256(f"{unit}\n{title}\n{body}{roles}".encode("utf-8")).hexdigest()[:16]
        key = f"dispatch:{unit}:{digest}"

    number = existing_number(args.board, key)
    created = number is None
    if created:
        inbox = load_inbox()
        if not args.skip_health:
            check_health(unit)
        entries, _, _ = inbox.scan(args.board)
        if args.verify_of is not None:
            original = entries.get(args.verify_of)
            if original is None:
                raise Fail(1, f"板上没有 #{args.verify_of},无法派它的回测卡")
            if original["unit"] == unit:
                raise Fail(1, f"#{args.verify_of} 就是 {unit} 做的,回测卡要派给另一台单元")
        if not args.allow_busy:
            load, _ = inbox.unit_load(args.board, date)
            held = load.get(unit, [])
            if held:
                cards = "、".join(f"#{c['number']}" for c in held)
                raise Fail(1, f"单元 {unit} 正忙:还占着 {cards}。等它了结,或加 --allow-busy")

        lines = [f"### {{N}}. {title}({date.isoformat()},{args.signature.strip()})", f"unit: {unit}"]
        if args.verify_of is not None:
            lines.append(f"verify-of: #{args.verify_of}")
        if impl:
            lines.append(f"impl: {impl}")
        if reviewers:
            lines.append("review: " + ", ".join(reviewers))
        entry = "\n".join(lines) + "\n\n" + body.rstrip("\n") + "\n"
        r = subprocess.run(
            ["bash", os.path.join(HERE, "olp-board-append.sh"), "--key", key, args.board],
            input=entry,
            capture_output=True,
            text=True,
        )
        if r.returncode != 0 or not r.stdout.strip().isdigit():
            raise Fail(4, f"写板失败: {r.stderr.strip() or r.stdout.strip()}")
        number = int(r.stdout.strip())

    woken, wake_error = None, ""
    if args.wake:
        woken, wake_error = wake(unit, args.wake, number, impl, reviewers)

    result = {
        "number": number,
        "unit": unit,
        "created": created,
        "key": key,
        "woken": woken,
        "watch_token": f"本地复验通过(#{number})",
    }
    if args.json:
        json.dump(result, sys.stdout, ensure_ascii=False)
        sys.stdout.write("\n")
    else:
        if created:
            print(f"已派出 #{number} → {unit}")
        else:
            print(f"#{number} 已在板上(同一个键),没有重复派卡")
        if woken:
            print(f"已唤醒 {unit} 的窗格 {args.wake}")
        print(f"挂哨标记:本地复验通过(#{number})")
        print(number)
    if woken is False:
        print(
            f"olp-dispatch: #{number} 已写板,但唤醒 {unit} 失败:{wake_error}。原样重跑本命令即可补做唤醒",
            file=sys.stderr,
        )
        return 5
    return 0


if __name__ == "__main__":
    sys.exit(main())
