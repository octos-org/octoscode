#!/usr/bin/env python3
"""olp-doctor.py — 检查各处写下的 OctoLoop 命令还能不能用。

git 钩子、单元工作仓库 hub 远程的 receivepack/uploadpack、保活任务、ssh
强制命令里都写着一条以后才执行的命令。旧版本写的是脚本的绝对路径;经
`octoscode olp` 安装时,这个路径落在 ~/.octos/outer/olp-bundle/<哈希>/ 下:
升级 octoscode 后它仍跑旧代码,旧目录被清掉后直接失败。新版本写
`<octoscode 绝对路径> olp …`,升级后自动用新版本。

用法
  octoscode olp doctor [--repo <裸仓库>]... [--work <工作仓库>]... [--remote hub] [--json]

检查
  - ~/.ssh/authorized_keys 里调用 OctoLoop 的强制命令;
  - hub 裸仓库的钩子:--repo 给出的,以及强制命令里 `--repo` 提到的;
  - 工作仓库 hub 远程的本机推送设置(单机部署):--work 给出的与当前目录所在
    的仓库;没有这项设置(多机部署走 ssh)时跳过;
  - 本机的保活任务(launchd:~/Library/LaunchAgents/org.octos.olp.*.plist;
    systemd:~/.config/systemd/user/olp-*.service);
  - ~/.octos/outer/octoscode-home(skill 读规程正文的位置);
  - 工作仓库的 hub 远程写的是本机路径时,这台机器上有没有那个裸仓库(照抄了 hub
    上的路径就没有;ssh 地址不联网检查);
  - 本机还在跑已被替换的 octoscode / octos / herdr 的进程(升级后要重启才用上新版);
  - hub 上各单元最近一次心跳是不是旧格式(缺 agent、full 巡检或 agents 字段)。

每项给出 ok / warn / fail;warn 与 fail 附修复命令。
  ok    调用 octoscode 的绝对路径,或仓库里的脚本(随 git pull 更新)
  warn  固定在某一版本的工具目录;或按 PATH 找 octoscode(这些地方的 PATH 很短)
  fail  写下的路径已不存在;钩子缺失;命令无法解析

退出码:0 没有 fail;1 有 fail;2 用法错误。
"""

import argparse
import json
import os
import plistlib
import re
import shlex
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT_RE = re.compile(r"^olp-[a-z-]+\.(py|sh)$")
HOOKS = ("pre-receive", "post-receive")
LAUNCHD_PREFIX = "org.octos.olp."
SYSTEMD_PREFIX = "olp-"
SYSTEMD_COMMAND_MARK = "# olp-command: "
# A script and its subcommand -> the same thing under `octoscode olp`.
OLP_WORDS = {
    ("olp-unit.py", "receive"): ["unit", "receive"],
    ("olp-unit.py", "heartbeat"): ["unit", "heartbeat"],
    ("olp-git-hub.py", "serve"): ["repo", "serve"],
    ("olp-git-hub.py", "hook"): ["repo", "hook"],
    ("olp-git-hub.py", "local-pack"): ["repo", "local-pack"],
    ("olp-board-remote.py", "serve"): ["remote", "serve"],
    ("olp-answer.py", "serve"): ["ask", "serve"],
}
RANK = {"ok": 0, "warn": 1, "fail": 2}


