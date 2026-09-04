#!/usr/bin/env bash
# olp-evo-metrics.sh — evolution loop §1 success metrics as one command
# (#43c, SDD contract specs/task-req-olp-evo-p2.spec.md).
#
# Usage: olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json]
#                            [--baseline <json>]
# Read-only: creates/modifies nothing.
set -euo pipefail

REPO_ROOT="${1:?usage: olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json] [--baseline <json>]}"
shift || true
SINCE=""
JSON=0
BASELINE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --since) SINCE="${2:?--since needs EVO-NNNN}"; shift 2 ;;
        --json) JSON=1; shift ;;
        --baseline) BASELINE="${2:?--baseline needs a json file}"; shift 2 ;;
        *) echo "error: unknown flag $1" >&2; exit 1 ;;
    esac
done

if [ ! -d "$REPO_ROOT" ]; then
    echo "error: repo-root not found: $REPO_ROOT" >&2
    exit 1
fi

EVO_BOARD="${OLP_EVO_BOARD:-$REPO_ROOT/.octos/EVOLUTION.md}"
STATE_ROOT="${OLP_EVO_STATE:-$HOME/.octos/outer/evo}"
PROJECT_KEY=$(printf '%s' "$(realpath "$REPO_ROOT")" | sha256sum | cut -c1-16)
STATE_DIR="$STATE_ROOT/$PROJECT_KEY"
RETRO_DIR="${OLP_EVO_RETRO_DIR:-$STATE_DIR/retro}"

python3 - "$EVO_BOARD" "$RETRO_DIR" "$SINCE" "$JSON" "$BASELINE" <<'PY'
import glob, json, os, re, sys

board, retro_dir, since, as_json, baseline = sys.argv[1:6]
as_json = as_json == "1"
since_n = 0
if since:
    m = re.match(r"^EVO-(\d+)$", since)
    if m:
        since_n = int(m.group(1))

# --- parse cards --------------------------------------------------------
cards = []
if os.path.isfile(board):
    cur = None
    for line in open(board, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.startswith("### EVO-"):
            if cur:
                cards.append(cur)
            num = line[len("### EVO-"):].split("（")[0]
            cur = {"num": int(num) if num.isdigit() else 0, "trigger": None, "source": None}
            continue
        if cur is None:
            continue
        for field in ("trigger:", "source:"):
            if line.startswith(field):
                cur[field[:-1]] = line[len(field):].strip()
    if cur:
        cards.append(cur)

sel = [c for c in cards if c["num"] > since_n]
by_trigger = {}
by_source = {}
for c in sel:
    t = c.get("trigger") or "unknown"
    by_trigger[t] = by_trigger.get(t, 0) + 1
    s = (c.get("source") or "?").split(" ")[0]
    by_source[s] = by_source.get(s, 0) + 1

# --- recurring candidates from the latest brief -------------------------
recurring = []
if os.path.isdir(retro_dir):
    briefs = sorted(glob.glob(os.path.join(retro_dir, "*.md")))
    if briefs:
        text = open(briefs[-1], encoding="utf-8").read()
        for m in re.finditer(r"^## C\d+ (\S+) · recurrence_hint=(\d+)", text, re.M):
            trigger, hint = m.group(1), int(m.group(2))
            if hint >= 2:
                recurring.append((trigger, hint))

metrics = {
    "cards": len(sel),
    "by_trigger": dict(sorted(by_trigger.items())),
    "by_source": dict(sorted(by_source.items())),
    "recurring_candidates": [
        {"trigger": t, "hint": h} for t, h in recurring
    ],
}

# --- baseline regression flags ------------------------------------------
regress = []
if baseline and os.path.isfile(baseline):
    try:
        base = json.load(open(baseline))
        btrig = base.get("by_trigger", {})
        for k, v in sorted(by_trigger.items()):
            prev = btrig.get(k, 0)
            if v > prev:
                regress.append(f"{k} {prev}->{v}")
    except Exception:
        pass

# --- output -------------------------------------------------------------
if as_json:
    out = dict(metrics)
    if regress:
        out["regress"] = regress
    print(json.dumps(out, ensure_ascii=False, indent=1))
else:
    print(f"cards: {metrics['cards']}")
    trig_line = " ".join(f"{k}={v}" for k, v in metrics["by_trigger"].items())
    src_line = " ".join(f"{k}={v}" for k, v in metrics["by_source"].items())
    print(f"by_trigger: {trig_line}")
    print(f"by_source: {src_line}")
    print(f"recurring_candidates: {len(recurring)}")
    for t, h in recurring:
        print(f"- {t} hint={h}")
    for r in regress:
        print(f"regress: {r}")
sys.exit(0)
PY
