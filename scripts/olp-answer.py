#!/usr/bin/env python3
"""OLP 提问的回程:大外环在 hub 上回答单元里内环的 ask_outer。

内环卡住时会调用 ask_outer 向外环提问。提问存在单元本机的信箱里
(~/.octos/outer/mcp/questions/),内环等回答文件出现在
~/.octos/outer/mcp/answers/<id>.json,等不到就按降级指引继续(默认 50 秒)。
多机部署时大外环在 hub 上:提问的审计行会出现在 hub 的黑板上(见
olp-board-remote.py 的 --sign),本工具负责把回答送回那台单元。

hub 上的子命令
  list <单元号>                 这台单元上还没回答的提问
  show <单元号> <提问id>        提问的全文(问题、上下文、已经试过什么)
  answer <单元号> <提问id>      回答从 stdin 喂,写进单元的信箱
  选项:--to <ssh 目标>(默认就是单元号——约定单元的主机名等于单元号)

单元上的子命令
  serve [--root <目录>] [--max-age 秒]
      ssh 强制命令。只接受 list、show <id>、answer <id> 三个动作,只读写
      信箱目录。--max-age 是内环等回答的时间(默认 50 秒,与 MCP 服务的
      超时一致);更早的提问不再接受回答。在单元的 authorized_keys 里给 hub 的密钥配:
        command="/path/scripts/olp-answer.py serve",no-pty,no-port-forwarding ssh-ed25519 AAAA… hub-answer

规则
  - 只能回答还在等的提问。提问不存在(已回答或写错 id)、或已经超过内环
    的等待时间时拒绝,不会在信箱里留下没人读的回答。
  - 回答上限 64 KiB,必须是 UTF-8。写入经临时文件原子替换,内环不会读到
    半个文件。
  - 内环只等 50 秒(MCP 客户端对一次工具调用的上限是 60 秒,等待时间必须
    比它短)。来不及时回答会被拒绝(提问已不在等),这时改用黑板
    条目或 herdr 直接告诉那台单元。
  - 回答的落板由单元上的 MCP 服务自己做(它读到回答后记一行 answer 审计,
    经 --sign 转发回 hub 的黑板),本工具不写黑板。

退出码:0 成功;2 用法错误;3 单元拒绝(没有这个提问、动作不允许);
75 单元不可达。
"""

import argparse
import datetime
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
import time

UNIT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
TARGET_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._@-]{0,127}$")
ID_RE = re.compile(r"^[A-Za-z0-9_-]{4,64}$")
MAX_ANSWER = 64 * 1024
EX_TEMPFAIL = 75
SSH_UNREACHABLE = 255


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


# --- hub side ------------------------------------------------------------


def target_of(args):
    if not UNIT_RE.match(args.unit):
        raise Fail(2, f"单元号不合法: {args.unit}")
    target = args.to or args.unit
    if not TARGET_RE.match(target):
        raise Fail(2, f"--to 不是合法的 ssh 目标: {target}")
    return target


