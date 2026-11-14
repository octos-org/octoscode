#!/usr/bin/env python3
"""OLP 远程写板:单元经 ssh 把内容追加到 hub 的权威黑板,断网时排队重发。

多机部署时黑板只在 hub 上。单元(mini)不能直接写那个文件,也不该把
黑板放到网络盘上(网络文件系统上的 flock 不可靠)。本工具让单元的写入
在 hub 本机、在 flock 下完成,并在 hub 不可达时不丢内容。

单元上的子命令
  append [--key <键>] [--sign]   正文从 stdin 喂,追加到 hub 的黑板;--sign
                                 给每一行加上 `> 单元(<单元号>)·` 署名
  ack <N> done|wontdo|blocked <说明>
                                 追加一行 `> 内环(<单元号>)·#N ACK(…): <说明>`
  flush                          把排队的写入按原顺序重发
  queue                          列出排队中的写入
  mirror [--to <本地路径>]       把 hub 的黑板拉成本机的只读镜像;不给 --to 时
                                 用配置里的镜像路径

hub 上的子命令
  serve --unit <单元号> --board <黑板路径>
      ssh 强制命令。只接受 `append <键>` 与 `read` 两个动作,黑板路径由
      这一行决定。在 hub 的 authorized_keys 里给每个单元的密钥配:
        command="/path/scripts/olp-board-remote.py serve --unit m07 --board /path/OUTER_LOOP_REVIEW.md",no-pty,no-port-forwarding ssh-ed25519 AAAA…

单元的配置(命令行 > 环境变量 > 配置文件)
  hub 的 ssh 目标   --hub    OLP_HUB       hub=…
  本单元号          --unit   OLP_UNIT_ID   unit=…
  镜像路径(mirror)  --to     OLP_MIRROR    mirror=…(绝对路径)
  配置文件 ${OLP_UNIT_CONF:-~/.octos/outer/unit.conf},每行 `键=值`。

幂等与排队
  - 每次写入都带一个键 `<单元号>:<…>`。不给 --key 时用正文的 sha256 前
    16 位,所以同一段正文重发不会在板上出现两次(hub 侧按键去重)。
  - ssh 连不上(退出码 255)或超时:写入存进
    ${OLP_QUEUE_DIR:-~/.octos/outer/queue},命令以退出码 75 结束。
  - 每次 append/ack 先重发队列里的旧写入,保持顺序;旧的发不出去,新的
    也进队列。`flush` 可以单独调用(例如每分钟一次)。
  - hub 明确拒绝的写入(退出码不是 255)不进队列:重发也不会成功。

hub 侧对单元写入的限制(serve)
  - 键必须以 `<单元号>:` 开头——单元之间不会互相撞键或冒名去重。
  - 正文里不能有以 `外环(` 署名开头的行(剥掉 `> `、`**` 之后判断):
    采认、打回、改判、搁置只能由 hub 上的外环写。否则单元能给自己签发
    采认。
  - 正文里不能有编号标题(`### <数字>`、`### {N}`):立卡只在大外环。
  - 正文上限 256 KiB。

只读镜像
  内环**每次读板前先执行 `mirror`**:派卡之后大外环马上唤醒内环,定时同步
  赶不上,镜像里可能还没有那张新卡。配置里写了 `mirror=` 之后这是一条不带
  参数的命令。定时同步(每分钟一次)只作兜底,让人和别的工具看到的镜像不
  太旧。
  `mirror` 把黑板拉到本机并设为只读,经临时文件原子替换。内环照常读这个
  镜像;写入一律走 append/ack。镜像永不回写。

把 MCP 提问送到 hub
  内环的 ask_outer / report_blocked 会调用 ~/.octos/outer/board_append.sh
  记一行审计。在单元上把它写成:
      #!/bin/sh
      exec python3 /path/scripts/olp-board-remote.py append --sign
  这些行就会带着单元的署名出现在 hub 的黑板上(多余的参数被忽略):
      > 单元(m07)·- 2026-11-13 03:00:00 MCP(ask_outer) ask: id=ab12 question=…
  大外环用 scripts/olp-answer.py 把回答送回单元本机的信箱。

退出码:0 成功;2 用法或配置错误;3 hub 拒绝(serve 的规则);75 hub
不可达,已排队;其余为 hub 上写板脚本的退出码。
"""

