#!/usr/bin/env python3
"""OLP 保活:用 launchd 让单元上的常驻任务自动拉起(macOS)。

多机部署时,每台单元(Mac mini)上有几件事必须一直在跑:向 hub 报心跳、
转发负哨事件、终端工作区的后台服务。本工具把它们注册成当前用户的
launchd 任务:开机登录后自动启动,退出后自动重启。

子命令
  heartbeat --unit <单元号> --hub <ssh 目标> [--interval 60] [--scripts <目录>]
      注册心跳任务:每 --interval 秒执行一次
          olp-unit.py heartbeat --unit <单元号> | ssh <hub> receive
      hub 上这把密钥应配了强制命令(见 docs/OLP_CLUSTER.md),所以远端
      实际执行的是 `olp-unit.py receive --unit <单元号>`。
      单机部署写 --hub local:心跳直接交给本机的 `olp-unit.py receive`,不经 ssh。
  install <名字> (--interval 秒 | --keepalive) [--env K=V]... -- <命令> [参数…]
      注册任意命令。--interval 定时执行;--keepalive 常驻,退出即重启。
  uninstall <名字>
      停掉并删除一个任务。
  list
      列出本工具注册的任务,以及是否已加载。
  render <名字> (--interval 秒 | --keepalive) [--env K=V]... -- <命令> [参数…]
      只把 plist 打印到 stdout,不写文件。
  doctor
      检查这台机器的电源设置是否适合常驻(不休眠、断电恢复后自动开机),
      只报告并给出修正命令,不修改任何设置。

  install / heartbeat / uninstall / list 都可加:
    --dir <目录>   plist 所在目录(默认 ~/Library/LaunchAgents)
    --no-load      只写或删文件,不调用 launchctl

任务的标签是 org.octos.olp.<名字>,日志在 ~/.octos/outer/logs/<名字>.log。
launchd 给任务的 PATH 很短,所以默认写入
/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin,可用 --env 覆盖。

保活的边界(有意不做)
  - 只拉起基础设施。内环窗格里的对话、外环的主审权锁不会自动恢复:
    重启后由巡检上报,让大外环或人决定怎么接(协议 R7:活锁接管归
    operator)。
  - `octos serve` 在标准用法里是 TUI 的 stdio 子进程,随 TUI 启停,不单独
    注册成 launchd 任务。
  - 不改系统设置。doctor 给出的 pmset 命令需要 sudo,由人执行。

退出码:0 成功;1 launchctl 失败或 doctor 发现问题;2 用法错误,或在非
macOS 上要求加载任务。
"""

import argparse
import os
import platform
import plistlib
import re
import shlex
import shutil
import subprocess
import sys

PROG = "olp-launchd"
PREFIX = "org.octos.olp."
NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,40}$")
UNIT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
HUB_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._@-]{0,127}$")
ENV_KEY_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
DEFAULT_PATH = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def home():
    return os.path.expanduser("~")


def agents_dir(args):
    return args.dir or os.path.join(home(), "Library", "LaunchAgents")


def log_path(name):
    return os.path.join(home(), ".octos", "outer", "logs", name + ".log")


def check_name(name):
    if not NAME_RE.match(name):
        raise Fail(2, f"名字不合法: {name}(小写字母、数字、连字符,以字母或数字开头)")
    return name


def build_plist(name, command, interval, keepalive, env_pairs):
    if not command:
        raise Fail(2, "缺少要运行的命令(写在 -- 之后)")
    if keepalive == (interval is not None):
        raise Fail(2, "--interval 与 --keepalive 必须且只能给一个")
    if interval is not None and not 10 <= interval <= 86400:
        raise Fail(2, "--interval 须在 10 到 86400 秒之间")
    env = {"PATH": DEFAULT_PATH}
    for pair in env_pairs or []:
        key, sep, value = pair.partition("=")
        if not sep or not ENV_KEY_RE.match(key):
            raise Fail(2, f"--env 须写成 KEY=VALUE: {pair}")
        env[key] = value
    log = log_path(name)
    plist = {
        "Label": PREFIX + name,
        "ProgramArguments": list(command),
        "RunAtLoad": True,
        "ProcessType": "Background",
        "EnvironmentVariables": env,
        "StandardOutPath": log,
        "StandardErrorPath": log,
    }
    if keepalive:
        plist["KeepAlive"] = True
        # launchd 的默认节流也是 10 秒;写明它,崩溃循环时不至于空转。
        plist["ThrottleInterval"] = 10
    else:
        plist["StartInterval"] = interval
    return plist


def launchctl(*args):
    return subprocess.run(["launchctl", *args], capture_output=True, text=True)


def domain():
    return f"gui/{os.getuid()}"


