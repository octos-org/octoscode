#!/usr/bin/env bash
# olp-evo-retro.sh — evolution loop phase-1 retro (#42, SDD contract
# specs/task-req-olp-evo-p1.spec.md). Reads cards harvested since the
# last retro, groups them into candidates, and writes a brief with
# record drafts. Judgement stays with the locked outer reviewer.
#
# Usage: olp-evo-retro.sh <repo-root> [--dry-run]
# Exit codes: 70 injected fault; 0 ok (incl. no new cards); 1 errors.
set -euo pipefail

REPO_ROOT="${1:?usage: olp-evo-retro.sh <repo-root> [--dry-run]}"
DRY_RUN=0
[ "${2:-}" = "--dry-run" ] && DRY_RUN=1

if [ ! -d "$REPO_ROOT" ]; then
    echo "error: repo-root not found: $REPO_ROOT" >&2
    exit 1
fi

REAL_REPO=$(realpath "$REPO_ROOT")
EVO_BOARD="${OLP_EVO_BOARD:-$REPO_ROOT/.octos/EVOLUTION.md}"
STATE_ROOT="${OLP_EVO_STATE:-$HOME/.octos/outer/evo}"
PROJECT_KEY=$(printf '%s' "$REAL_REPO" | sha256sum | cut -c1-16)
STATE_DIR="$STATE_ROOT/$PROJECT_KEY"
RETRO_JSON="$STATE_DIR/retro.json"
RETRO_LOCK="$STATE_DIR/retro.lock"
RETRO_DIR="$STATE_DIR/retro"

RETRO_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

die() { echo "error: $*" >&2; exit 1; }

# --- helpers -----------------------------------------------------------
read_last_id() {
    if [ -f "$RETRO_JSON" ]; then
        python3 -c '
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    print(d.get("last_id", 0))
except Exception:
    print(0)
' "$RETRO_JSON"
    else
        echo 0
    fi
}

read_runs_len() {
    if [ -f "$RETRO_JSON" ]; then
        python3 -c '
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    print(len(d.get("runs", [])))
except Exception:
    print(0)
' "$RETRO_JSON"
    else
        echo 0
    fi
}

