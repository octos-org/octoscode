#!/usr/bin/env python3
"""olp-outer.py — 外环挂了自动拉起(R7:锁空出时由指定的外环重新 hold)。

外环跑在 herdr 的一个窗格里,经 `octoscode outer-duty hold` 持有项目的主审权锁。
外环死了锁就空出(VACANT)。监督任务每隔一段时间检查一次:锁空着,就在外环的
窗格里重新执行 `hold … -- <外环命令>`;锁被占着,什么都不做——所以它永远不会
造出第二个外环,也不会接管一把还活着的锁(那仍归 operator)。

用法
  octoscode olp outer supervise --project P --signature S --duties D
        [--label olp-outer] [--cwd 目录] [--session herdr 会话] [--board 黑板]
        [--max-restarts 3] [--window 1800] [--wait 20] -- <外环命令…>
      检查一次。锁 HELD:输出 held,退出码 0。锁 VACANT:找到 label 为 --label 的
      herdr 工作区(没有就新建,工作目录 --cwd,默认项目目录),确认它的窗格里
      没有还在运行的 agent,在窗格里执行 hold,等锁变成 HELD(最多 --wait 秒)并
      保持 --settle 秒(默认 2;一拿到锁就挂的不算重启成功)。
      给了 --board 时在黑板上记一行。--window 秒内已经重启了 --max-restarts
      次就不再重启(外环一起来就挂,多半要人看),在黑板上告警一次,退出码 4;
      `--reset` 清掉重启记录。
  octoscode olp outer install [--interval 60] [--name outer] <supervise 的参数> -- <外环命令…>
      把 supervise 注册成常驻任务(macOS launchd、Linux systemd),每 --interval
      秒运行一次。任务里写的是 octoscode 的绝对路径;herdr 的路径记进环境变量
      OLP_HERDR(任务的 PATH 很短)。--no-load、--dir 原样交给保活工具。

外环命令
  要能接着上次的会话,例如 `claude --continue …`、`codex resume --last --no-daemon …`。
  Codex 必须带 --no-daemon(hold 会拒绝不带的)。

herdr 不在 PATH 上时用环境变量 OLP_HERDR 指定。herdr 服务要由人或常驻任务启动;
supervise 找不到 herdr 服务时什么都不做,退出码 1。

退出码:0 锁 HELD(本来就是,或已重启成功);1 出错(锁读不出、herdr 不可用、
重启后锁仍空着);2 用法错误;3 外环窗格里还有程序在跑,未重启;4 重启过于频繁,已暂停。
"""