def require_macos_for_load(args):
    if args.no_load:
        return
    if platform.system() != "Darwin":
        raise Fail(2, "launchd 只在 macOS 上可用;在别的系统上只能加 --no-load 写文件")


def write_and_load(args, name, plist):
    require_macos_for_load(args)
    directory = agents_dir(args)
    os.makedirs(directory, exist_ok=True)
    os.makedirs(os.path.dirname(log_path(name)), exist_ok=True)
    path = os.path.join(directory, PREFIX + name + ".plist")
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        plistlib.dump(plist, f, sort_keys=True)
    os.chmod(tmp, 0o644)
    os.replace(tmp, path)
    print(f"written {path}")
    if args.no_load:
        return 0
    # 已加载的同名任务先卸掉,新的配置才会生效;没加载时失败是正常的。
    launchctl("bootout", f"{domain()}/{PREFIX}{name}")
    r = launchctl("bootstrap", domain(), path)
    if r.returncode != 0:
        raise Fail(1, f"launchctl bootstrap 失败: {r.stderr.strip() or r.stdout.strip()}")
    print(f"loaded {PREFIX}{name}")
    return 0


def cmd_render(args):
    plist = build_plist(check_name(args.name), args.command, args.interval, args.keepalive, args.env)
    sys.stdout.write(plistlib.dumps(plist, sort_keys=True).decode("utf-8"))
    return 0


def cmd_install(args):
    name = check_name(args.name)
    plist = build_plist(name, args.command, args.interval, args.keepalive, args.env)
    return write_and_load(args, name, plist)


def cmd_heartbeat(args):
    if not UNIT_RE.match(args.unit):
        raise Fail(2, f"单元号不合法: {args.unit}")
    if not HUB_RE.match(args.hub):
        raise Fail(2, f"--hub 不是合法的 ssh 目标: {args.hub}")
    scripts = os.path.abspath(args.scripts or os.path.dirname(os.path.abspath(__file__)))
    tool = os.path.join(scripts, "olp-unit.py")
    if not os.path.isfile(tool):
        print(f"olp-launchd: 提醒:{tool} 现在不存在,任务运行时需要它", file=sys.stderr)
    require = args.require
    if require is not None and not re.match(r"^[a-z]+(,[a-z]+)*$", require):
        raise Fail(2, f"--require 须写成逗号分隔的巡检项,如 agent,disk: {require}")
    env_pairs = list(args.env or [])
    # The heartbeat asks herdr which inner loops run. The job's PATH is short,
    # so record where herdr is now unless the caller already said.
    if not any(p.startswith("OLP_HERDR=") for p in env_pairs):
        herdr = shutil.which("herdr")
        if herdr:
            env_pairs.append(f"OLP_HERDR={os.path.realpath(herdr)}")
        else:
            print(
                f"{PROG}: 提醒:现在的 PATH 上找不到 herdr;心跳的 agent 巡检会失败。"
                "装好后重跑,或用 --env OLP_HERDR=<路径>",
                file=sys.stderr,
            )
    pipeline = (
        f"python3 -B {shlex.quote(tool)} heartbeat --unit {shlex.quote(args.unit)}"
        + (f" --require {shlex.quote(require)}" if require else "")
        + f" | ssh -o BatchMode=yes -o ConnectTimeout=10 {shlex.quote(args.hub)} receive"
    )
    if args.hub == "local":
        # Single machine: hand the heartbeat to the hub side directly.
        pipeline = (
            f"python3 -B {shlex.quote(tool)} heartbeat --unit {shlex.quote(args.unit)}"
            + (f" --require {shlex.quote(require)}" if require else "")
            + f" | python3 -B {shlex.quote(tool)} receive --unit {shlex.quote(args.unit)}"
        )
    plist = build_plist("heartbeat", ["/bin/sh", "-c", pipeline], args.interval, False, env_pairs)
    return write_and_load(args, "heartbeat", plist)


def cmd_uninstall(args):
    name = check_name(args.name)
    require_macos_for_load(args)
    path = os.path.join(agents_dir(args), PREFIX + name + ".plist")
    if not args.no_load:
        launchctl("bootout", f"{domain()}/{PREFIX}{name}")
    if os.path.isfile(path):
        os.unlink(path)
        print(f"removed {path}")
    else:
        print(f"{path} 不存在")
    return 0