def cli_lib():
    import importlib.util

    spec = importlib.util.spec_from_file_location("olp_cli_lib", os.path.join(HERE, "olp-cli-lib.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CLI = cli_lib()


def octoscode_for_fixes():
    """The octoscode to name in suggested forced commands."""
    return CLI.octoscode_bin() or shutil.which("octoscode") or "/path/to/octoscode"


def calls(tokens):
    """Where a command line calls an OctoLoop tool: [(kind, path, index)]."""
    found = []
    for i, token in enumerate(tokens):
        base = os.path.basename(token)
        if base == "octoscode" and i + 1 < len(tokens) and tokens[i + 1] == "olp":
            found.append(("cli", token, i))
        elif SCRIPT_RE.match(base):
            found.append(("script", token, i))
    return found


def bundle_roots():
    """Directories that hold one tool directory per octoscode version."""
    roots = [os.path.join(os.path.expanduser("~"), ".octos", "outer", "olp-bundle")]
    if os.environ.get("OCTOSCODE_OLP_BUNDLE"):
        roots.append(os.environ["OCTOSCODE_OLP_BUNDLE"])
    # Running from a bundle: <root>/<hash>/scripts/olp-doctor.py.
    if os.path.isfile(os.path.join(os.path.dirname(HERE), ".complete")):
        roots.append(os.path.dirname(os.path.dirname(HERE)))
    # Both spellings of each root; the checked path keeps its links, so a
    # path through `current` stays recognisable.
    return sorted({form for r in roots for form in (os.path.abspath(r), os.path.realpath(r))})


ROOTS = bundle_roots()


def pinned(path):
    """A path inside one version's tool directory (not the `current` link)."""
    path = os.path.abspath(path)
    for root in ROOTS:
        if path.startswith(root + os.sep):
            return path[len(root) + 1 :].split(os.sep)[0] != "current"
    return "/olp-bundle/" in path and "/olp-bundle/current/" not in path


def judge(tokens):
    """(status, detail) for one written command; None when it calls no tool."""
    found = calls(tokens)
    if not found:
        return None
    status, notes = "ok", []
    for kind, path, _ in found:
        if not os.path.isabs(path):
            if kind == "cli":
                status = max(status, "warn", key=RANK.get)
                notes.append("按 PATH 找 octoscode;钩子、保活任务、ssh 强制命令的 PATH 很短,写绝对路径更稳")
            continue
        if not os.path.exists(path):
            status = "fail"
            notes.append(f"不存在:{path}")
        elif kind == "script" and pinned(path):
            status = max(status, "warn", key=RANK.get)
            notes.append(f"固定在某一版本的工具目录,升级 octoscode 后仍跑旧代码:{path}")
        elif kind == "script":
            notes.append(f"调用仓库里的脚本(随 git pull 更新):{path}")
        else:
            notes.append(f"调用 {path} olp")
    return status, ";".join(notes)


def option(tokens, name):
    """The value after `name` in a token list, or None."""
    for i, token in enumerate(tokens[:-1]):
        if token == name:
            return tokens[i + 1]
    return None


def as_olp(tokens):
    """The same command written as `<octoscode> olp …`, or None."""
    for kind, path, i in calls(tokens):
        if kind == "cli":
            continue
        words = OLP_WORDS.get((os.path.basename(path), tokens[i + 1] if i + 1 < len(tokens) else ""))
        if words:
            return CLI.shell([octoscode_for_fixes(), "olp", *words, *tokens[i + 2 :]])
    return None


class Report:
    def __init__(self):
        self.items = []

    def add(self, kind, where, status, detail, fix=None):
        self.items.append(
            {"kind": kind, "where": where, "status": status, "detail": detail, "fix": fix if status != "ok" else None}
        )


def split(report, kind, where, line, fix):
    try:
        tokens = shlex.split(line)
    except ValueError:
        report.add(kind, where, "fail", f"命令无法解析:{line}", fix)
        return None
    verdict = judge(tokens)
    if verdict:
        report.add(kind, where, verdict[0], verdict[1], fix(tokens) if callable(fix) else fix)
    return tokens


# --- ssh forced commands -------------------------------------------------

FORCED = re.compile(r'(?:^|,)command="((?:[^"\\]|\\.)*)"')


def check_authorized_keys(report, path):
    """Returns the hub repositories that forced commands serve."""
    repos = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError:
        return repos
    for number, line in enumerate(lines, 1):
        m = FORCED.search(line)
        if not m or line.lstrip().startswith("#"):
            continue
        command = m.group(1).replace('\\"', '"')
        fix = lambda tokens: (  # noqa: E731
            f'把这一行的强制命令改成 command="{as_olp(tokens)}"' if as_olp(tokens) else None
        )
        tokens = split(report, "ssh 强制命令", f"{path}:{number}", command, fix)
        if tokens and calls(tokens):
            repo = option(tokens, "--repo")
            if repo and ("serve" in tokens) and repo not in repos:
                repos.append(repo)
    return repos


# --- hub hooks -----------------------------------------------------------


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def check_hub_repo(report, repo):
    board = git("config", "olp.board", cwd=repo) if os.path.isdir(repo) else ""
    fix = f"octoscode olp repo init {shlex.quote(repo)} --board {shlex.quote(board or '<黑板>')}"
    if not os.path.isdir(repo):
        report.add("hub 钩子", repo, "fail", "裸仓库不存在", None)
        return
    for name in HOOKS:
        path = os.path.join(repo, "hooks", name)
        where = f"{repo} {name}"
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except OSError:
            report.add("hub 钩子", where, "fail", "钩子缺失", fix)
            continue
        line = next((ln for ln in text.splitlines() if ln.startswith("exec ")), "")
        if not os.access(path, os.X_OK):
            report.add("hub 钩子", where, "fail", "钩子不可执行", fix)
            continue
        tokens = split(report, "hub 钩子", where, line[len("exec ") :], fix)
        if tokens is not None and not calls(tokens):
            report.add("hub 钩子", where, "fail", "钩子里没有调用 OctoLoop 的命令", fix)


# --- local push settings (single machine) --------------------------------


def check_work_repo(report, work, remote, explicit):
    top = git("rev-parse", "--show-toplevel", cwd=work) if os.path.isdir(work) else ""
    if not top:
        if explicit:
            report.add("本机推送", work, "fail", "不是 git 工作仓库", None)
        return
    found = False
    for key in ("receivepack", "uploadpack"):
        line = git("config", f"remote.{remote}.{key}", cwd=top)
        if not line:
            continue
        found = True

        def fix(tokens):
            unit, repo = option(tokens, "--unit"), option(tokens, "--repo")
            return (
                f"cd {shlex.quote(top)} && octoscode olp repo connect-local --unit {unit or '<单元>'} "
                f"--repo {shlex.quote(repo or '<裸仓库>')} --remote {shlex.quote(remote)}"
            )

        split(report, "本机推送", f"{top} remote.{remote}.{key}", line, fix)
    if explicit and not found:
        report.add("本机推送", top, "ok", f"remote {remote} 没有本机推送设置(多机部署走 ssh,不需要)")
    check_remote_url(report, top, remote)


def local_path_of(url):
    """The local path a git remote URL names, or None for a network URL."""
    if url.startswith("file://"):
        return url[len("file://"):]
    if url.startswith("/") or url.startswith("~") or url.startswith("."):
        return url
    return None


def check_remote_url(report, top, remote):
    """Retrospective #13: a unit's clone copied the hub's own local path as
    the hub remote; on the unit that path is not the bare repository."""
    url = git("remote", "get-url", remote, cwd=top)
    if not url:
        return
    path = local_path_of(url)
    if path is None:
        return  # ssh/https: not checked offline
    path = os.path.abspath(os.path.join(top, os.path.expanduser(path)))
    bare = os.path.isdir(path) and git("rev-parse", "--is-bare-repository", cwd=path) == "true"
    where = f"{top} remote.{remote}.url"
    if bare:
        report.add("hub 远程", where, "ok", f"本机的裸仓库 {path}")
    else:
        report.add(
            "hub 远程", where, "fail",
            f"指向本机路径 {path},但这台机器上没有这个裸仓库(多半是照抄了 hub 上的路径)",
            f"cd {shlex.quote(top)} && git remote set-url {shlex.quote(remote)} <hub 的 ssh 地址,例如 olp-hub-git:/path/hub.git>",
        )


# --- keepalive jobs ------------------------------------------------------


def job_line(args):
    if len(args) == 3 and args[1] == "-c" and os.path.basename(args[0]) in ("sh", "bash"):
        return args[2]
    return CLI.shell(args)


def keepalive_fix(name):
    def fix(tokens):
        if name != "heartbeat":
            return f"用 octoscode olp keepalive install {name} … 重装这个任务"
        unit = option(tokens, "--unit") or "<单元>"
        hub = "local"
        if "ssh" in tokens and "receive" in tokens:
            hub = tokens[tokens.index("receive") - 1]
        require = option(tokens, "--require")
        return (
            f"octoscode olp keepalive heartbeat --unit {unit} --hub {shlex.quote(hub)}"
            + (f" --require {require}" if require else "")
        )

    return fix


def check_keepalive(report, home):
    agents = os.path.join(home, "Library", "LaunchAgents")
    if os.path.isdir(agents):
        for entry in sorted(os.listdir(agents)):
            if not (entry.startswith(LAUNCHD_PREFIX) and entry.endswith(".plist")):
                continue
            name = entry[len(LAUNCHD_PREFIX) : -len(".plist")]
            path = os.path.join(agents, entry)
            try:
                with open(path, "rb") as f:
                    args = plistlib.load(f).get("ProgramArguments", [])
            except Exception:
                report.add("保活任务", path, "fail", "无法解析", keepalive_fix(name)([]))
                continue
            split(report, "保活任务", f"launchd {name}", job_line(args), keepalive_fix(name))
    units = os.path.join(home, ".config", "systemd", "user")
    if os.path.isdir(units):
        for entry in sorted(os.listdir(units)):
            if not (entry.startswith(SYSTEMD_PREFIX) and entry.endswith(".service")):
                continue
            name = entry[len(SYSTEMD_PREFIX) : -len(".service")]
            path = os.path.join(units, entry)
            args = None
            try:
                with open(path, encoding="utf-8") as f:
                    for line in f:
                        if line.startswith(SYSTEMD_COMMAND_MARK):
                            args = json.loads(line[len(SYSTEMD_COMMAND_MARK) :])
                            break
            except (OSError, ValueError):
                args = None
            if not isinstance(args, list):
                report.add("保活任务", path, "fail", "找不到它记下的命令", keepalive_fix(name)([]))
                continue
            split(report, "保活任务", f"systemd {name}", job_line(args), keepalive_fix(name))


# --- running processes on a replaced binary -------------------------------

COMPONENTS = ("octoscode", "octos", "herdr")


def running_components():
    """[(pid, executable, started_epoch or None, replaced)] for this user's
    octoscode/octos/herdr processes. Linux: the executable's inode is no
    longer the file at its path (or it was deleted). macOS: the file at the
    path is newer than the process, or gone."""
    out = []
    me = os.getpid()
    if sys.platform.startswith("linux"):
        for name in os.listdir("/proc"):
            if not name.isdigit() or int(name) == me:
                continue
            try:
                exe = os.readlink(f"/proc/{name}/exe")
                running = os.stat(f"/proc/{name}/exe")
            except OSError:
                continue
            deleted = exe.endswith(" (deleted)")
            path = exe[: -len(" (deleted)")] if deleted else exe
            if os.path.basename(path) not in COMPONENTS:
                continue
            try:
                current = os.stat(path)
                replaced = deleted or (current.st_ino, current.st_dev) != (running.st_ino, running.st_dev)
            except OSError:
                replaced = True
            out.append((int(name), path, None, replaced))
    elif sys.platform == "darwin":
        r = subprocess.run(
            ["ps", "-U", str(os.getuid()), "-o", "pid=,lstart=,comm="], capture_output=True, text=True
        )
        for line in r.stdout.splitlines():
            parts = line.split(None, 6)
            if len(parts) < 7 or not parts[0].isdigit() or int(parts[0]) == me:
                continue
            # `comm` is the name it was started with (often bare "herdr");
            # ask the kernel for the executable's real path.
            if os.path.basename(parts[6].strip()) not in COMPONENTS:
                continue
            path = pid_path(int(parts[0]))
            if not path or os.path.basename(path) not in COMPONENTS:
                continue
            try:
                started = time.mktime(time.strptime(" ".join(parts[1:6]), "%a %b %d %H:%M:%S %Y"))
            except ValueError:
                continue
            try:
                replaced = os.stat(path).st_mtime > started + 1
            except OSError:
                replaced = True
            out.append((int(parts[0]), path, started, replaced))
    return out


def pid_path(pid):
    """macOS: the executable path of `pid` (proc_pidpath), or None."""
    import ctypes
    import ctypes.util

    try:
        lib = ctypes.CDLL(ctypes.util.find_library("proc") or "/usr/lib/libproc.dylib")
    except OSError:
        return None
    buf = ctypes.create_string_buffer(4096)
    n = lib.proc_pidpath(ctypes.c_int(pid), buf, ctypes.c_uint32(len(buf)))
    return buf.value.decode(errors="replace") if n > 0 else None


def check_running_versions(report):
    """Retrospective #06: 41 sessions kept running the old code after an
    upgrade; a version command shows only what a new call would run."""
    stale = {}
    for pid, path, _started, replaced in running_components():
        if replaced:
            stale.setdefault(path, []).append(pid)
    for path, pids in sorted(stale.items()):
        shown = ", ".join(str(p) for p in sorted(pids)[:12]) + (" …" if len(pids) > 12 else "")
        report.add(
            "运行中的旧版本", path, "warn",
            f"{len(pids)} 个进程在 {os.path.basename(path)} 更新之前启动,还在跑旧代码(PID {shown})",
            "逐个在合适的时候重启这些会话或服务;常驻任务用 octoscode olp keepalive 重新加载",
        )


# --- heartbeats on the hub ------------------------------------------------

V3_CHECKS = ("agent", "full")


def check_heartbeats(report):
    """Retrospective #10: heartbeats written by a pre-v3 unit lack the
    agent/full checks and the agents list, yet looked healthy."""
    root = os.environ.get("OLP_UNITS_DIR") or os.path.join(os.path.expanduser("~"), ".octos", "outer", "units")
    beats = os.path.join(root, "heartbeat")
    if not os.path.isdir(beats):
        return
    for entry in sorted(os.listdir(beats)):
        if not entry.endswith(".json"):
            continue
        unit = entry[: -len(".json")]
        try:
            with open(os.path.join(beats, entry), encoding="utf-8") as f:
                beat = json.load(f)
        except (OSError, ValueError):
            report.add("单元心跳", unit, "warn", "最近一次心跳读不出", None)
            continue
        names = {c.get("name") for c in beat.get("checks") or [] if isinstance(c, dict)}
        lacking = [c for c in V3_CHECKS if c not in names]
        if lacking or "agents" not in beat:
            what = "、".join(lacking + ([] if "agents" in beat else ["agents 字段"]))
            report.add(
                "单元心跳", unit, "warn",
                f"心跳是旧版本写的,缺 {what}:看起来健康不代表内环在跑",
                f"在单元 {unit} 上升级 octoscode 后重装心跳任务:octoscode olp keepalive heartbeat --unit {unit} --hub <hub>",
            )
        else:
            report.add("单元心跳", unit, "ok", "心跳带 v3 巡检项(agent、full)")


# --- skill ---------------------------------------------------------------


def check_octoscode_home(report, home):
    path = os.path.join(home, ".octos", "outer", "octoscode-home")
    try:
        with open(path, encoding="utf-8") as f:
            recorded = f.read().strip()
    except OSError:
        return
    fix = "octoscode olp skill install"
    if not os.path.isfile(os.path.join(recorded, "skills", "octoloop", "AGENTS.md")):
        report.add("skill", path, "fail", f"记下的位置没有规程正文:{recorded}", fix)
    elif pinned(recorded + "/"):
        report.add("skill", path, "warn", f"固定在某一版本的工具目录,升级后仍读旧正文:{recorded}", fix)
    else:
        report.add("skill", path, "ok", f"规程正文在 {recorded}")


# --- main ----------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser(prog="octoscode olp doctor", description="检查各处写下的 OctoLoop 命令")
    ap.add_argument("--repo", action="append", default=[], help="hub 裸仓库(可重复)")
    ap.add_argument("--work", action="append", default=[], help="单元的工作仓库(可重复)")
    ap.add_argument("--remote", default="hub", help="工作仓库里 hub 远程的名字(默认 hub)")
    ap.add_argument("--json", action="store_true", help="输出 JSON")
    args = ap.parse_args()

    home = os.path.expanduser("~")
    report = Report()
    repos = check_authorized_keys(report, os.path.join(home, ".ssh", "authorized_keys"))
    for repo in args.repo:
        repo = os.path.abspath(repo)
        if repo not in repos:
            repos.append(repo)
    for repo in repos:
        check_hub_repo(report, repo)
    works = [(os.path.abspath(w), True) for w in args.work]
    if not any(os.path.realpath(w) == os.path.realpath(os.getcwd()) for w, _ in works):
        works.append((os.getcwd(), False))
    for work, explicit in works:
        check_work_repo(report, work, args.remote, explicit)
    check_keepalive(report, home)
    check_octoscode_home(report, home)
    check_heartbeats(report)
    check_running_versions(report)

    writes = CLI.octoscode_bin()
    counts = {s: sum(1 for i in report.items if i["status"] == s) for s in RANK}
    if args.json:
        json.dump(
            {"octoscode": writes, "counts": counts, "checks": report.items},
            sys.stdout,
            ensure_ascii=False,
            indent=2,
        )
        print()
    else:
        print(f"新写下的命令将调用:{writes + ' olp …' if writes else '脚本路径(没有经 octoscode olp 运行)'}")
        if not report.items:
            print("没有找到写下的 OctoLoop 命令")
        for item in report.items:
            print(f"{item['status']:<5} {item['kind']}  {item['where']}")
            print(f"      {item['detail']}")
            if item["fix"]:
                print(f"      修复:{item['fix']}")
        print(f"共 {len(report.items)} 项:ok {counts['ok']},warn {counts['warn']},fail {counts['fail']}")
    return 1 if counts["fail"] else 0


if __name__ == "__main__":
    sys.exit(main())
