#!/usr/bin/env python3
"""OLP 黑板归档:把已闭环条目的正文移到归档文件,板上留编号桩。

用法:
  scripts/olp-board-archive.py <board.md> [--archive <file>] [--keep N] [--dry-run]

黑板只追加,多机写入时增长很快;内环每轮都要读全板。本工具把已闭环的
条目移到归档文件,板上每个条目只留一个**编号桩**:

  ### 12. <原标题>
  ACK(done): <原 ACK 行,逐字保留>
  <!-- olp-archived: OUTER_LOOP_REVIEW.archive.md -->

规则:
  - 条目 = 一个 `### <数字>` 标题到下一个 `###`/`##` 标题之前的全部行。
    代码围栏(```)里的标题不算。
  - 已闭环,满足其一即可:
      a. 外环已经下了结论:板上关于 #N 最后出现的一行是采认或打回
         (`> 外环(…)·采认(#N)` / `·打回(#N)`),或者 #N 被改判作废。结论
         之后单元又推送、内环又 ACK 的,不算闭环。
      b. 板上没有关于 #N 的任何判词或登记时,沿用旧规则:条目里最后一条
         `ACK(` 行是 `ACK(done` 或 `ACK(wontdo`。
    `ACK(blocked`、没有 ACK、旧格式 `ACK:` 的条目不动。
  - 最近的 --keep 个编号条目(默认 10)不论是否闭环都原样保留。
  - 桩保留标题和条目里所有"给机器读"的行:`ACK(` 行、`unit:` 行、写板
    助手 `--key` 留下的 `<!-- olp-key: … -->` 标记,以及署名的行首定式
    (采认、打回、搁置、改判、单元的推送登记、内环的署名 ACK、单元的负哨、
    单元转发的 MCP blocked/timeout 审计行——后两种是进化环采集的信号,
    留在板上,先归档后采集也不会漏)。这些判词
    按 `#N` 归属,物理上可能落在别的条目下面,不能跟着那个条目的正文离开
    黑板。于是:编号不断档(下一个编号仍是板上最大号 +1),内环按"最小未
    ACK 条目"找活时会跳过它,`#12` 的引用仍能在板上找到,收件箱、本地 PR
    状态、hub 的推送钩子读到的内容不变。
  - 板的前言、`##` 小节、非编号标题的内容原样保留,顺序不变。
  - 归档文件只追加,每个条目前有一行 `<!-- olp-archived-entry: N … -->`;
    已在归档文件里的条目不会重复写入,中断后重跑是安全的。

并发:
  - 持有 `<board>.lock` 的 flock(与 olp-board-append.sh 同一把锁),经
    助手的写入不会与归档交错。
  - 内环的原地 ACK 不走这把锁。替换黑板前会核对文件未被改动,变了就以
    退出码 4 放弃替换(归档文件里已写入的条目下次重跑时跳过)。尽量在
    内环空闲时运行。
  - 替换是原子的(同目录临时文件 + rename),读者不会看到半块板。

运行后要做的事:
  - 板的行数变少。常驻的 olp-watch-board.sh 会自己重取基线;在归档那一
    刻前后追加的行可能落在基线之前,重要的一击哨请在归档后重新挂。
  - 进化环采集(olp-evo-harvest.sh)会把板识别为被替换并重扫;已采集的
    信号按 identity 去重。负哨与单元的 MCP 审计行留在桩里,不受归档
    影响。**归档前仍应先跑一次采集**:条目正文里未署名的改判/R2 记档行
    归档后不在板上,尚未采集的会被漏掉。

退出码:0 成功(含无可归档);2 用法错误或板不存在;4 归档期间板被改动。
"""

import argparse
import datetime
import fcntl
import os
import re
import sys
import tempfile

HEADING = re.compile(r"^### (\d+)")
ARCHIVED_MARK = "<!-- olp-archived: "
KEY_MARK = "<!-- olp-key: "
ENTRY_MARK = re.compile(r"^<!-- olp-archived-entry: (\d+) ")


def split_blocks(lines):
    """Split the board into blocks: ("entry", number, lines) for numbered
    entries and ("other", None, lines) for everything else, in file order."""
    blocks = []
    kind, number, cur = "other", None, []
    in_fence = False
    for line in lines:
        stripped = line.lstrip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            in_fence = not in_fence
            cur.append(line)
            continue
        boundary = not in_fence and (line.startswith("### ") or line.startswith("## "))
        if boundary:
            if cur:
                blocks.append((kind, number, cur))
            m = HEADING.match(line)
            if m:
                kind, number = "entry", int(m.group(1))
            else:
                kind, number = "other", None
            cur = [line]
        else:
            cur.append(line)
    if cur:
        blocks.append((kind, number, cur))
    return blocks