import argparse
import datetime
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
PROG = "olp-outer"
SHELLS = {"zsh", "bash", "sh", "fish", "dash", "ksh", "tcsh", "csh", "nu"}
# Short-lived helpers a shell runs while it starts (prompt themes, git status).
SHELL_HELPERS = {"git", "sed", "awk", "uname", "tput", "stty", "dircolors", "locale", "basename", "dirname"}


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def cli_lib():
    import importlib.util

    spec = importlib.util.spec_from_file_location("olp_cli_lib", os.path.join(HERE, "olp-cli-lib.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def octoscode():
    exe = cli_lib().octoscode_bin() or shutil.which("octoscode")
    if not exe:
        raise Fail(1, "找不到 octoscode(经 octoscode olp 运行,或把它放进 PATH)")
    return exe


def herdr(args, session):
    exe = os.environ.get("OLP_HERDR") or shutil.which("herdr") or "herdr"
    cmd = [exe] + (["--session", session] if session else []) + list(args)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Fail(1, f"herdr 不可用: {exc}")
    try:
        data = json.loads(r.stdout or "{}")
    except ValueError:
        data = {}
    if r.returncode != 0 or "error" in data:
        err = (data.get("error") or {}).get("message") or r.stderr.strip() or r.stdout.strip()
        raise Fail(1, f"herdr {' '.join(args[:2])} 失败: {err}")
    return data.get("result", {})


def duty_state(project):
    r = subprocess.run(
        [octoscode(), "outer-duty", "check", "--project", project], capture_output=True, text=True
    )
    return r.stdout.strip() or "ERROR"


def state_path(project):
    digest = hashlib.sha256(os.path.realpath(project).encode()).hexdigest()[:16]
    root = os.environ.get("OLP_OUTER_STATE") or os.path.join(os.path.expanduser("~"), ".octos", "outer", "supervise")
    return os.path.join(root, digest + ".json")


def load_state(project):
    try:
        with open(state_path(project), encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def save_state(project, data):
    path = state_path(project)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f)
    os.replace(tmp, path)


def board_note(board, text):
    if not board:
        return
    append = os.path.join(HERE, "olp-board-append.sh")
    env = {k: v for k, v in os.environ.items() if not k.startswith("OLP_DUTY_")}
    r = subprocess.run(["bash", append, board], input=text, text=True, capture_output=True, env=env)
    if r.returncode != 0:
        print(f"{PROG}: 提醒:没写进黑板: {r.stderr.strip()}", file=sys.stderr)


def find_pane(args):
    """The outer's pane: the first pane of the workspace labelled --label."""
    workspaces = herdr(["workspace", "list"], args.session).get("workspaces", [])
    match = [w for w in workspaces if w.get("label") == args.label]
    if not match:
        created = herdr(
            ["workspace", "create", "--cwd", args.cwd or args.project, "--label", args.label, "--no-focus"],
            args.session,
        )
        return created["root_pane"]["pane_id"], True
    wid = match[0]["workspace_id"]
    panes = [p for p in herdr(["pane", "list"], args.session).get("panes", []) if p.get("workspace_id") == wid]
    if not panes:
        raise Fail(1, f"工作区 {args.label} 里没有窗格")
    return panes[0]["pane_id"], False


def busy_programs(pane, session):
    """Programs in the pane's foreground other than its shell (and the
    helpers a shell runs while starting)."""
    info = herdr(["pane", "process-info", "--pane", pane], session).get("process_info", {})
    names = []
    for proc in info.get("foreground_processes", []):
        name = os.path.basename(str(proc.get("name") or proc.get("argv0") or "")).lstrip("-")
        if name and name not in SHELLS and name not in SHELL_HELPERS:
            names.append(name)
    return names


def stamp():
    return datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")


def cmd_supervise(args):
    if not args.command:
        raise Fail(2, "缺少外环命令(写在 -- 之后)")
    args.project = os.path.abspath(args.project)
    if not os.path.isdir(args.project):
        raise Fail(2, f"项目目录不存在: {args.project}")
    if args.reset:
        save_state(args.project, {})
    state = duty_state(args.project)
    if state == "HELD":
        print("held")
        return 0
    if state != "VACANT":
        raise Fail(1, f"读不出锁的状态({state})")

    record = load_state(args.project)
    now = time.time()
    recent = [t for t in record.get("restarts", []) if now - t < args.window]
    if len(recent) >= args.max_restarts:
        if not record.get("paused_noted"):
            board_note(
                args.board,
                f"> 外环自动重启已暂停({args.signature}):{args.window // 60} 分钟内重启了 "
                f"{len(recent)} 次,外环一起来就挂,需要人看。{stamp()}\n",
            )
            record["paused_noted"] = True
            save_state(args.project, {**record, "restarts": recent})
        print(f"paused: {len(recent)} restarts in {args.window}s", file=sys.stderr)
        return 4

    pane, created = find_pane(args)
    busy = busy_programs(pane, args.session)
    if busy and not created:
        print(f"{PROG}: 外环窗格 {pane} 里还有程序在跑({', '.join(busy)}),锁却空着;未重启", file=sys.stderr)
        return 3

    hold = [
        octoscode(), "outer-duty", "hold",
        "--project", args.project, "--signature", args.signature, "--duties", args.duties,
        "--", *args.command,
    ]
    herdr(["pane", "run", pane, shlex.join(hold)], args.session)
    save_state(args.project, {"restarts": recent + [now], "last_pane": pane})

    deadline = time.time() + args.wait
    while time.time() < deadline:
        if duty_state(args.project) == "HELD" and stays_held(args.project, args.settle):
            board_note(
                args.board,
                f"> 外环已自动重启({args.signature}):锁空出,已在 herdr 窗格 {pane} 重新 hold。{stamp()}\n",
            )
            print(f"restarted in pane {pane}")
            return 0
        time.sleep(0.5)
    raise Fail(1, f"已在窗格 {pane} 执行 hold,{args.wait} 秒内锁仍是空的(或拿到后又放掉了)")


def stays_held(project, settle):
    """HELD for `settle` seconds: an outer that dies right after taking the
    lock does not count as restarted."""
    end = time.time() + settle
    while time.time() < end:
        time.sleep(0.25)
        if duty_state(project) != "HELD":
            return False
    return True


def keepalive_tool():
    if sys.platform == "darwin":
        return os.path.join(HERE, "olp-launchd.py")
    if sys.platform.startswith("linux"):
        return os.path.join(HERE, "olp-systemd.py")
    raise Fail(2, "常驻任务只支持 macOS(launchd)与 Linux(systemd)")


def cmd_install(args):
    if not args.command:
        raise Fail(2, "缺少外环命令(写在 -- 之后)")
    project = os.path.abspath(args.project)
    supervise = [
        octoscode(), "olp", "outer", "supervise",
        "--project", project, "--signature", args.signature, "--duties", args.duties,
        "--label", args.label,
    ]
    for flag, value in (("--cwd", args.cwd), ("--session", args.session), ("--board", args.board)):
        if value:
            supervise += [flag, os.path.abspath(value) if flag != "--session" else value]
    supervise += [
        "--max-restarts", str(args.max_restarts), "--window", str(args.window),
        "--wait", str(args.wait), "--settle", str(args.settle), "--", *args.command,
    ]
    tool = [sys.executable, "-B", keepalive_tool(), "install", args.name, "--interval", str(args.interval)]
    herdr_path = os.environ.get("OLP_HERDR") or shutil.which("herdr")
    if herdr_path:
        tool += ["--env", f"OLP_HERDR={os.path.realpath(herdr_path)}"]
    else:
        print(f"{PROG}: 提醒:PATH 上找不到 herdr;装好后重跑,或设 OLP_HERDR", file=sys.stderr)
    if args.no_load:
        tool.append("--no-load")
    if args.dir:
        tool += ["--dir", args.dir]
    r = subprocess.run(tool + ["--", *supervise])
    return r.returncode


def common(p):
    p.add_argument("--project", required=True)
    p.add_argument("--signature", required=True)
    p.add_argument("--duties", required=True)
    p.add_argument("--label", default="olp-outer", help="外环所在 herdr 工作区的 label(默认 olp-outer)")
    p.add_argument("--cwd", help="新建工作区时的工作目录(默认项目目录)")
    p.add_argument("--session", help="herdr 会话名(默认不指定)")
    p.add_argument("--board", help="黑板;给了就记下每次自动重启")
    p.add_argument("--max-restarts", type=int, default=3)
    p.add_argument("--window", type=int, default=1800, help="秒")
    p.add_argument("--wait", type=int, default=20, help="执行 hold 后等锁 HELD 的秒数")
    p.add_argument("--settle", type=float, default=2.0, help="锁 HELD 后再观察的秒数,期间一直 HELD 才算重启成功")


def main():
    argv = sys.argv[1:]
    command = []
    if "--" in argv:
        i = argv.index("--")
        argv, command = argv[:i], argv[i + 1 :]
    ap = argparse.ArgumentParser(prog="octoscode olp outer", description="外环挂了自动拉起")
    sub = ap.add_subparsers(dest="action", required=True)
    s = sub.add_parser("supervise", help="检查一次,锁空着就在外环窗格里重新 hold")
    common(s)
    s.add_argument("--reset", action="store_true", help="清掉重启记录(暂停后人看过再继续)")
    s.set_defaults(func=cmd_supervise)
    i = sub.add_parser("install", help="把 supervise 注册成常驻任务")
    common(i)
    i.add_argument("--interval", type=int, default=60)
    i.add_argument("--name", default="outer", help="常驻任务名(默认 outer)")
    i.add_argument("--no-load", action="store_true")
    i.add_argument("--dir")
    i.set_defaults(func=cmd_install)
    try:
        args = ap.parse_args(argv)
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    args.command = command
    try:
        return args.func(args)
    except Fail as exc:
        print(f"{PROG}: {exc}", file=sys.stderr)
        return exc.code


if __name__ == "__main__":
    sys.exit(main())
