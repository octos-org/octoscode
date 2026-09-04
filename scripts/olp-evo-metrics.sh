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
STALL=""; STALL_THRESHOLD=0; NOW=""
while [ $# -gt 0 ]; do
    case "$1" in
        --since) SINCE="${2:?--since needs EVO-NNNN}"; shift 2 ;;
        --json) JSON=1; shift ;;
        --baseline) BASELINE="${2:?--baseline needs a json file}"; shift 2 ;;
        --stall) STALL="${2:?--stall needs a review-board path}"; shift 2 ;;
        --stall-threshold) STALL_THRESHOLD="${2:?--stall-threshold needs minutes}"; shift 2 ;;
        --now) NOW="${2:?--now needs ISO8601}"; shift 2 ;;
        *) echo "error: unknown flag $1" >&2; exit 0 ;;
    esac
done

if [ ! -d "$REPO_ROOT" ]; then
    echo "error: repo-root not found: $REPO_ROOT" >&2
    exit 0
fi

EVO_BOARD="${OLP_EVO_BOARD:-$REPO_ROOT/.octos/EVOLUTION.md}"

python3 - "$EVO_BOARD" "$SINCE" "$JSON" "$BASELINE" "$(dirname "$0")" "$STALL" "$STALL_THRESHOLD" "$NOW" <<'PY'
import datetime, importlib.util, json, os, re, sys

board, since, as_json, baseline, script_dir, stall_board, stall_threshold, now_s = sys.argv[1:9]
as_json = as_json == "1"
stall_threshold = int(stall_threshold or 0)

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

# --- stall diagnostics (#44c) -------------------------------------------
stalls = []
if stall_board and os.path.isfile(stall_board):
    SLICE = r"[0-9]+[a-z]?(?:-[0-9a-z]+)*"
    re_dispatch_a = re.compile(rf"(?:^|\s)(?:派单\s+(?P<a>{SLICE}))|(?P<b>{SLICE})\s+派单(?![0-9a-z])")
    re_ack = re.compile(rf"^(?:> )?ACK\((?P<s>{SLICE})\s+(?:done|blocked|wontdo)\b")
    re_date_inline = re.compile(r"\((\d{4}-\d\d-\d\d) (\d\d:\d\d)")
    re_date_bracket = re.compile(r"\[(\d{4}-\d\d-\d\d)T(\d\d:\d\d)")
    re_heading_date = re.compile(r"^###\s+\S+.*?\((\d{4}-\d\d-\d\d)")

    now = datetime.datetime.now(datetime.timezone.utc)
    if now_s:
        now = datetime.datetime.fromisoformat(now_s.replace("Z", "+00:00"))
        if now.tzinfo is None:
            now = now.replace(tzinfo=datetime.timezone.utc)

    dispatched = {}  # slice → (time or None, longest-key)
    acked = set()
    heading_date = None
    for raw in open(stall_board, encoding="utf-8"):
        line = raw.rstrip("\n")
        m = re_heading_date.match(line)
        if m:
            heading_date = m.group(1)
        # strip prefixes for dispatch detection
        t = line
        t = re.sub(r"^(?:>\s+)?", "", t)
        t = t.replace("**", "")
        t = re.sub(r"^外环(?:\([^)]*\))?·?", "", t)
        md = re_dispatch_a.search(t)
        if md:
            sl = md.group("a") or md.group("b")
            if sl and sl not in dispatched:
                tm = None
                mi = re_date_inline.search(line) or re_date_bracket.search(line)
                if mi:
                    tm = datetime.datetime.fromisoformat(
                        f"{mi.group(1)}T{mi.group(2)}:00+00:00"
                    )
                elif heading_date:
                    tm = datetime.datetime.fromisoformat(f"{heading_date}T00:00:00+00:00")
                dispatched[sl] = (tm, sl)
        ma = re_ack.match(line)
        if ma:
            acked.add(ma.group("s"))

    for sl, (tm, _) in dispatched.items():
        if sl in acked:
            continue
        if tm is None:
            stalls.append((sl, None))
        else:
            minutes = int((now - tm).total_seconds() // 60)
            if minutes >= stall_threshold:
                stalls.append((sl, minutes))
    # keep only the longest ack-slice matches: an acked superset like
    # "43" does not un-stall "43c-2"; nothing extra needed since acked
    # entries use the same atom.

fake_verified = by_trigger.get("r2_record", 0)

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
        "stalls": [{"slice": sl, "minutes": m} for sl, m in stalls],
        "fake_verified": fake_verified,
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
    if stall_board:
        for sl, m in stalls:
            print(f"stall: {sl} {'open' if m is None else m}")
        print(f"stalls: {len(stalls)}")
    print(f"fake_verified: {fake_verified}")
sys.exit(0)
PY
