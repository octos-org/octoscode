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