UNIT_LINE = re.compile(r"^(unit|impl|review|verify-of):\s*\S.*$")
# 判词与登记的行首定式(剥掉 `> `、`**` 之后判断)。
SIGNED_OUTER = re.compile(r"^外环\([^)]+\)·(采认|打回|搁置)\(#(\d+)\)")
SIGNED_VOID = re.compile(r"^外环\([^)]+\)·改判\(作废 #(\d+)\)")
SIGNED_PUSH = re.compile(r"^单元\([^)]+\)·本地复验通过\(#(\d+)\)")
SIGNED_INNER_ACK = re.compile(
    r"^内环\([^)]+\)·(?:#(\d+)\s*)?ACK\((done|wontdo|blocked)\)(?:[^:：]*[:：]\s*#(\d+))?"
)
# 进化环采集(olp-evo-harvest.sh)在活板上读的单元信号:负哨,以及单元转发
# 来的 MCP blocked / timeout 审计行。
SIGNED_ALERT = re.compile(r"^单元\([^)]+\)·负哨\(")
SIGNED_UNIT_MCP = re.compile(
    r"^单元\([^)]+\)·-\s+\S+\s+\S+\s+MCP\(ask_outer\)\s+(?:blocked|timeout):"
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


def is_signed_form(line):
    signed = strip_quote(line)
    return bool(
        SIGNED_OUTER.match(signed)
        or SIGNED_VOID.match(signed)
        or SIGNED_PUSH.match(signed)
        or SIGNED_INNER_ACK.match(signed)
        or SIGNED_ALERT.match(signed)
        or SIGNED_UNIT_MCP.match(signed)
    )


def ack_lines(entry_lines):
    """The lines a stub must keep, outside code fences and in order — every
    line other tools read by machine:
    - the entry's `ACK(` lines;
    - the idempotency markers olp-board-append.sh writes for `--key` (a
      resend must still find its key on the board);
    - the `unit:` line (the hub's git hook reads it on a rework push);
    - signed line-start forms: verdicts, snoozes, overrides, push records and
      signed inner ACKs. They attach to an entry by `#N` and may physically
      sit under a different entry, so they must not leave the board with it;
    - the unit signals the evolution harvest reads on the live board:
      negative-sentinel lines and forwarded MCP blocked / timeout lines.
      Archiving before a harvest must not lose them.
    """
    out = []
    in_fence = False
    for line in entry_lines:
        stripped = line.lstrip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        bare = line.rstrip("\r\n")
        if (
            line.startswith("ACK(")
            or line.startswith(KEY_MARK)
            or UNIT_LINE.match(bare)
            or is_signed_form(bare)
        ):
            out.append(line)
    return out


def board_verdicts(lines):
    """Scan the whole board for signed forms. Returns (latest, voided):
    latest[N] is the kind of the last event about #N among accepted /
    returned / pushed / inner_ack; voided is the set of overridden entries."""
    latest, voided = {}, set()
    in_fence = False
    current = None
    for line in lines:
        bare = line.rstrip("\r\n")
        stripped = bare.lstrip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if bare.startswith("### ") or bare.startswith("## "):
            m = HEADING.match(bare)
            current = int(m.group(1)) if m else None
            continue
        signed = strip_quote(bare)
        m = SIGNED_OUTER.match(signed)
        if m:
            if m.group(1) != "搁置":
                latest[int(m.group(2))] = "accepted" if m.group(1) == "采认" else "returned"
            continue
        m = SIGNED_VOID.match(signed)
        if m:
            voided.add(int(m.group(1)))
            continue
        m = SIGNED_PUSH.match(signed)
        if m:
            latest[int(m.group(1))] = "pushed"
            continue
        m = SIGNED_INNER_ACK.match(signed)
        if m:
            ref = m.group(1) or m.group(3)
            target = int(ref) if ref else current
            if target is not None:
                latest[target] = "inner_ack"
    return latest, voided


def is_closed(number, entry_lines, latest, voided):
    """Closed when the outer loop has ruled on it — accepted, returned or
    overridden, with nothing newer — or, for boards without verdict lines,
    when the entry's own last ACK is done or wontdo."""
    if number in voided:
        return True
    if number in latest:
        return latest[number] in ("accepted", "returned")
    acks = [l for l in ack_lines(entry_lines) if l.startswith("ACK(")]
    if not acks:
        return False
    last = acks[-1]
    return last.startswith("ACK(done") or last.startswith("ACK(wontdo")


def is_stub(entry_lines):
    return any(line.startswith(ARCHIVED_MARK) for line in entry_lines)


def with_newline(line):
    return line if line.endswith("\n") else line + "\n"


def stat_key(path):
    st = os.stat(path)
    return (st.st_ino, st.st_size, st.st_mtime_ns)


def main():
    ap = argparse.ArgumentParser(
        prog="olp-board-archive.py",
        description="把已闭环条目的正文移到归档文件,板上留编号桩。",
    )
    ap.add_argument("board")
    ap.add_argument("--archive", help="归档文件(默认 <板名>.archive.md,与板同目录)")
    ap.add_argument("--keep", type=int, default=10, help="原样保留最近 N 个编号条目(默认 10)")
    ap.add_argument("--dry-run", action="store_true", help="只列出会归档的条目,不写任何文件")
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    if args.keep < 0:
        print("olp-board-archive: --keep 不能为负数", file=sys.stderr)
        return 2

    board = args.board
    if not os.path.isfile(board):
        print(f"olp-board-archive: 板不存在: {board}", file=sys.stderr)
        return 2
    board_dir = os.path.dirname(os.path.abspath(board))
    if args.archive:
        archive = args.archive
    else:
        stem = os.path.basename(board)
        if stem.endswith(".md"):
            stem = stem[:-3]
        archive = os.path.join(board_dir, stem + ".archive.md")
    if os.path.abspath(archive) == os.path.abspath(board):
        print("olp-board-archive: 归档文件不能是黑板本身", file=sys.stderr)
        return 2
    archive_name = os.path.basename(archive)

    lock_fd = None
    if not args.dry_run:
        lock_fd = os.open(board + ".lock", os.O_WRONLY | os.O_CREAT, 0o644)
        fcntl.flock(lock_fd, fcntl.LOCK_EX)
    try:
        before = stat_key(board)
        with open(board, "r", encoding="utf-8", newline="") as f:
            lines = f.readlines()
        blocks = split_blocks(lines)

        entry_positions = [i for i, b in enumerate(blocks) if b[0] == "entry"]
        protected = set(entry_positions[len(entry_positions) - args.keep:]) if args.keep else set()
        latest, voided = board_verdicts(lines)
        targets = [
            i
            for i in entry_positions
            if i not in protected
            and not is_stub(blocks[i][2])
            and is_closed(blocks[i][1], blocks[i][2], latest, voided)
        ]
        if not targets:
            print("olp-board-archive: 没有可归档的条目")
            return 0

        if args.dry_run:
            for i in targets:
                print(f"would archive #{blocks[i][1]} {blocks[i][2][0].strip()[4:]}")
            print(f"olp-board-archive: dry-run,{len(targets)} 个条目可归档,未写入任何文件")
            return 0

        already = set()
        if os.path.isfile(archive):
            with open(archive, "r", encoding="utf-8", newline="") as f:
                for line in f:
                    m = ENTRY_MARK.match(line)
                    if m:
                        already.add(int(m.group(1)))

        now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        to_append = []
        new_blocks = list(blocks)
        for i in targets:
            _, number, entry = blocks[i]
            if number not in already:
                to_append.append(f"<!-- olp-archived-entry: {number} at {now} -->\n")
                to_append.extend(with_newline(l) for l in entry)
                if entry[-1].strip():
                    to_append.append("\n")
            stub = [with_newline(entry[0])]
            stub.extend(with_newline(l) for l in ack_lines(entry))
            stub.append(f"{ARCHIVED_MARK}{archive_name} -->\n")
            stub.append("\n")
            new_blocks[i] = ("entry", number, stub)

        # Archive first: if anything fails after this point the entries exist
        # in both places, never in neither.
        if to_append:
            new_archive = not os.path.isfile(archive) or os.path.getsize(archive) == 0
            with open(archive, "a", encoding="utf-8", newline="") as f:
                if new_archive:
                    f.write(
                        f"# 黑板归档({os.path.basename(board)})\n\n"
                        "> 由 olp-board-archive.py 追加;已闭环条目的完整原文,只供审计,不可执行。\n\n"
                    )
                elif not _ends_with_newline(archive):
                    f.write("\n")
                f.writelines(to_append)
                f.flush()
                os.fsync(f.fileno())

        fd, tmp = tempfile.mkstemp(prefix=".olp-archive-", dir=board_dir)
        try:
            with os.fdopen(fd, "w", encoding="utf-8", newline="") as f:
                for _, _, blines in new_blocks:
                    f.writelines(blines)
                f.flush()
                os.fsync(f.fileno())
            os.chmod(tmp, os.stat(board).st_mode & 0o7777)
            # Test-only: simulate an unlocked writer (an inner loop's
            # in-place ACK) landing between the read and the replace.
            injected = os.environ.get("OLP_BOARD_ARCHIVE_TEST_CONCURRENT_EDIT")
            if injected:
                with open(board, "a", encoding="utf-8", newline="") as f:
                    f.write(injected)
            if stat_key(board) != before:
                print(
                    "olp-board-archive: 归档期间板被改动,未替换黑板。"
                    "归档文件里已写入的条目下次重跑时会跳过;请在内环空闲时重跑。",
                    file=sys.stderr,
                )
                return 4
            os.replace(tmp, board)
            tmp = None
        finally:
            if tmp and os.path.exists(tmp):
                os.unlink(tmp)

        for i in targets:
            print(f"archived #{blocks[i][1]} {blocks[i][2][0].strip()[4:]}")
        print(f"olp-board-archive: {len(targets)} 个条目已归档到 {archive}")
        print(
            "olp-board-archive: 板的行数已变少;一击观测哨请重新挂。",
            file=sys.stderr,
        )
        return 0
    finally:
        if lock_fd is not None:
            os.close(lock_fd)


def _ends_with_newline(path):
    with open(path, "rb") as f:
        f.seek(-1, os.SEEK_END)
        return f.read(1) == b"\n"


if __name__ == "__main__":
    sys.exit(main())
