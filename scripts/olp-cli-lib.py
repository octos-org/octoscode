"""olp-cli-lib.py — which command to write into things that outlive this run.

Git hooks, a git remote's receivepack/uploadpack, keepalive jobs and ssh
forced commands keep a command line that calls an OctoLoop tool later.

`octoscode olp` runs the tools from a directory named after the content of the
binary (~/.octos/outer/olp-bundle/<hash>/) and sets OCTOSCODE_BIN to its own
path. A command line written from there must not name a script in that
directory: after an upgrade it keeps running the old copy, or fails once the
old directory is removed. It calls `<octoscode> olp …` instead, which always
runs the tools of the installed binary.

Run straight from a checkout (no OCTOSCODE_BIN), the tools keep writing the
path of the script in that checkout, which follows `git pull`.

A library, not a command: loaded by the scripts next to it.
"""

import os
import shlex
import shutil


def octoscode_bin():
    """Absolute path of the octoscode that runs this tool, or None.

    The name found on PATH is preferred when it is the same file: a package
    manager's link survives upgrades, the versioned file behind it does not.
    """
    exe = os.environ.get("OCTOSCODE_BIN", "")
    if not exe or not os.path.isabs(exe) or not os.access(exe, os.X_OK):
        return None
    on_path = shutil.which("octoscode")
    if on_path and os.path.realpath(on_path) == os.path.realpath(exe):
        return os.path.abspath(on_path)
    return exe


def command(olp_words, script, script_words, python="python3", use_cli=True):
    """Argument list that calls a tool later.

    `<octoscode> olp <olp_words>` when run through `octoscode olp` (and
    `use_cli`), else `<python> -B <script> <script_words>`.
    """
    exe = octoscode_bin() if use_cli else None
    if exe:
        return [exe, "olp", *olp_words]
    return [python, "-B", os.path.abspath(script), *script_words]


def shell(words):
    """The argument list as one shell command line."""
    return " ".join(shlex.quote(w) for w in words)


def duty_gate(prog):
    """Refuse a write from a dead or replaced outer session (R7 lease).

    `octoscode outer-duty hold` gives the outer agent, and every command it
    runs, OLP_DUTY_LEASE. A write carrying one goes through only while the
    project's duty lock is HELD with that lease; otherwise this exits 6.
    Without OLP_DUTY_LEASE (inner loops, units, a person), or from another
    herdr pane than the outer's, it does nothing.
    """
    import subprocess
    import sys

    lease = os.environ.get("OLP_DUTY_LEASE", "")
    if not lease:
        return
    # A writer in another herdr pane is not the outer, even if the lease
    # leaked into its environment (a herdr server started from the outer).
    if os.environ.get("HERDR_PANE_ID", "") != os.environ.get("OLP_DUTY_PANE", ""):
        return
    project = os.environ.get("OLP_DUTY_PROJECT", "")
    exe = os.environ.get("OLP_DUTY_BIN") or shutil.which("octoscode")
    reason = None
    if not project or not exe:
        reason = "环境里有 OLP_DUTY_LEASE,却没有 OLP_DUTY_PROJECT 或 octoscode"
    else:
        r = subprocess.run(
            [exe, "outer-duty", "lease", "--project", project], capture_output=True, text=True
        )
        if r.returncode != 0:
            reason = r.stderr.strip() or f"outer-duty lease 退出码 {r.returncode}"
    if reason:
        print(f"{prog}: 主审权租约无效,拒绝写入({reason})", file=sys.stderr)
        sys.exit(6)