def ssh_run(target, remote_args, stdin_bytes=None):
    ssh = os.environ.get("OLP_SSH") or "ssh"
    cmd = [ssh, "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", target] + remote_args
    try:
        r = subprocess.run(cmd, input=stdin_bytes, capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        return SSH_UNREACHABLE, b"", "ssh 超时"
    except OSError as e:
        return SSH_UNREACHABLE, b"", f"无法运行 ssh: {e}"
    return r.returncode, r.stdout, r.stderr.decode("utf-8", "replace")


def relay(args, remote_args, stdin_bytes=None):
    code, out, err = ssh_run(target_of(args), remote_args, stdin_bytes)
    if code == SSH_UNREACHABLE:
        print(f"olp-answer: 单元 {args.unit} 不可达:{err.strip()}", file=sys.stderr)
        return EX_TEMPFAIL
    sys.stdout.write(out.decode("utf-8", "replace"))
    if err.strip():
        print(err.strip(), file=sys.stderr)
    return code


def check_id(ask_id):
    if not ID_RE.match(ask_id):
        raise Fail(2, f"提问 id 不合法: {ask_id}")
    return ask_id


def cmd_list(args):
    return relay(args, ["list"])


def cmd_show(args):
    return relay(args, ["show", check_id(args.ask_id)])


def cmd_answer(args):
    check_id(args.ask_id)
    body = sys.stdin.buffer.read(MAX_ANSWER + 1)
    if len(body) > MAX_ANSWER:
        raise Fail(2, "回答超过 64 KiB")
    if not body.strip():
        raise Fail(2, "回答为空(从 stdin 喂)")
    return relay(args, ["answer", args.ask_id], body)


# --- unit side -----------------------------------------------------------


def mailbox(root):
    base = os.path.join(root, "mcp")
    return os.path.join(base, "questions"), os.path.join(base, "answers")


def pending(questions_dir):
    """Pending questions as {id: path}; the file name ends with -<id>.json."""
    out = {}
    if not os.path.isdir(questions_dir):
        return out
    for name in sorted(os.listdir(questions_dir)):
        if not name.endswith(".json"):
            continue
        path = os.path.join(questions_dir, name)
        try:
            with open(path, "r", encoding="utf-8") as f:
                payload = json.load(f)
        except (OSError, ValueError):
            continue
        ask_id = payload.get("id") if isinstance(payload, dict) else None
        if isinstance(ask_id, str) and ID_RE.match(ask_id):
            out[ask_id] = (path, payload)
    return out


def cmd_serve(args):
    root = args.root or os.path.join(os.path.expanduser("~"), ".octos", "outer")
    questions_dir, answers_dir = mailbox(root)
    try:
        words = shlex.split(os.environ.get("SSH_ORIGINAL_COMMAND", ""))
    except ValueError:
        words = []
    asks = pending(questions_dir)

    if words == ["list"]:
        for ask_id, (path, payload) in asks.items():
            head = str(payload.get("question", "")).replace("\n", " ")[:120]
            age = max(0.0, time.time() - os.path.getmtime(path))
            state = "等待中" if age <= args.max_age else "已超时"
            print(f"{ask_id}\t{state}\t{int(age)}s\t{head}")
        return 0
    if len(words) == 2 and words[0] in ("show", "answer") and ID_RE.match(words[1]):
        ask_id = words[1]
        if ask_id not in asks:
            raise Fail(3, f"没有在等的提问 {ask_id}(可能已回答,或写错了 id)")
        age = max(0.0, time.time() - os.path.getmtime(asks[ask_id][0]))
        if words[0] == "answer" and age > args.max_age:
            # 提问文件还在,但提问的内环早已按降级指引继续,没有人会读回答。
            raise Fail(
                3,
                f"提问 {ask_id} 已等了 {int(age)} 秒,超过内环的等待时间({int(args.max_age)} 秒);"
                "它已按降级指引继续。请改用黑板条目或 herdr 告诉这台单元",
            )
        if words[0] == "show":
            _, payload = asks[ask_id]
            # 只给提问本身的字段;信箱里不该有别的,但不原样吐文件。
            shown = {k: payload.get(k, "") for k in ("id", "ts", "question", "context", "tried")}
            json.dump(shown, sys.stdout, ensure_ascii=False, indent=2)
            sys.stdout.write("\n")
            return 0
        body = sys.stdin.buffer.read(MAX_ANSWER + 1)
        if len(body) > MAX_ANSWER:
            raise Fail(3, "回答超过 64 KiB")
        try:
            text = body.decode("utf-8").strip()
        except UnicodeDecodeError:
            raise Fail(3, "回答不是 UTF-8")
        if not text:
            raise Fail(3, "回答为空")
        os.makedirs(answers_dir, exist_ok=True)
        answer = {
            "answer": text,
            "ts": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "via": "olp-answer",
        }
        fd, tmp = tempfile.mkstemp(prefix=".answer-", dir=answers_dir)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(answer, f, ensure_ascii=False)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, os.path.join(answers_dir, ask_id + ".json"))
        print(f"answered {ask_id}")
        return 0
    raise Fail(3, "这个密钥只能用于 list、show <id>、answer <id>")


def main():
    ap = argparse.ArgumentParser(prog="olp-answer.py", description="OLP 提问的回程")
    sub = ap.add_subparsers(dest="cmd")
    for name, fn, with_id in (("list", cmd_list, False), ("show", cmd_show, True), ("answer", cmd_answer, True)):
        p = sub.add_parser(name)
        p.add_argument("unit")
        if with_id:
            p.add_argument("ask_id")
        p.add_argument("--to")
        p.set_defaults(fn=fn)
    p = sub.add_parser("serve")
    p.add_argument("--root")
    p.add_argument("--max-age", type=float, default=50.0)
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
        print(f"olp-answer: {e}", file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main())
