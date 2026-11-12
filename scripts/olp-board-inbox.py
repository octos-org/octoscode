#!/usr/bin/env python3
"""Query or wait for actionable olp-board/v1 ledger state."""

import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import sys
import time


def event_module():
    path = Path(__file__).with_name("olp-board-event.py")
    spec = importlib.util.spec_from_file_location("olp_board_event", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def positive_finite(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("Expected a positive finite number")
    return number


def nonnegative_finite(value):
    number = float(value)
    if not math.isfinite(number) or number < 0:
        raise argparse.ArgumentTypeError("Expected a finite nonnegative number")
    return number


def printable(value):
    """Keep machine output valid UTF-8 JSON when arguments carry non-UTF-8 bytes."""
    return event_module().board_module().printable(value)


def query(board, role, actor, since_head=None, lock_timeout=10.0):
    """Project actionable ledger state for one role.

    With `since_head`, `messages` keeps only entries whose triggering event
    was appended after that event, so a caller that persisted the head of an
    output it fully handled is not woken again by work it deliberately left
    open (its own in-flight item, an ACK or escalation it deferred). Items a
    runtime has not received yet are never filtered: they always need work.
    """
    event = event_module()
    _, state = event.read_state(board, lock_timeout)
    projection = state.projection()
    floor = -1
    if since_head is not None:
        if since_head not in state.order:
            raise ValueError("Unknown --since-head event: " + since_head)
        floor = state.order[since_head]

    def fresh(entry):
        return state.order[state.trigger(entry)] > floor

    unreceived = []
    received_pending = []
    if role == "runtime":
        identity = actor or "runtime"
        unreceived = [item for item in projection["unreceived"] if item["to"] == identity]
        received_pending = [item for item in projection["received_pending"]
                            if item["to"] == identity]
        messages = unreceived + [item for item in received_pending if fresh(item)]
        category = "runtime_action"
    else:
        identity = actor
        messages = projection["unreviewed_ack"] + projection["escalated"]
        if identity:
            messages = [entry for entry in messages
                        if state.items[(entry.get("item") or state.acks[entry["ack"]]["item"])]["event"]["actor"] == identity]
        messages = [entry for entry in messages if fresh(entry)]
        messages.sort(key=lambda entry: state.order[entry["id"]])
        category = "outer_action"
    return {"schema": event.SCHEMA, "mode": projection["mode"], "for": role,
            "actor": identity, "head": projection["head"], "since_head": since_head,
            "category": category, "matched": bool(messages), "messages": messages,
            "unreceived": unreceived, "received_pending": received_pending,
            "drift": projection["drift"],
            "dispatch_blocked": projection["dispatch_blocked"],
            "partial_tail": projection["partial_tail"], "execution_authorized": False,
            "limitation": ("legacy mode requires the existing text workflow"
                           if projection["mode"] == "legacy" else None)}


def write_ready(path, result):
    with open(path, "x", encoding="utf-8") as stream:
        json.dump(printable({"status": "initial_query_ok", "pid": os.getpid(), **result}), stream,
                  ensure_ascii=False, allow_nan=False)
        stream.write("\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--board", required=True)
    parser.add_argument("--for", dest="role", choices=("runtime", "outer"), required=True)
    parser.add_argument("--actor")
    parser.add_argument("--wait", action="store_true")
    parser.add_argument("--interval", type=positive_finite, default=10.0)
    parser.add_argument("--timeout", type=positive_finite, default=1800.0)
    parser.add_argument("--ready-file")
    parser.add_argument("--since-head")
    parser.add_argument("--lock-timeout", type=nonnegative_finite, default=10.0)
    args = parser.parse_args(argv)
    try:
        deadline = time.monotonic() + args.timeout
        initialized = False
        while True:
            result = query(args.board, args.role, args.actor, args.since_head, args.lock_timeout)
            if not initialized:
                initialized = True
                if args.ready_file:
                    write_ready(args.ready_file, result)
            if result["dispatch_blocked"] or result["matched"] or not args.wait:
                print(json.dumps(printable(result), ensure_ascii=False, allow_nan=False))
                return 0
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                print(json.dumps(printable({"status": "timeout", **result}), ensure_ascii=False),
                      file=sys.stderr)
                return 3
            time.sleep(min(args.interval, remaining))
    # RecursionError: deeply nested JSON input is an input error like any other.
    except (OSError, ValueError, KeyError, TypeError, OverflowError, RecursionError,
            KeyboardInterrupt) as error:
        # Escaped in place rather than through printable(): the error may be a
        # failure to load the board tools that printable() itself needs.
        message = str(error).encode("utf-8", "backslashreplace").decode("utf-8")
        print(json.dumps({"status": "error", "matched": False, "error": message},
                         ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
