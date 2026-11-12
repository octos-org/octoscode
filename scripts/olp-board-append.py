#!/usr/bin/env python3
"""Append literal UTF-8 bytes under an existing board lock and verify them."""

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import sys
import time


STAMP = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", re.ASCII)
PROTOCOL_PREFIX = re.compile(rb"(?m)^> OLP-EVENT ")
HARD_LINKED = ("Board has more than one hard link; each name would lock its own <name>.lock, "
               "so keep exactly one link to the board")


def timestamp(value):
    if not isinstance(value, str) or not STAMP.fullmatch(value):
        raise ValueError("Timestamp must be YYYY-MM-DDTHH:MM:SSZ")
    datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    return value


def identity(value):
    return value.st_dev, value.st_ino


def printable(value):
    """A copy of a JSON-ready value whose strings all encode as UTF-8.

    Command-line arguments reach Python with undecodable bytes as lone
    surrogates, which no UTF-8 stream accepts; the inbox and the sentinel show
    them as backslash escapes rather than failing the output after the work is
    done.
    """
    if isinstance(value, str):
        return value.encode("utf-8", "backslashreplace").decode("utf-8")
    if isinstance(value, dict):
        return {printable(key): printable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [printable(item) for item in value]
    return value


def single_link(fd):
    """Refuse a board that gained a second name after board_path() checked it.

    Checked on the opened board once the lock is held, which covers a link
    made while the lock was awaited. Locks derive from paths, so a link made
    and removed concurrently with a write stays outside what they can guard.
    """
    if os.fstat(fd).st_nlink != 1:
        raise ValueError(HARD_LINKED)


@contextmanager
def regular_file(path, flags):
    fd = os.open(str(path), flags | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError("Expected a regular file: " + str(path))
        yield fd
    finally:
        os.close(fd)


@contextmanager
def locked(fd, timeout, shared=False):
    """Hold the board lock; readers share it, writers hold it exclusively."""
    if not math.isfinite(timeout) or timeout < 0:
        raise ValueError("Lock timeout must be finite and nonnegative")
    mode = fcntl.LOCK_SH if shared else fcntl.LOCK_EX
    deadline = time.monotonic() + timeout
    while True:
        try:
            fcntl.flock(fd, mode | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Board lock unavailable; no unlocked fallback")
            time.sleep(min(0.05, remaining))
    try:
        yield
    finally:
        fcntl.flock(fd, fcntl.LOCK_UN)


def read_all(fd):
    chunks = []
    while True:
        chunk = os.read(fd, 65536)
        if not chunk:
            return b"".join(chunks)
        chunks.append(chunk)


def read_region(fd, offset, length):
    chunks = []
    while length:
        chunk = os.pread(fd, min(length, 65536), offset)
        if not chunk:
            break
        chunks.append(chunk)
        offset += len(chunk)
        length -= len(chunk)
    return b"".join(chunks)


def write_all(fd, payload):
    remaining = memoryview(payload)
    while remaining:
        count = os.write(fd, remaining)
        if count <= 0:
            raise OSError("Append made no progress")
        remaining = remaining[count:]


def board_path(value):
    """Resolve the canonical board path and refuse a symlinked or hard-linked board.

    Every tool locks `<board path>.lock`. The legacy shell would lock
    `<link>.lock` for a symlinked board while these tools resolve it to
    `<target>.lock`, and each name of a hard-linked board has a lock of its
    own; either way one board ends up in two lock domains. Directory symlinks
    are harmless because the lock sits beside the board.
    """
    if os.path.islink(str(value)):
        raise ValueError("Board path is a symlink; pass the real board path so every "
                         "writer shares one lock")
    path = Path(value).resolve(strict=True)
    if path.stat().st_nlink != 1:
        raise ValueError(HARD_LINKED)
    return path


def _paths(board_path_value, body_path=None):
    board = board_path(board_path_value)
    lock_path = Path(str(board) + ".lock")
    lock_path.resolve(strict=True)
    body = Path(body_path).resolve(strict=True) if body_path is not None else None
    return board, lock_path, body


def read_snapshot(board_path_value, lock_timeout):
    """Read the whole board under a shared lock and release it before parsing."""
    board, lock_path, _ = _paths(board_path_value)
    with regular_file(lock_path, os.O_RDONLY) as lock_fd:
        with locked(lock_fd, lock_timeout, shared=True):
            with regular_file(board, os.O_RDONLY) as board_fd:
                if identity(os.fstat(board_fd)) == identity(os.fstat(lock_fd)):
                    raise ValueError("Board must not alias its lock")
                single_link(board_fd)
                data = read_all(board_fd)
                if identity(board.stat()) != identity(os.fstat(board_fd)):
                    raise ValueError("Board path changed during read")
    return board, data


def _validate_body(body, max_bytes=None, allow_protocol=False):
    if not body:
        raise ValueError("Body must not be empty")
    if max_bytes is not None and (max_bytes < 1 or len(body) > max_bytes):
        raise ValueError("Body exceeds max-bytes or limit is not positive")
    body.decode("utf-8", errors="strict")
    if not allow_protocol and PROTOCOL_PREFIX.search(body):
        raise ValueError("Protocol prefixes require the protocol writer")
    if not body.endswith(b"\n"):
        raise ValueError("Body must end with LF; bytes are never normalized")
    if re.search(rb"(?m)^ts=\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z\r?$", body):
        raise ValueError("The tool owns standalone UTC timestamp lines, fenced examples "
                         "included; indent or reword the example line")


def _receipt(board, offset, body, payload, ts):
    return {
        "board": str(board),
        "offset": offset,
        "length": len(payload),
        "body_sha256": hashlib.sha256(body).hexdigest(),
        "payload_sha256": hashlib.sha256(payload).hexdigest(),
        "ts": ts,
        "verified": True,
    }


def append_generated(board_path_value, lock_timeout, builder, progress,
                     may_terminate_partial_line=False):
    """Build and append a protocol payload while holding the canonical lock.

    Only a caller that explicitly quarantines a partial trailing line may
    start its payload with LF to terminate that line; everyone else refuses
    to merge bytes into a partial line.
    """
    board, lock_path, _ = _paths(board_path_value)
    with regular_file(lock_path, os.O_RDONLY) as lock_fd:
        with locked(lock_fd, lock_timeout):
            if identity(lock_path.stat()) != identity(os.fstat(lock_fd)):
                raise ValueError("Lock path changed")
            with regular_file(board, os.O_RDWR | os.O_APPEND) as fd:
                if identity(os.fstat(fd)) == identity(os.fstat(lock_fd)):
                    raise ValueError("Board must not alias its lock")
                single_link(fd)
                offset = os.lseek(fd, 0, os.SEEK_END)
                existing = read_region(fd, 0, offset)
                partial = bool(existing) and not existing.endswith(b"\n")
                if partial and not may_terminate_partial_line:
                    raise ValueError("Board must end with LF before appending; void the "
                                     "partial trailing line first")
                ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
                body = builder(existing, offset, ts)
                if partial != body.startswith(b"\n"):
                    raise ValueError("Only a partial trailing line may be terminated")
                _validate_body(body, allow_protocol=True)
                payload = body + ("ts=" + ts + "\n").encode("ascii")
                progress["may_have_appended"] = True
                write_all(fd, payload)
                os.fsync(fd)
                if read_region(fd, offset, len(payload)) != payload:
                    raise OSError("Byte verification failed; do not truncate or blindly retry")
                if identity(board.stat()) != identity(os.fstat(fd)):
                    raise OSError("Board path changed during operation")
                return _receipt(board, offset, body, payload, ts)


def run(args, progress):
    board, lock_path, body_path = _paths(args.board, args.body_file)
    shared = args.command == "verify"
    with regular_file(lock_path, os.O_RDONLY) as lock_fd:
        with regular_file(body_path, os.O_RDONLY) as body_fd:
            body_stat = os.fstat(body_fd)
            if identity(body_stat) in (identity(board.stat()), identity(os.fstat(lock_fd))):
                raise ValueError("Body must not alias the board or its lock")
            body = read_all(body_fd)
            _validate_body(body, getattr(args, "max_bytes", None))
        with locked(lock_fd, args.lock_timeout, shared=shared):
            if identity(lock_path.stat()) != identity(os.fstat(lock_fd)):
                raise ValueError("Lock path changed")
            flags = os.O_RDWR | os.O_APPEND if args.command == "append" else os.O_RDONLY
            with regular_file(board, flags) as fd:
                if identity(os.fstat(fd)) in (identity(body_stat), identity(os.fstat(lock_fd))):
                    raise ValueError("Board must not alias the body or lock")
                single_link(fd)
                if args.command == "append":
                    offset = os.lseek(fd, 0, os.SEEK_END)
                    partial = bool(offset) and os.pread(fd, 1, offset - 1) != b"\n"
                    terminate = getattr(args, "terminate_partial_line", False)
                    if partial and not (terminate and body.startswith(b"\n")):
                        raise ValueError("Board must end with LF before appending")
                    if terminate and not partial:
                        raise ValueError("--terminate-partial-line needs a board that ends "
                                         "with a partial line")
                    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
                else:
                    offset = args.offset
                    if offset < 0:
                        raise ValueError("Offset must be nonnegative")
                    ts = timestamp(args.ts)
                payload = body + ("ts=" + ts + "\n").encode("ascii")
                if args.command == "append":
                    progress["may_have_appended"] = True
                    write_all(fd, payload)
                    os.fsync(fd)
                if read_region(fd, offset, len(payload)) != payload:
                    raise OSError("Byte verification failed; do not truncate or blindly retry")
                if identity(board.stat()) != identity(os.fstat(fd)):
                    raise OSError("Board path changed during operation")
                return _receipt(board, offset, body, payload, ts)


class MachineArgumentParser(argparse.ArgumentParser):
    """Report an argument error like any other failure: one machine JSON, nothing appended."""

    def error(self, message):
        print(json.dumps({"verified": False, "error": self.prog + ": " + message,
                          "may_have_appended": False,
                          "action": "Inspect evidence; never truncate or blindly retry"}),
              file=sys.stderr, flush=True)
        raise SystemExit(2)


def finite_nonnegative(value):
    number = float(value)
    if not math.isfinite(number) or number < 0:
        raise argparse.ArgumentTypeError("Expected a finite nonnegative number")
    return number


def main(argv=None):
    # Subcommand parsers inherit the parser class, so they report the same way.
    parser = MachineArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("append", "verify"):
        command = commands.add_parser(name)
        command.add_argument("--board", required=True)
        command.add_argument("--body-file", required=True)
        command.add_argument("--lock-timeout", type=finite_nonnegative, default=10.0)
        if name == "append":
            command.add_argument("--max-bytes", type=int)
            # Explicitly terminate a partial last line (the body must start
            # with LF), e.g. to close a code fence that would hide a void.
            command.add_argument("--terminate-partial-line", action="store_true")
        else:
            command.add_argument("--offset", type=int, required=True)
            command.add_argument("--ts", required=True)
    args = parser.parse_args(argv)
    progress = {"may_have_appended": False}
    try:
        print(json.dumps(run(args, progress), ensure_ascii=False), flush=True)
        return 0
    except (OSError, ValueError, OverflowError, KeyboardInterrupt) as error:
        print(json.dumps({"verified": False, "error": str(error), **progress,
                          "action": "Inspect evidence; never truncate or blindly retry"}),
              file=sys.stderr, flush=True)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