import argparse
import hashlib
import os
import re
import shlex
import stat
import subprocess
import sys
import tempfile
import time

UNIT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
HUB_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._@-]{0,127}$")
KEY_BODY = r"[A-Za-z0-9._:/@+=-]{1,180}"
KEY_RE = re.compile(r"^" + KEY_BODY + r"$")
NUMBERED_HEADING = re.compile(r"^### (\d|\{N\})")
MAX_BODY = 256 * 1024
EX_TEMPFAIL = 75
SSH_UNREACHABLE = 255


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


# --- unit-side configuration --------------------------------------------


def load_conf():
    path = os.environ.get("OLP_UNIT_CONF") or os.path.join(
        os.path.expanduser("~"), ".octos", "outer", "unit.conf"
    )
    conf = {}
    if os.path.isfile(path):
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, _, value = line.partition("=")
                conf[key.strip()] = value.strip()
    return conf


def resolve(args):
    conf = load_conf()
    hub = getattr(args, "hub", None) or os.environ.get("OLP_HUB") or conf.get("hub") or ""
    unit = getattr(args, "unit", None) or os.environ.get("OLP_UNIT_ID") or conf.get("unit") or ""
    if not HUB_RE.match(hub):
        raise Fail(2, "没有配置 hub 的 ssh 目标(--hub、OLP_HUB 或 unit.conf 里的 hub=)")
    if not UNIT_RE.match(unit):
        raise Fail(2, "没有配置本单元号(--unit、OLP_UNIT_ID 或 unit.conf 里的 unit=)")
    return hub, unit


def queue_dir():
    return os.environ.get("OLP_QUEUE_DIR") or os.path.join(
        os.path.expanduser("~"), ".octos", "outer", "queue"
    )


def ssh_run(hub, remote_args, stdin_bytes=None):
    """Run the hub side. Returns (returncode, stdout bytes, stderr text)."""
    ssh = os.environ.get("OLP_SSH") or "ssh"
    cmd = [ssh, "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", hub] + remote_args
    try:
        r = subprocess.run(cmd, input=stdin_bytes, capture_output=True, timeout=60)
    except subprocess.TimeoutExpired:
        return SSH_UNREACHABLE, b"", "ssh 超时"
    except OSError as e:
        return SSH_UNREACHABLE, b"", f"无法运行 ssh: {e}"
    return r.returncode, r.stdout, r.stderr.decode("utf-8", "replace")


# --- unit: queue ---------------------------------------------------------


def queued():
    """Queued writes in send order: [(path, key, body bytes)]."""
    directory = queue_dir()
    out = []
    if not os.path.isdir(directory):
        return out
    for name in sorted(os.listdir(directory)):
        if not name.endswith(".entry"):
            continue
        path = os.path.join(directory, name)
        try:
            with open(path, "rb") as f:
                key_line = f.readline().decode("utf-8", "replace").rstrip("\n")
                body = f.read()
        except OSError:
            continue
        if KEY_RE.match(key_line):
            out.append((path, key_line, body))
    return out


def enqueue(key, body):
    directory = queue_dir()
    os.makedirs(directory, exist_ok=True)
    for _, queued_key, _ in queued():
        if queued_key == key:
            return  # 同一个写入已经在队列里
    # 时间戳加序号保证顺序;同一纳秒内不会有两个写入方(单元上只有一个内环)。
    name = f"{time.time_ns():020d}-{hashlib.sha256(key.encode()).hexdigest()[:12]}.entry"
    fd, tmp = tempfile.mkstemp(prefix=".q-", dir=directory)
    with os.fdopen(fd, "wb") as f:
        f.write(key.encode("utf-8") + b"\n" + body)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, os.path.join(directory, name))


def send(hub, key, body):
    """Return (code, stdout, stderr) of one append on the hub."""
    return ssh_run(hub, ["append", key], body)