next_flaw_number() {
    local dir="$1" best=0 n
    for f in "$dir"/FLAW-[0-9]*.md; do
        [ -f "$f" ] || continue
        n=${f##*/}
        n=${n#FLAW-}
        n=${n%.md}
        [[ $n =~ ^[0-9]+$ ]] || continue
        n=$((10#$n))
        [ "$n" -gt "$best" ] && best=$n
    done
    echo $((best + 1))
}

# --- brief computation (single python pass; prints brief to stdout,
# --- plus a summary line "SUMMARY|cards|candidates|max_id" on fd 3) ----
compute_brief() { # board last_id repo_root flaw_start
    python3 - "$1" "$2" "$3" "$4" <<'PY'
import json, os, re, sys

board_path, last_id, repo_root, flaw_start = (
    sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]),
)

LAYER = {
    "ack_blocked": "Lifecycle", "ack_wontdo": "Lifecycle",
    "goal_blocked": "Lifecycle", "goal_budget_limited": "Lifecycle",
    "escalation": "Lifecycle",
    "report_blocked": "Tooling", "ask_outer_timeout": "Tooling",
    "turn_error": "Execution",
    "override": "Governance", "r2_record": "Verification",
}

PREFIXES = [
    "ACK(blocked):", "ACK(wontdo):",
]

def group_text(trigger, symptom):
    if trigger in ("ack_blocked", "ack_wontdo", "override", "r2_record"):
        t = symptom
        for p in PREFIXES:
            if t.startswith(p):
                t = t[len(p):]
                break
        # strip the signed form prefix 外环(…)·改判(…): / 外环(…)·R2 记档(…):
        m = re.match(r"^外环\([^)]*\)·(?:改判|R2 记档)\([^)]*\)[:：]\s*", t)
        if m:
            t = t[m.end():]
        return t
    if trigger in ("escalation", "turn_error", "goal_blocked", "goal_budget_limited"):
        try:
            d = json.loads(symptom)
            if isinstance(d, dict):
                det = d.get("data", {}).get("detail") or d.get("detail")
                if det is not None:
                    return str(det)
        except Exception:
            pass
        return symptom
    # report_blocked / ask_outer_timeout
    idx = symptom.find("reason=")
    if idx >= 0:
        return symptom[idx + len("reason="):]
    return symptom

def normalize(text):
    t = text.lower()
    t = re.sub(r"(?<![\w])/[^\s]+", "<path>", t)
    t = re.sub(r"(?<![A-Za-z])[0-9a-f]{8,}(?![A-Za-z])", "<hex>", t)
    t = re.sub(r"(?<![A-Za-z_\d])\d+(?![A-Za-z])", "<num>", t)
    t = re.sub(r"\s+", " ", t).strip()
    return t[:80]

def anchor(identity, fallback_id):
    body = identity
    for pfx in ("board:", "events:", "mcp:"):
        if body.startswith(pfx):
            body = body[len(pfx):]
            break
    parts = body.rsplit("#")
    if body.startswith("/") or "#" in body:
        # board: 倒数第 3 段;events/mcp: 倒数第 2 段 — decided by prefix
        # (prefix was stripped; re-detect from the original identity)
        pass
    return parts, body

def anchor_value(identity, source_kind, fallback_id):
    body = identity
    kind = None
    for pfx in ("board:", "events:", "mcp:"):
        if body.startswith(pfx):
            kind = pfx[:-1]
            body = body[len(pfx):]
            break
    parts = body.rsplit("#")
    if kind == "board":
        seg = parts[-3] if len(parts) >= 3 else "-"
    else:
        seg = parts[-2] if len(parts) >= 2 else "-"
    if seg == "-" or seg == "":
        return fallback_id
    return seg

# --- parse cards -------------------------------------------------------
cards = []
malformed = []
if os.path.isfile(board_path):
    with open(board_path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    cur = None
    for line in lines:
        if line.startswith("### EVO-"):
            if cur is not None:
                cards.append(cur)
            num = line[len("### EVO-"):].split("（")[0]
            cur = {"id": f"EVO-{num}", "num": int(num) if num.isdigit() else 0,
                   "trigger": None, "identity": None, "symptom": None,
                   "source": "-", "envelope": "-"}
            continue
        if cur is None:
            continue
        for field in ("trigger:", "identity:", "symptom:", "source:", "envelope:"):
            if line.startswith(field):
                cur[field[:-1]] = line[len(field):].strip()
    if cur is not None:
        cards.append(cur)

# malformed: missing trigger/identity/symptom — reported only when the
# card is NEWER than the cursor (a rerun must not re-report it).
kept = []
for c in cards:
    if c["trigger"] and c["identity"] and c["symptom"]:
        kept.append(c)
    elif c["num"] > last_id:
        malformed.append(c["id"])

# only cards with number > last_id are new
new_cards = [c for c in kept if c["num"] > last_id]
# last_id covers MALFORMED cards too (rerun must not re-report them).
max_seen = max((c["num"] for c in cards), default=last_id)
if max_seen < last_id:
    max_seen = last_id

# --- group into candidates ---------------------------------------------
order = []
groups = {}
for c in new_cards:
    gt = group_text(c["trigger"], c["symptom"])
    key = c["trigger"] + "|" + normalize(gt)
    if key not in groups:
        groups[key] = {"trigger": c["trigger"], "gt": gt, "cards": []}
        order.append(key)
    groups[key]["cards"].append(c)

for k in order:
    g = groups[k]
    anchors = []
    for c in g["cards"]:
        a = anchor_value(c["identity"], c["source"], c["id"])
        if a not in anchors:
            anchors.append(a)
    g["anchors"] = anchors
    g["hint"] = len(anchors)
    g["layer"] = LAYER.get(g["trigger"], "Observability")

# --- brief -------------------------------------------------------------
print(f"cards: {len(new_cards)}")
print(f"candidates: {len(order)}")
print("note: recurrence_hint 是去重锚点数,不是跨 goal 计数;草稿 TODO 未消除前不得保存为 FLAW-NNN.md")

flaw_n = flaw_start
for i, k in enumerate(order, 1):
    g = groups[k]
    print()
    print(f"## C{i} {g['trigger']} · recurrence_hint={g['hint']} · layer={g['layer']}")
    print(f"key: {k}")
    print("anchors: " + ", ".join(g["anchors"]))
    print("cards: " + ", ".join(c["id"] for c in g["cards"]))
    for c in g["cards"]:
        sym = c["symptom"][:120]
        print(f"- {c['id']} | source={c['source']} | envelope={c['envelope']} | symptom={sym}")
    print()
    print("```yaml")
    print("draft: true")
    print("kind: context")
    print(f"id: FLAW-{flaw_n:03d}")
    print(f'title: "{g["gt"][:60]}"')
    print("repo: TODO")
    print(f"layers: [{g['layer']}]")
    print("status: open")
    print("severity: TODO")
    print(f"recurrence: {g['hint']}")
    print("fingerprint: TODO")
    print("cards: [" + ", ".join(c["id"] for c in g["cards"]) + "]")
    print("```")
    flaw_n += 1

# summary on fd 3
os.write(3, f"SUMMARY|{len(new_cards)}|{len(order)}|{max_seen}\n".encode())
for m in malformed:
    os.write(3, f"MALFORMED|{m}\n".encode())
PY
}

# --- main --------------------------------------------------------------
LAST_ID=$(read_last_id)
FLAW_START=$(next_flaw_number "$REPO_ROOT/knowledge/context/evolution")

if [ "$DRY_RUN" -eq 1 ]; then
    # Dry-run: brief to stdout, malformed notes to stderr, ZERO file
    # creation (no lock, no state dir, no retro dir).
    SUMFILE=$(mktemp)
    compute_brief "$EVO_BOARD" "$LAST_ID" "$REAL_REPO" "$FLAW_START" 3>"$SUMFILE"
    while IFS= read -r m; do
        echo "malformed-card: $m" >&2
    done < <(grep '^MALFORMED|' "$SUMFILE" | cut -d'|' -f2 || true)
    rm -f "$SUMFILE"
    exit 0
fi

mkdir -p "$STATE_DIR" 2>/dev/null || die "cannot create $STATE_DIR"
exec 8>"$RETRO_LOCK"
flock -x 8

LAST_ID=$(read_last_id)  # re-read under lock

OUT=$(mktemp)
compute_brief "$EVO_BOARD" "$LAST_ID" "$REAL_REPO" "$FLAW_START" 3>"$OUT.summary" 2>"$OUT.err" >"$OUT.brief"

NEW_CARDS=$(sed -n 's/^SUMMARY|\([0-9]*\)|.*/\1/p' "$OUT.summary" | head -1)
CANDIDATES=$(sed -n 's/^SUMMARY|[0-9]*|\([0-9]*\)|.*/\1/p' "$OUT.summary" | head -1)
MAX_ID=$(sed -n 's/^SUMMARY|[0-9]*|[0-9]*|\([0-9]*\)$/\1/p' "$OUT.summary" | head -1)
while IFS= read -r m; do
    echo "malformed-card: $m" >&2
done < <(grep '^MALFORMED|' "$OUT.summary" | cut -d'|' -f2 || true)

if [ "$NEW_CARDS" = "0" ]; then
    echo "retro: 0 new card(s)"
    rm -f "$OUT" "$OUT.brief" "$OUT.err" "$OUT.summary"
    exit 0
fi

mkdir -p "$RETRO_DIR" 2>/dev/null || die "cannot create $RETRO_DIR"

RUNS_LEN=$(read_runs_len)
RUN=$((RUNS_LEN + 1))
BRIEF_PATH="$RETRO_DIR/$RETRO_TS-$RUN.md"

cp "$OUT.brief" "$BRIEF_PATH.tmp"
{ printf '# retro %s · %s · run %s\n\n' "$RETRO_TS" "$PROJECT_KEY" "$RUN"; cat "$BRIEF_PATH.tmp"; } > "$BRIEF_PATH.tmp2"
sync
mv -f "$BRIEF_PATH.tmp2" "$BRIEF_PATH"
rm -f "$BRIEF_PATH.tmp"

# fault injection: after brief, before retro.json
if [ "${OLP_EVO_TEST:-0}" = "1" ] && [ "${OLP_EVO_FAULT:-}" = "after-brief" ]; then
    echo "fault-injected: after-brief" >&2
    exit 70
fi

# append run + rewrite retro.json atomically
python3 - "$RETRO_JSON" "$MAX_ID" "$RETRO_TS" "$RUN" "$NEW_CARDS" "$CANDIDATES" "$BRIEF_PATH" <<'PY'
import json, os, sys, tempfile

path, max_id, ts, run, cards, candidates, brief = sys.argv[1:8]
max_id, run, cards, candidates = int(max_id), int(run), int(cards), int(candidates)

runs = []
if os.path.isfile(path):
    try:
        runs = json.load(open(path)).get("runs", [])
    except Exception:
        runs = []
runs.append({
    "ts": ts, "run": run, "cards": cards,
    "candidates": candidates, "brief": os.path.abspath(brief),
})
state = {"last_id": max_id, "runs": runs}

d = os.path.dirname(path)
fd, tmp = tempfile.mkstemp(dir=d, prefix="retro.json.tmp.")
with os.fdopen(fd, "w") as f:
    json.dump(state, f)
    f.flush()
    os.fsync(f.fileno())
os.replace(tmp, path)
PY

rm -f "$OUT" "$OUT.brief" "$OUT.err" "$OUT.summary"
echo "retro: $NEW_CARDS new card(s), $CANDIDATES candidate(s), brief: $BRIEF_PATH"
exit 0
