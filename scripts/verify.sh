#!/usr/bin/env bash
# Unified validation entry.
#
# Fixes two recurring child-process environment problems in one place:
#   1. macOS ships BSD stat/realpath; repo scripts/tests expect GNU flags
#      (`stat -c '%s'`, `realpath -m`). On Darwin we ensure GNU tools are
#      usable: an ALREADY-GNU-capable PATH wins as-is; otherwise Homebrew
#      coreutils' gnubin is prepended (brew prefix first, then the stock
#      /opt/homebrew and /usr/local prefixes). Nothing is installed
#      automatically; if GNU tools remain unavailable the script fails with
#      actionable guidance (exit 127).
#   2. Nested python children did not inherit the parent's `-B`: export
#      PYTHONDONTWRITEBYTECODE=1 for the WHOLE subprocess tree so no
#      __pycache__ is written by any child.
#
# Usage:
#   scripts/verify.sh                # fmt --check, clippy -D warnings, test --all-targets
#   scripts/verify.sh -- <cmd...>    # run <cmd> in the fixed environment (passthrough)
#
# Exit codes are REAL: the first failing step's status is propagated; nothing
# is swallowed.
# Passthrough propagation applies after a command starts. A failed exec
# (for example a nonexistent executable) can exit 1 under this shell's
# errexit handling; callers must not interpret that as a test failure.

set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: scripts/verify.sh [-- <command> [args...]]
  (no arguments)   run the default ladder: cargo fmt --all --check,
                   cargo clippy --all-targets -- -D warnings, cargo test --all-targets
  -- <command>     exec <command> in the fixed environment (GNU tools +
                   PYTHONDONTWRITEBYTECODE=1); the command's exit code propagates
USAGE
}

# ── argument handling ────────────────────────────────────────────────────
if [[ $# -gt 0 ]]; then
  if [[ "$1" != "--" ]]; then
    echo "verify.sh: unknown argument: $1" >&2
    usage
    exit 64
  fi
  shift
  if [[ $# -eq 0 ]]; then
    echo "verify.sh: '--' requires a command" >&2
    usage
    exit 64
  fi
fi

# ── 2. bytecode suppression for the entire subprocess tree ──────────────
export PYTHONDONTWRITEBYTECODE=1

# ── 1. GNU tool capability (Darwin only; Linux is GNU-native) ───────────
gnu_probe_succeeds() {
  # Capability probe, not a directory-existence check: BOTH tools the repo
  # relies on must be GNU-capable — `stat -c` AND `realpath -m`. A mixed
  # PATH (GNU stat from one install, BSD realpath) is NOT sufficient and
  # must fall through to the gnubin resolution.
  command -v stat >/dev/null 2>&1 \
    && stat -c '%s' /dev/null >/dev/null 2>&1 \
    && command -v realpath >/dev/null 2>&1 \
    && realpath -m /dev/null >/dev/null 2>&1
}

if [[ "$(uname -s)" == "Darwin" ]]; then
  if ! gnu_probe_succeeds; then
    gnubin=""
    # Prefer the active brew prefix (any arch / custom location). The
    # assignment is guarded so a failing `brew --prefix` cannot trip `set -e`
    # before the fallback prefixes run.
    if command -v brew >/dev/null 2>&1; then
      candidate="$(brew --prefix coreutils 2>/dev/null || true)"
      candidate="$candidate/libexec/gnubin"
      [[ -d "$candidate" ]] && gnubin="$candidate"
    fi
    if [[ -z "$gnubin" ]]; then
      for prefix in /opt/homebrew /usr/local; do
        candidate="$prefix/opt/coreutils/libexec/gnubin"
        if [[ -d "$candidate" ]]; then
          gnubin="$candidate"
          break
        fi
      done
    fi
    if [[ -n "$gnubin" ]]; then
      PATH="$gnubin:$PATH"
      export PATH
    fi
    # Re-probe AFTER prepending: only now can we judge true availability.
    if ! gnu_probe_succeeds; then
      echo "verify.sh: GNU coreutils (stat -c / realpath -m) not usable on this Mac." >&2
      echo "  If Homebrew coreutils is installed, ensure 'brew --prefix coreutils'/libexec/gnubin" >&2
      echo "  exists under /opt/homebrew or /usr/local. Otherwise install once:" >&2
      echo "    brew install coreutils" >&2
      echo "  No automatic install is performed." >&2
      exit 127
    fi
  fi
fi

if [[ $# -gt 0 ]]; then
  exec "$@"
fi

# ── default validation ladder (real exit codes, no swallowing) ───────────
run_step() {
  local title="$1"; shift
  echo "==> verify.sh: $title"
  "$@"
  echo "==> verify.sh: $title OK"
}

run_step "cargo fmt --check"   cargo fmt --all --check
run_step "cargo clippy"        cargo clippy --all-targets -- -D warnings
run_step "cargo test"          cargo test --all-targets
echo "==> verify.sh: all steps passed"