def flush_queue(hub):
    """Resend queued writes in order. Returns (sent, remaining, unreachable)."""
    sent = 0
    items = queued()
    for index, (path, key, body) in enumerate(items):
        code, _, err = send(hub, key, body)
        if code == SSH_UNREACHABLE:
            return sent, len(items) - index, True
        if code != 0:
            # hub 明确拒绝:留着也发不出去,挪到一边不挡后面的写入。
            os.replace(path, path[: -len(".entry")] + ".rejected")
            print(f"olp-board-remote: 排队的写入 {key} 被 hub 拒绝(退出码 {code}):{err.strip()}", file=sys.stderr)
            continue
        os.unlink(path)
        sent += 1
    return sent, 0, False


def deliver(hub, key, body):
    _, remaining, unreachable = flush_queue(hub)
    if unreachable:
        enqueue(key, body)
        print(f"olp-board-remote: hub 不可达,已排队(队列里共 {remaining + 1} 条)", file=sys.stderr)
        return EX_TEMPFAIL
    code, out, err = send(hub, key, body)
    if code == SSH_UNREACHABLE:
        enqueue(key, body)
        print("olp-board-remote: hub 不可达,已排队(队列里共 1 条)", file=sys.stderr)
        return EX_TEMPFAIL
    sys.stdout.write(out.decode("utf-8", "replace"))
    if err.strip():
        print(err.strip(), file=sys.stderr)
    return code


def make_key(unit, key, body):
    if key:
        if not KEY_RE.match(key):
            raise Fail(2, "--key 只允许 [A-Za-z0-9._:/@+=-],最长 180")
        suffix = key
    else:
        suffix = hashlib.sha256(body).hexdigest()[:16]
    full = suffix if suffix.startswith(unit + ":") else f"{unit}:{suffix}"
    if not KEY_RE.match(full):
        raise Fail(2, "--key 太长")
    return full


def cmd_append(args):
    hub, unit = resolve(args)
    body = sys.stdin.buffer.read(MAX_BODY + 1)
    if len(body) > MAX_BODY:
        raise Fail(2, "正文超过 256 KiB")
    if not body.strip():
        raise Fail(2, "正文为空")
    if args.sign:
        # Lines that carry no signature of their own (the MCP server's audit
        # entries) get the unit's, so the hub can tell whose they are.
        text = body.decode("utf-8", "replace")
        signed = [f"> 单元({unit})·{line}" for line in text.splitlines() if line.strip()]
        body = ("\n".join(signed) + "\n").encode("utf-8")
    return deliver(hub, make_key(unit, args.key, body), body)


def cmd_ack(args):
    hub, unit = resolve(args)
    text = " ".join(args.text).strip()
    if not text:
        raise Fail(2, "ACK 需要说明(done 附 commit/测试证据,wontdo 附理由,blocked 附原因)")
    if "\n" in text or "\r" in text:
        raise Fail(2, "ACK 说明必须是单行")
    line = f"> 内环({unit})·#{args.number} ACK({args.kind}): {text}\n"
    body = line.encode("utf-8")
    return deliver(hub, make_key(unit, None, body), body)


def cmd_flush(args):
    hub, _ = resolve(args)
    sent, remaining, unreachable = flush_queue(hub)
    print(f"已重发 {sent} 条,队列里还有 {remaining} 条")
    return EX_TEMPFAIL if unreachable else 0


def cmd_queue(args):
    items = queued()
    if not items:
        print("队列为空")
        return 0
    for _, key, body in items:
        first = body.decode("utf-8", "replace").splitlines()[0][:80] if body else ""
        print(f"{key}\t{first}")
    return 0


def mirror_target(args):
    """--to, then OLP_MIRROR, then `mirror=` in unit.conf. There is no
    guessed default: writing a read-only copy over the wrong file — say, the
    live board of whatever repository the command happens to run in — is
    worse than a usage error."""
    target = args.to or os.environ.get("OLP_MIRROR") or load_conf().get("mirror") or ""
    if not target:
        raise Fail(2, "没有镜像路径(--to、OLP_MIRROR 或 unit.conf 里的 mirror=)")
    if not os.path.isabs(target):
        if args.to:
            return os.path.abspath(target)
        raise Fail(2, f"配置里的镜像路径必须是绝对路径: {target}")
    return target


