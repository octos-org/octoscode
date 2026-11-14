#!/usr/bin/env python3
"""OLP 本地 git 仓库(hub):单元只能推自己的任务分支,且必须带本地复验记录。

多机部署时,各单元(mini)不直接推 GitHub,只向 hub 上的一个裸仓库推
`task/<N>` 分支——一次推送就是一个"本地 PR"。大外环基于这个仓库审核与
回测,通过后才由 hub 推到 GitHub。本工具负责把住单元 → hub 这一道门。

子命令(都在 hub 上)
  init <裸仓库路径> --board <黑板路径>
      建裸仓库(已存在则只刷新),装 pre-receive / post-receive 钩子,把
      黑板路径记进仓库配置。重跑是安全的。
  serve --unit <单元号> --repo <裸仓库路径>
      ssh 强制命令。只放行对这个仓库的 git 推送与拉取,并把单元号传给
      钩子。在 hub 的 authorized_keys 里给每个单元的密钥配:
        command="/path/scripts/olp-git-hub.py serve --unit m07 --repo /path/hub.git",no-pty,no-port-forwarding ssh-ed25519 AAAA…
  hook pre-receive | post-receive
      由钩子调用,不手工执行。

单机(hub 与单元在同一台机器)
  connect-local --unit <单元号> --repo <裸仓库路径> [--remote hub]
      在单元的工作仓库里执行一次。把 hub 远程指向本机的裸仓库,并让 git
      的推送与拉取经 `serve` 进行——与 ssh 强制命令同一条路径、同样的
      规则,单元号由这里给出。同一个用户下没有 ssh 那样的身份隔离:直接
      `git push <裸仓库路径>` 能绕开它,所以这只防误操作,不防有意绕开。
  local-pack …
      由 git 调用,不手工执行。

推送规则(对带单元号的推送,即经 serve 进来的)
  1. 只能推 `refs/heads/task/<N>`,N 是数字。不能推其他分支与标签,不能
     删分支。
  2. N 必须是黑板上的条目,且条目里有一行 `unit: <单元号>` 把它派给了
     这个单元。
  3. 推送必须带本地复验记录:
         git push -o olp-verified=<新提交的完整 sha>:<复验署名> hub task/<N>
     sha 必须等于这次推上来的分支头——复验的就是推送的那个提交。单元侧
     用 scripts/olp-task-push.sh 生成这条命令。
  4. 同一个单元可以强推自己的任务分支(返工、rebase 到新种子后)。

  不带单元号的推送(hub 本机上大外环的操作)不受限制:集成分支、种子
  分支由大外环自己维护。所以单元账号**必须**走 serve 强制命令,不要给
  单元发不带强制命令的密钥。

推送成功后
  post-receive 在黑板末尾追加一行(经 olp-board-append.sh,flock 互斥):
      > 单元(m07)·本地复验通过(#212): task/212 @ <sha 前 12 位> 复验 m07-ver
  大外环的正哨对这一行挂哨即可被唤醒。追加失败不回滚推送,只在推送方的
  输出里告警。

本地复验记录能证明什么
  它保证"有人声明复验了这个提交"并把声明落到黑板上留痕;它不能证明复验
  真的独立——内环和复验 agent 在同一台机器上。独立性靠单元内的身份隔离
  与大外环的第二道审核。

退出码:0 成功;1 推送被拒(钩子);2 用法错误;3 serve 拒绝的命令。
"""

import argparse
import os
import re
import shlex
import stat
import subprocess
import sys

UNIT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
TASK_REF = re.compile(r"^refs/heads/task/([0-9]{1,9})$")
SHA_RE = re.compile(r"^[0-9a-f]{40}([0-9a-f]{24})?$")
SIGNER_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._@+-]{0,63}$")
HEADING = re.compile(r"^### (\d+)")
UNIT_LINE = re.compile(r"^unit:\s*(\S+)\s*$")
ZERO = re.compile(r"^0+$")
HOOKS = ("pre-receive", "post-receive")


def die(code, msg):
    print(f"olp-hub: {msg}", file=sys.stderr)
    sys.exit(code)


def git(*args, cwd=None, check=True):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if check and r.returncode != 0:
        die(2, f"git {' '.join(args)} 失败: {r.stderr.strip()}")
    return r.stdout.strip()


# --- init ---------------------------------------------------------------


def cmd_init(args):
    repo = os.path.abspath(args.repo)
    board = os.path.abspath(args.board)
    if not os.path.isfile(board):
        die(2, f"黑板不存在: {board}")
    if os.path.exists(repo):
        if git("rev-parse", "--is-bare-repository", cwd=repo, check=False) != "true":
            die(2, f"{repo} 已存在且不是裸仓库")
    else:
        git("init", "--bare", "--quiet", repo)
    script = os.path.abspath(__file__)
    git("config", "olp.board", board, cwd=repo)
    git("config", "olp.scripts", os.path.dirname(script), cwd=repo)
    # 本地复验记录经 push option 传入。
    git("config", "receive.advertisePushOptions", "true", cwd=repo)
    hooks_dir = os.path.join(repo, "hooks")
    os.makedirs(hooks_dir, exist_ok=True)
    for name in HOOKS:
        path = os.path.join(hooks_dir, name)
        with open(path, "w", encoding="utf-8") as f:
            f.write(
                "#!/bin/sh\n"
                "# Installed by olp-git-hub.py init; re-run init to refresh.\n"
                f"exec python3 -B {shlex.quote(script)} hook {name}\n"
            )
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    print(f"hub repo ready: {repo} (board: {board})")
    return 0