def cmd_list(args):
    directory = agents_dir(args)
    names = []
    if os.path.isdir(directory):
        for entry in sorted(os.listdir(directory)):
            if entry.startswith(PREFIX) and entry.endswith(".plist"):
                names.append(entry[len(PREFIX):-len(".plist")])
    if not names:
        print("没有本工具注册的任务")
        return 0
    can_ask = not args.no_load and platform.system() == "Darwin"
    for name in names:
        path = os.path.join(directory, PREFIX + name + ".plist")
        try:
            with open(path, "rb") as f:
                plist = plistlib.load(f)
        except Exception:
            print(f"{name}\t无法解析 {path}")
            continue
        mode = "常驻" if plist.get("KeepAlive") else f"每 {plist.get('StartInterval')} 秒"
        if can_ask:
            loaded = launchctl("print", f"{domain()}/{PREFIX}{name}").returncode == 0
            state = "已加载" if loaded else "未加载"
        else:
            state = "未查询"
        print(f"{name}\t{mode}\t{state}\t{' '.join(plist.get('ProgramArguments', []))}")
    return 0


def parse_pmset(text):
    """Settings under 'Currently in use:' (or the whole text if absent)."""
    values = {}
    active = "Currently in use:" not in text
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.endswith(":"):
            active = stripped == "Currently in use:"
            continue
        if not active:
            continue
        m = re.match(r"^(\S+)\s+(\S+)", stripped)
        if m:
            values[m.group(1)] = m.group(2)
    return values


def cmd_doctor(args):
    if args.pmset_output:
        with open(args.pmset_output, "r", encoding="utf-8") as f:
            text = f.read()
    else:
        if platform.system() != "Darwin":
            raise Fail(2, "doctor 检查的是 macOS 的电源设置;在别的系统上不适用")
        r = subprocess.run(["pmset", "-g"], capture_output=True, text=True)
        if r.returncode != 0:
            raise Fail(1, f"pmset -g 失败: {r.stderr.strip()}")
        text = r.stdout
    values = parse_pmset(text)
    problems = 0
    checks = [
        ("sleep", "0", "机器不自动休眠", "sudo pmset -a sleep 0"),
        ("autorestart", "1", "断电恢复后自动开机", "sudo pmset -a autorestart 1"),
    ]
    for key, want, meaning, fix in checks:
        got = values.get(key)
        if got == want:
            print(f"ok    {meaning}({key} = {got})")
        elif got is None:
            problems += 1
            print(f"??    {meaning}:pmset 没有列出 {key}(笔记本没有 autorestart)。修正:{fix}")
        else:
            problems += 1
            print(f"FAIL  {meaning}:{key} = {got},应为 {want}。修正:{fix}")
    print("提醒  launchd 的用户任务在登录后才启动:单元要设成开机自动登录。")
    return 1 if problems else 0


def add_job_flags(p):
    p.add_argument("--interval", type=int)
    p.add_argument("--keepalive", action="store_true")
    p.add_argument("--env", action="append")


def add_dir_flags(p):
    p.add_argument("--dir")
    p.add_argument("--no-load", action="store_true")


def main():
    ap = argparse.ArgumentParser(prog="olp-launchd.py", description="OLP 保活(launchd)")
    sub = ap.add_subparsers(dest="cmd")

    p = sub.add_parser("render")
    p.add_argument("name")
    add_job_flags(p)
    p.set_defaults(fn=cmd_render)

    p = sub.add_parser("install")
    p.add_argument("name")
    add_dir_flags(p)
    add_job_flags(p)
    p.set_defaults(fn=cmd_install)

    p = sub.add_parser("heartbeat")
    p.add_argument("--unit", required=True)
    p.add_argument("--hub", required=True)
    p.add_argument("--interval", type=int, default=60)
    p.add_argument("--scripts")
    p.add_argument("--require", help="心跳必须通过的巡检项,传给 olp-unit.py heartbeat(默认用它的默认值 agent,disk)")
    p.add_argument("--env", action="append")
    add_dir_flags(p)
    p.set_defaults(fn=cmd_heartbeat)

    p = sub.add_parser("uninstall")
    p.add_argument("name")
    add_dir_flags(p)
    p.set_defaults(fn=cmd_uninstall)

    p = sub.add_parser("list")
    add_dir_flags(p)
    p.set_defaults(fn=cmd_list)

    p = sub.add_parser("doctor")
    p.add_argument("--pmset-output", help="读这个文件而不是运行 pmset -g(测试与离线检查用)")
    p.set_defaults(fn=cmd_doctor)

    # The job's own command follows `--`; split it off before argparse so its
    # flags are never mistaken for ours.
    argv = sys.argv[1:]
    command = []
    if "--" in argv:
        cut = argv.index("--")
        argv, command = argv[:cut], argv[cut + 1:]
    try:
        args = ap.parse_args(argv)
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    if not getattr(args, "fn", None):
        ap.print_usage(sys.stderr)
        return 2
    if args.cmd in ("install", "render"):
        args.command = command
    elif command:
        print(f"olp-launchd: {args.cmd} 不接受 -- 之后的命令", file=sys.stderr)
        return 2
    try:
        return args.fn(args)
    except Fail as e:
        print(f"olp-launchd: {e}", file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main())