def cmd_mirror(args):
    target = mirror_target(args)
    hub, _ = resolve(args)
    code, out, err = ssh_run(hub, ["read"])
    if code == SSH_UNREACHABLE:
        print("olp-board-remote: hub 不可达,镜像保持不变", file=sys.stderr)
        return EX_TEMPFAIL
    if code != 0:
        raise Fail(code, f"hub 拒绝读取:{err.strip()}")
    directory = os.path.dirname(target)
    os.makedirs(directory, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".mirror-", dir=directory)
    with os.fdopen(fd, "wb") as f:
        f.write(out)
    os.chmod(tmp, stat.S_IRUSR | stat.S_IRGRP | stat.S_IROTH)
    os.replace(tmp, target)
    print(f"mirrored {len(out)} bytes to {target}")
    return 0


# --- hub: serve ----------------------------------------------------------


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


def cmd_serve(args):
    if not UNIT_RE.match(args.unit):
        raise Fail(2, f"单元号不合法: {args.unit}")
    board = os.path.abspath(args.board)
    try:
        words = shlex.split(os.environ.get("SSH_ORIGINAL_COMMAND", ""))
    except ValueError:
        words = []
    if words == ["read"]:
        if not os.path.isfile(board):
            raise Fail(3, "黑板不存在")
        with open(board, "rb") as f:
            sys.stdout.buffer.write(f.read())
        return 0
    if len(words) != 2 or words[0] != "append":
        raise Fail(3, "这个密钥只能用于 append <键> 与 read")
    key = words[1]
    if not re.match(r"^" + re.escape(args.unit) + r":" + KEY_BODY + r"$", key) or len(key) > 200:
        raise Fail(3, f"键必须以 {args.unit}: 开头且只含 [A-Za-z0-9._:/@+=-]")
    body = sys.stdin.buffer.read(MAX_BODY + 1)
    if len(body) > MAX_BODY:
        raise Fail(3, "正文超过 256 KiB")
    if not body.strip():
        raise Fail(3, "正文为空")
    try:
        text = body.decode("utf-8")
    except UnicodeDecodeError:
        raise Fail(3, "正文不是 UTF-8")
    for line in text.splitlines():
        if strip_quote(line).startswith("外环("):
            raise Fail(3, "单元不能写外环署名的行(采认、打回、改判、搁置只在 hub 上写)")
        if NUMBERED_HEADING.match(line):
            raise Fail(3, "单元不能写编号条目(立卡只在大外环)")
    append = os.path.join(os.path.dirname(os.path.abspath(__file__)), "olp-board-append.sh")
    r = subprocess.run(["bash", append, "--key", key, board], input=body)
    return r.returncode


def main():
    ap = argparse.ArgumentParser(prog="olp-board-remote.py", description="OLP 远程写板")
    sub = ap.add_subparsers(dest="cmd")

    def unit_flags(p):
        p.add_argument("--hub")
        p.add_argument("--unit")

    p = sub.add_parser("append")
    unit_flags(p)
    p.add_argument("--key")
    p.add_argument("--sign", action="store_true", help="给每一行加上 `> 单元(<单元号>)·` 署名")
    p.add_argument("ignored", nargs="*", help=argparse.SUPPRESS)
    p.set_defaults(fn=cmd_append)

    p = sub.add_parser("ack")
    unit_flags(p)
    p.add_argument("number", type=int)
    p.add_argument("kind", choices=["done", "wontdo", "blocked"])
    p.add_argument("text", nargs="+")
    p.set_defaults(fn=cmd_ack)

    p = sub.add_parser("flush")
    unit_flags(p)
    p.set_defaults(fn=cmd_flush)

    p = sub.add_parser("queue")
    p.set_defaults(fn=cmd_queue)

    p = sub.add_parser("mirror")
    unit_flags(p)
    p.add_argument("--to")
    p.set_defaults(fn=cmd_mirror)

    p = sub.add_parser("serve")
    p.add_argument("--unit", required=True)
    p.add_argument("--board", required=True)
    p.set_defaults(fn=cmd_serve)

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
        print(f"olp-board-remote: {e}", file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main())