# --- serve (ssh forced command) -----------------------------------------


def cmd_serve(args):
    if not UNIT_RE.match(args.unit):
        die(2, f"单元号不合法: {args.unit}")
    repo = os.path.realpath(args.repo)
    original = os.environ.get("SSH_ORIGINAL_COMMAND", "")
    try:
        words = shlex.split(original)
    except ValueError:
        words = []
    if len(words) == 3 and words[0] == "git" and words[1] in ("receive-pack", "upload-pack"):
        words = ["git-" + words[1], words[2]]
    if len(words) != 2 or words[0] not in ("git-receive-pack", "git-upload-pack"):
        die(3, "这个密钥只能用于对 hub 仓库的 git 推送与拉取")
    if os.path.realpath(words[1]) != repo:
        die(3, f"这个密钥只能访问 {repo}")
    env = dict(os.environ)
    env["OLP_UNIT"] = args.unit
    os.execvpe("git", ["git", words[0][4:], repo], env)
    return 3  # unreachable


def cmd_connect_local(args):
    """Single machine: point this repository's hub remote at the bare repo on
    the same disk so that pushes still go through `serve` with the unit id."""
    if not UNIT_RE.match(args.unit):
        die(2, f"单元号不合法: {args.unit}")
    repo = os.path.realpath(args.repo)
    if not os.path.isdir(repo):
        die(2, f"hub 仓库不存在: {repo}")
    me = os.path.abspath(__file__)
    # git runs remote.<name>.receivepack through the shell with the repository
    # path appended. `serve` then sees the same command an ssh push would send.
    pack = f"{shlex.quote(sys.executable)} -B {shlex.quote(me)} local-pack --unit {args.unit} --repo {shlex.quote(repo)} git-receive-pack"
    upload = pack[: -len("git-receive-pack")] + "git-upload-pack"
    remote = args.remote
    have = subprocess.run(["git", "remote", "get-url", remote], capture_output=True, text=True)
    verb = "set-url" if have.returncode == 0 else "add"
    for cmd in (
        ["git", "remote", verb, remote, repo],
        ["git", "config", f"remote.{remote}.receivepack", pack],
        ["git", "config", f"remote.{remote}.uploadpack", upload],
    ):
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            die(2, f"{' '.join(cmd[:3])} 失败: {r.stderr.strip()}")
    print(f"remote {remote} -> {repo}(本机;推送经 serve,单元号 {args.unit})")
    return 0


def cmd_local_pack(args):
    """What git runs for a local push or fetch on a connected remote."""
    os.environ["SSH_ORIGINAL_COMMAND"] = " ".join(
        shlex.quote(w) for w in [args.pack] + list(args.path)
    )
    return cmd_serve(args)


# --- hooks --------------------------------------------------------------


