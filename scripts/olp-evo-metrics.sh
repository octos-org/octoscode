#!/usr/bin/env bash
# olp-evo-metrics.sh — evolution loop windowed diagnostics (#43c-2, SDD
# contract v2 specs/task-req-olp-evo-p2.spec.md). Diagnostic only, NOT a
# KPI. Read-only: never writes files, never reads retro state.
#
# Usage: olp-evo-metrics.sh <repo-root> [--since EVO-NNNN] [--json]
#                            [--baseline <json>]
# Exit code is always 0 (diagnostics).
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
        *) echo "error: unknown flag $1" >&2; exit 0 ;;
    esac
done

if [ ! -d "$REPO_ROOT" ]; then
    echo "error: repo-root not found: $REPO_ROOT" >&2
    exit 0
fi

EVO_BOARD="${OLP_EVO_BOARD:-$REPO_ROOT/.octos/EVOLUTION.md}"

python3 -B - "$EVO_BOARD" "$SINCE" "$JSON" "$BASELINE" "$(dirname "$0")" <<'PY'
import importlib.util, json, os, re, sys

board, since, as_json, baseline, script_dir = sys.argv[1:6]
as_json = as_json == "1"

spec = importlib.util.spec_from_file_location(
    "olp_evo_lib", os.path.join(script_dir, "olp-evo-lib.py")
)
lib = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lib)

# window base: --since, or the baseline's through_evo when given
base_since = 0
if since:
    m = re.match(r"^EVO-(\d+)$", since)
    if m:
        base_since = int(m.group(1))
base_trig = {}
if baseline and os.path.isfile(baseline):
    try:
        b = json.load(open(baseline))
        if not since:
            te = b.get("through_evo", "EVO-0000")
            mm = re.match(r"^EVO-(\d+)$", te)
            if mm:
                base_since = int(mm.group(1))
        base_trig = b.get("by_trigger", {})
    except Exception:
        base_trig = {}

text = ""
if os.path.isfile(board):
    text = open(board, encoding="utf-8").read()

cards = lib.parse_cards(text)
window = [c for c in cards if c["num"] > base_since]
through = max((c["num"] for c in window), default=0)

by_trigger = {}
by_source = {"review": 0, "events": 0, "mcp": 0}
for c in window:
    t = c.get("trigger") or "unknown"
    by_trigger[t] = by_trigger.get(t, 0) + 1
    s = (c.get("source") or "?").split(" ")[0]
    by_source[s] = by_source.get(s, 0) + 1

candidates = lib.group(window)
recurring = [g for g in candidates if g["recurrence_hint"] >= 2]

note = "diagnostic only, not a KPI; rising counts may mean better detection"
through_evo = f"EVO-{through:04d}"

# baseline deltas: increase:/decrease: for every known trigger on either
# side; equal → no line.
deltas = []
for k in sorted(set(by_trigger) | set(base_trig)):
    now = by_trigger.get(k, 0)
    base = base_trig.get(k, 0)
    if now > base:
        deltas.append(("increase", k, base, now))
    elif now < base:
        deltas.append(("decrease", k, base, now))

if as_json:
    out = {
        "note": note,
        "through_evo": through_evo,
        "cards": len(window),
        "by_trigger": dict(sorted(by_trigger.items())),
        "by_source": by_source,
        "recurring_candidates": len(recurring),
        "recurring": [
            {"trigger": g["trigger"], "hint": g["recurrence_hint"], "anchors": g["anchors"]}
            for g in recurring
        ],
        "deltas": [
            {"kind": d, "trigger": k, "base": b, "now": n}
            for d, k, b, n in deltas
        ],
    }
    print(json.dumps(out, ensure_ascii=False, indent=1))
else:
    print(f"note: {note}")
    print(f"through_evo: {through_evo}")
    print(f"cards: {len(window)}")
    trig = " ".join(f"{k}={v}" for k, v in sorted(by_trigger.items()))
    print(f"by_trigger: {trig}")
    print(
        "by_source: "
        + " ".join(f"{k}={by_source.get(k, 0)}" for k in ("review", "events", "mcp"))
    )
    print(f"recurring_candidates: {len(recurring)}")
    for g in recurring:
        print(f"- {g['trigger']} hint={g['recurrence_hint']} anchors={','.join(g['anchors'])}")
    for d, k, b, n in deltas:
        print(f"{d}: {k} {b}->{n}")
sys.exit(0)
PY
