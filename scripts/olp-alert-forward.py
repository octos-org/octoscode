#!/usr/bin/env python3
"""OLP 负哨转发:把单元上运行时守护的告警送到 hub 的黑板。

用法(在单元上,接在守护命令后面):
  octoscode olp-runtime-watch … | scripts/olp-alert-forward.py [--include tool_failed]

`octoscode olp-runtime-watch` 在单元本机观察内环:进程是否还在、目标
turn 有没有报错、是不是在空转。它只把观察结果按行输出(NDJSON),不通知
任何人。本工具读这些行,把值得大外环知道的事经 `olp-board-remote.py`
追加到 hub 的黑板——任何一台单元上的熔断都不会沉默。

转发哪些事
  默认转发:
    agent_missing              内环进程不在了或成了僵尸
    backend_missing            后端进程不在了或成了僵尸
    watcher_missing            观测哨进程不在了
    process_identity_changed   pid 被别的进程复用
    turn_error                 目标 turn 报错
    stalled_no_progress        长时间没有真实进展(提示性质)
  默认不转发:
    tool_failed                单次工具失败在正常工作中很常见,会刷屏;
                               需要时用 --include tool_failed 打开
  --only <kind>[,<kind>…] 只转发列出的种类。

每件事只报一次
  同一件事在它持续期间只转发一次(守护每个心跳都会重复它)。它消失后再
  出现算新的一次。转发带键,断网排队后重发也不会在板上重复。

板上的行
  > 单元(<单元号>)·负哨(#<任务>): <种类> turn=<turn> <细节>
  任务取守护的第一个 --task;是数字时写成 #N。大外环对 `负哨(` 挂哨。
  只转发白名单字段:种类、pid、工具名、序号、停滞秒数,不含任何输出
  内容。

本工具不判断、不处置:不杀进程、不重启、不回写单元。hub 不可达时由
olp-board-remote.py 排队。读到 stdin 结束即退出。

退出码:0 stdin 结束;2 用法或配置错误。
"""

import argparse
import hashlib
import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_KINDS = [
    "agent_missing",
    "backend_missing",
    "watcher_missing",
    "process_identity_changed",
    "turn_error",
    "stalled_no_progress",
]
ALL_KINDS = DEFAULT_KINDS + ["tool_failed"]
SAFE = re.compile(r"[^A-Za-z0-9._:/@+=-]")


def load_remote():
    spec = importlib.util.spec_from_file_location(
        "olp_board_remote", os.path.join(HERE, "olp-board-remote.py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def clean(value, limit=80):
    """A value safe to print on one board line and to use inside a key."""
    return SAFE.sub("_", str(value))[:limit]


def events_of(record):
    """The things worth reporting in one observation record, as
    (kind, discriminator, detail) tuples built from whitelisted fields."""
    out = []
    for role in ("agent", "backend"):
        state = (record.get(role) or {}).get("state")
        if state in ("process_missing", "process_zombie"):
            pid = (record.get("identity") or {}).get(f"{role}_pid")
            out.append((f"{role}_missing", str(pid), f"pid={clean(pid)} state={state}"))
    for monitor in record.get("monitors") or []:
        state = (monitor.get("state") or {}).get("state")
        if state in ("watcher_missing", "process_zombie"):
            pid = monitor.get("pid")
            out.append(("watcher_missing", str(pid), f"pid={clean(pid)} state={state}"))
    for alert in record.get("alerts") or []:
        kind = alert.get("kind")
        if kind == "process_identity_changed":
            out.append((kind, str(alert.get("pid")), f"pid={clean(alert.get('pid'))}"))
        elif kind == "turn_error":
            out.append((kind, str(alert.get("seq")), f"seq={clean(alert.get('seq'))}"))
        elif kind == "tool_failed":
            detail = f"tool={clean(alert.get('tool_name'))} seq={clean(alert.get('seq'))}"
            out.append((kind, str(alert.get("seq")), detail))
        elif kind == "stalled_no_progress":
            detail = f"age={clean(alert.get('progress_age_seconds'))}s activity={clean(alert.get('activity'))}"
            out.append((kind, "", detail))
    return out


def main():
    ap = argparse.ArgumentParser(prog="olp-alert-forward.py", description="OLP 负哨转发")
    ap.add_argument("--hub")
    ap.add_argument("--unit")
    ap.add_argument("--include", action="append", default=[], help="额外转发的种类,如 tool_failed")
    ap.add_argument("--only", help="只转发这些种类,逗号分隔")
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    kinds = set(DEFAULT_KINDS)
    if args.only:
        kinds = set(k for k in args.only.split(",") if k)
    kinds |= set(args.include)
    unknown = sorted(kinds - set(ALL_KINDS))
    if unknown or not kinds:
        print(f"olp-alert-forward: 未知的种类: {', '.join(unknown) or '(空)'}", file=sys.stderr)
        return 2

    remote = load_remote()
    try:
        hub, unit = remote.resolve(args)
    except remote.Fail as e:
        print(f"olp-alert-forward: {e}", file=sys.stderr)
        return 2

    active = {}  # (kind, discriminator) -> episode number while it persists
    episodes = {}  # (kind, discriminator) -> how many times it has started
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            record = json.loads(raw)
        except ValueError:
            continue
        if not isinstance(record, dict):
            continue
        identity = record.get("identity") or {}
        tasks = identity.get("task") or []
        task = clean(tasks[0] if tasks else "unknown", 40)
        task_label = f"#{task}" if task.isdigit() else task
        turn = clean(identity.get("turn_id", "unknown"), 60)

        present = set()
        for kind, disc, detail in events_of(record):
            if kind not in kinds:
                continue
            ident = (kind, disc)
            present.add(ident)
            if ident in active:
                continue
            episodes[ident] = episodes.get(ident, 0) + 1
            active[ident] = episodes[ident]
            line = f"> 单元({unit})·负哨({task_label}): {kind} turn={turn} {detail}\n"
            # 键只要对同一件事稳定即可;用摘要避免超出键的长度上限。
            digest = hashlib.sha256(
                f"{task}\n{turn}\n{kind}\n{disc}\n{episodes[ident]}".encode("utf-8")
            ).hexdigest()[:24]
            key = f"neg:{kind}:{digest}"
            body = line.encode("utf-8")
            try:
                code = remote.deliver(hub, remote.make_key(unit, key, body), body)
            except remote.Fail as e:
                print(f"olp-alert-forward: {e}", file=sys.stderr)
                continue
            if code not in (0, remote.EX_TEMPFAIL):
                print(f"olp-alert-forward: hub 拒绝了 {kind}(退出码 {code})", file=sys.stderr)
        # 这一条记录里不再出现的事结束了;再出现时算新的一次。
        for ident in [i for i in active if i not in present]:
            del active[ident]
    return 0


if __name__ == "__main__":
    sys.exit(main())