def board_assignment(board, number):
    """Return (found, unit) for entry `number` on the board: whether the
    entry exists, and the unit named by its `unit:` line (None if absent)."""
    found, unit, current, in_fence = False, None, None, False
    try:
        with open(board, "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.rstrip("\r\n")
                stripped = line.lstrip()
                if stripped.startswith("```") or stripped.startswith("~~~"):
                    in_fence = not in_fence
                    continue
                if in_fence:
                    continue
                if line.startswith("### ") or line.startswith("## "):
                    m = HEADING.match(line)
                    current = int(m.group(1)) if m else None
                    if current == number:
                        found = True
                    continue
                if current == number:
                    m = UNIT_LINE.match(line)
                    if m:
                        # 以最后一条为准:改派时在条目里追加新的 unit 行。
                        unit = m.group(1)
    except OSError:
        return False, None
    return found, unit


def push_options():
    try:
        count = int(os.environ.get("GIT_PUSH_OPTION_COUNT", "0"))
    except ValueError:
        count = 0
    return [os.environ.get(f"GIT_PUSH_OPTION_{i}", "") for i in range(count)]


def verified_option(new_sha):
    """Return the signer if a push option vouches for exactly `new_sha`."""
    for opt in push_options():
        if not opt.startswith("olp-verified="):
            continue
        sha, _, signer = opt[len("olp-verified="):].partition(":")
        if sha == new_sha and SIGNER_RE.match(signer):
            return signer
    return None


def read_updates():
    updates = []
    for line in sys.stdin:
        parts = line.split()
        if len(parts) == 3:
            updates.append(tuple(parts))
    return updates


def hook_pre_receive():
    unit = os.environ.get("OLP_UNIT", "")
    updates = read_updates()
    if not unit:
        # hub 本机上大外环的推送,不受单元规则约束。但带着本地复验记录的推送
        # 是单元在交活:它没有经过 serve,说明这个克隆的 hub 远程没接好(单机
        # 部署时漏了 connect-local)。照收会绕开推送规则、黑板上也不会登记,
        # 所以拒收并说明怎么接。
        if any(o.startswith("olp-verified=") for o in push_options()):
            print(
                "olp-hub: 拒收:这是单元带复验记录的推送,却没有经过 serve(没有单元号)。"
                "单机部署时先在这个克隆里执行 "
                "scripts/olp-git-hub.py connect-local --unit <单元号> --repo <hub 仓库>,再推送",
                file=sys.stderr,
            )
            return 1
        return 0
    if not UNIT_RE.match(unit):
        print(f"olp-hub: 拒收:单元号不合法 {unit}", file=sys.stderr)
        return 1
    board = git("config", "olp.board", check=False)
    errors = []
    for old, new, ref in updates:
        m = TASK_REF.match(ref)
        if not m:
            errors.append(f"{ref}:单元只能推 refs/heads/task/<N>")
            continue
        number = int(m.group(1))
        if ZERO.match(new):
            errors.append(f"{ref}:单元不能删除分支")
            continue
        if not SHA_RE.match(new):
            errors.append(f"{ref}:提交号不合法")
            continue
        found, assigned = board_assignment(board, number)
        if not found:
            errors.append(f"{ref}:黑板上没有条目 #{number}")
            continue
        if assigned != unit:
            who = assigned or "未指定单元(条目里没有 unit: 行)"
            errors.append(f"{ref}:#{number} 派给的是 {who},不是 {unit}")
            continue
        if not verified_option(new):
            errors.append(
                f"{ref}:缺少本地复验记录。本地复验通过后用 "
                f"scripts/olp-task-push.sh {number} --verified-by <复验署名> 推送"
                "(push option olp-verified 的 sha 必须等于分支头)"
            )
    if errors:
        for e in errors:
            print(f"olp-hub: 拒收 {e}", file=sys.stderr)
        return 1
    return 0


def hook_post_receive():
    unit = os.environ.get("OLP_UNIT", "")
    updates = read_updates()
    if not unit:
        return 0
    board = git("config", "olp.board", check=False)
    scripts = git("config", "olp.scripts", check=False)
    append = os.path.join(scripts, "olp-board-append.sh")
    for old, new, ref in updates:
        m = TASK_REF.match(ref)
        if not m or ZERO.match(new):
            continue
        signer = verified_option(new) or "?"
        line = (
            f"> 单元({unit})·本地复验通过(#{m.group(1)}): "
            f"task/{m.group(1)} @ {new[:12]} 复验 {signer}\n"
        )
        # 钩子环境里的 GIT_DIR 等变量不该漏给写板脚本。
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        try:
            r = subprocess.run(
                ["bash", append, board],
                input=line,
                text=True,
                capture_output=True,
                env=env,
                timeout=60,
            )
            ok, why = r.returncode == 0, r.stderr.strip()
        except (OSError, subprocess.SubprocessError) as e:
            ok, why = False, str(e)
        if ok:
            print(f"olp-hub: 已收 {ref} @ {new[:12]},已在黑板登记", file=sys.stderr)
        else:
            print(
                f"olp-hub: 已收 {ref} @ {new[:12]},但黑板登记失败({why});"
                "请告知大外环这次推送",
                file=sys.stderr,
            )
    return 0


def cmd_hook(args):
    if args.name == "pre-receive":
        return hook_pre_receive()
    return hook_post_receive()


def main():
    ap = argparse.ArgumentParser(prog="olp-git-hub.py", description="OLP 本地 git 仓库(hub)")
    sub = ap.add_subparsers(dest="cmd")
    p = sub.add_parser("init", help="建或刷新 hub 裸仓库与钩子")
    p.add_argument("repo")
    p.add_argument("--board", required=True)
    p.set_defaults(fn=cmd_init)
    p = sub.add_parser("serve", help="ssh 强制命令:只放行对 hub 仓库的 git 操作")
    p.add_argument("--unit", required=True)
    p.add_argument("--repo", required=True)
    p.set_defaults(fn=cmd_serve)
    p = sub.add_parser("connect-local", help="(单机)把当前仓库的 hub 远程接到本机的 hub 仓库")
    p.add_argument("--unit", required=True)
    p.add_argument("--repo", required=True)
    p.add_argument("--remote", default="hub")
    p.set_defaults(fn=cmd_connect_local)
    p = sub.add_parser("local-pack", help="由 git 调用(connect-local 配置的 receivepack/uploadpack)")
    p.add_argument("--unit", required=True)
    p.add_argument("--repo", required=True)
    p.add_argument("pack", choices=["git-receive-pack", "git-upload-pack"])
    p.add_argument("path", nargs="*")
    p.set_defaults(fn=cmd_local_pack)
    p = sub.add_parser("hook", help="由钩子调用")
    p.add_argument("name", choices=HOOKS)
    p.set_defaults(fn=cmd_hook)
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return 2 if exc.code not in (0, None) else 0
    if not getattr(args, "fn", None):
        ap.print_usage(sys.stderr)
        return 2
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
