#!/usr/bin/env python3
"""Watch structured ledger state and post-baseline board text."""

import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import sys
import time


def inbox_module():
    path = Path(__file__).with_name("olp-board-inbox.py")
    spec = importlib.util.spec_from_file_location("olp_board_inbox", path)
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


# Set once the board tools load: keeps every emitted line valid UTF-8 JSON when
# the token, actor or --since-head carries bytes that are not UTF-8.
PRINTABLE = None


def emit(prefix, payload):
    if PRINTABLE is None:
        text = json.dumps(payload, allow_nan=False)
    else:
        text = json.dumps(PRINTABLE(payload), ensure_ascii=False, allow_nan=False)
    print(prefix + ": " + text, flush=True)


def signature(result):
    return tuple(sorted(event["id"] for event in result["messages"]))


def write_ready(path, result):
    with open(path, "x", encoding="utf-8") as stream:
        json.dump(PRINTABLE({"status": "sentinel_ready", "pid": os.getpid(), **result}), stream,
                  ensure_ascii=False, allow_nan=False)
        stream.write("\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--board", required=True)
    parser.add_argument("--token", required=True)
    parser.add_argument("--for", dest="role", choices=("runtime", "outer"), required=True)
    parser.add_argument("--actor")
    parser.add_argument("--skip-signature", action="append", default=[])
    parser.add_argument("--interval", type=positive_finite, default=10.0)
    parser.add_argument("--timeout", type=positive_finite, default=1800.0)
    parser.add_argument("--ready-file")
    parser.add_argument("--since-head")
    parser.add_argument("--lock-timeout", type=nonnegative_finite, default=10.0)
    args = parser.parse_args(argv)
    try:
        # Board text is matched as bytes: the legacy shell appender never
        # validates UTF-8, and a line it wrote must wake the watcher, not end it.
        token = os.fsencode(args.token)
        skips = [os.fsencode(skip) for skip in args.skip_signature]
        global PRINTABLE
        inbox = inbox_module()
        PRINTABLE = inbox.event_module().board_module().printable
        board = inbox.event_module().board_module().board_path(args.board)
        stat = board.stat()
        baseline = stat.st_size
        inode = (stat.st_dev, stat.st_ino)
        result = inbox.query(str(board), args.role, args.actor, args.since_head,
                                 args.lock_timeout)
        if args.ready_file:
            write_ready(args.ready_file, result)
        if result["drift"]:
            emit("DRIFT", result)
            return 0
        if result["matched"]:
            emit("LEDGER-SIGNAL", result)
            return 0
        seen = signature(result)
        deadline = time.monotonic() + args.timeout
        while True:
            current = board.stat()
            if (current.st_dev, current.st_ino) != inode or current.st_size < baseline:
                emit("ERROR", {"error": "board was replaced or shortened", "board": str(board)})
                return 2
            result = inbox.query(str(board), args.role, args.actor, args.since_head,
                                 args.lock_timeout)
            if result["drift"]:
                emit("DRIFT", result)
                return 0
            pending = signature(result)
            if pending and pending != seen:
                emit("LEDGER-SIGNAL", result)
                return 0
            seen = pending
            if current.st_size > baseline:
                with open(board, "rb") as stream:
                    stream.seek(baseline)
                    raw = stream.read(current.st_size - baseline)
                complete = raw.rfind(b"\n") + 1
                if complete == 0:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        emit("TIMEOUT", {"status": "timeout",
                                         "reason": "incomplete trailing line",
                                         "board": str(board)})
                        return 3
                    time.sleep(min(args.interval, remaining))
                    continue
                hits = [line for line in raw[:complete].split(b"\n")
                        if token in line and not any(skip in line for skip in skips)]
                baseline += complete
                if hits:
                    matches = [line.decode("utf-8", errors="replace") for line in hits[:3]]
                    emit("BOARD-SIGNAL", {"token": args.token, "matches": matches,
                                          "board": str(board)})
                    return 0
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                emit("TIMEOUT", {"status": "timeout", **result})
                return 3
            time.sleep(min(args.interval, remaining))
    # RecursionError: deeply nested JSON input is an input error like any other.
    except (OSError, ValueError, KeyError, TypeError, OverflowError, RecursionError,
            KeyboardInterrupt) as error:
        emit("ERROR", {"error": str(error)})
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
