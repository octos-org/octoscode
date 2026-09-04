#!/usr/bin/env bash
# olp-evo-harvest.sh — evolution loop phase-0 harvest (#41, SDD contract
# specs/task-req-olp-evo-p0.spec.md). Read-only shadow pilot: collects
# friction signals from three sources and appends identity-stable symptom
# cards to the evolution board. Never writes the review board.
#
# Usage: olp-evo-harvest.sh <repo-root> [--dry-run]
# Exit codes: 2 board missing; 70 injected fault; 0 ok; 1 other errors.
set -euo pipefail

REPO_ROOT="${1:?usage: olp-evo-harvest.sh <repo-root> [--dry-run]}"
DRY_RUN=0
[ "${2:-}" = "--dry-run" ] && DRY_RUN=1

BOARD="${OLP_EVO_REVIEW_BOARD:-$REPO_ROOT/.octos/OUTER_LOOP_REVIEW.md}"
EVENTS="${OLP_EVO_EVENTS:-}"
MCP_BOARD="${OLP_EVO_MCP_BOARD:-$HOME/.octos/outer/OUTER_LOOP_MCP.md}"
EVO_BOARD="${OLP_EVO_BOARD:-$REPO_ROOT/.octos/EVOLUTION.md}"
STATE_ROOT="${OLP_EVO_STATE:-$HOME/.octos/outer/evo}"

# 41-r5a ⑤: a missing repo-root must exit 2 with zero side effects —
# realpath would otherwise die (exit 1) under set -e before the board check.
if [ ! -d "$REPO_ROOT" ] && [ ! -f "$REPO_ROOT" ]; then
    echo "error: repo-root not found: $REPO_ROOT" >&2
    exit 2
fi
REAL_REPO=$(realpath "$REPO_ROOT")
PROJECT_KEY=$(printf '%s' "$REAL_REPO" | sha256sum | cut -c1-16)
STATE_DIR="$STATE_ROOT/$PROJECT_KEY"
STATE_FILE="$STATE_DIR/state.json"
LOCK_FILE="$STATE_DIR/harvest.lock"

# ① Board existence precondition BEFORE any mkdir/flock (contract commit
# protocol step ①): missing board = exit 2, zero side effects.
if [ ! -f "$BOARD" ]; then
    echo "error: review board not found: $BOARD" >&2
    exit 2
fi

if [ "$DRY_RUN" -eq 1 ]; then
    # Dry run: read-only across board/sources/state; print pending cards.
    # No state dir/lock/board creation or modification.
    :
fi

die() { echo "error: $*" >&2; exit 1; }
skip() { echo "skip: $*" >&2; }

# --- helpers -----------------------------------------------------------
# 41-r4: sample the collection timestamp ONCE per run — the card title
# and the envelope ts must be identical even when a second boundary passes
# between candidate creation and emission (CI caught the double-sample).
HARVEST_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
now_rfc3339() { printf '%s' "$HARVEST_TS"; }

hex_of() { printf '%s' "$1" | sha256sum | cut -d' ' -f1; }

file_stat() { stat -c '%d %i' "$1" 2>/dev/null || echo "0 0"; }

# prefix sha256 over [max(0,offset-64), offset)
prefix_sha() { # path offset
    local path=$1 offset=$2 start
    start=$(( offset > 64 ? offset - 64 : 0 ))
    dd if="$path" bs=1 skip="$start" count=$(( offset - start )) 2>/dev/null | sha256sum | cut -d' ' -f1
}

json_get() { # json key -> value via python3 stdlib
    python3 - "$1" "$2" <<'PY'
import json, sys
try:
    d = json.loads(sys.argv[1])
except Exception:
    sys.exit(0)
def walk(o):
    if isinstance(o, dict):
        for k, v in o.items():
            if k == sys.argv[2]:
                print(v if isinstance(v, str) else json.dumps(v))
                return True
            if walk(v):
                return True
    elif isinstance(o, list):
        for item in o:
            if walk(item):
                return True
    return False
walk(d)
PY
}

# --- candidate collection ----------------------------------------------
# Each candidate: TRIGGER|SOURCE|REALPATH|IDENTITY|LINE|OFFSET|TS|SYMPTOM
CANDIDATES=""

add_candidate() { # trigger source realpath identity line offset ts symptom
    CANDIDATES+="$1|$2|$3|$4|$5|$6|$7|$8"$'\n'
}

# Review board: leading-whitespace-stripped lines starting ACK(blocked): /
# ACK(wontdo): under the nearest preceding ### <number> heading.
harvest_board() { # realpath
    # 41-r5b ⑦: only newline-TERMINATED lines fire (REQ-OLP-EVO-CURSOR);
    # the trailing fragment (read fills $line but rc!=0) is skipped.
    local rp=$1 line_no=0 entry=-1 trimmed trigger rest lsha ident byte_off=0
    while IFS= read -r line; do
        line_no=$((line_no + 1))
        local line_off=$byte_off
        # byte accumulation covers EVERY newline-terminated line — empty
        # separator lines included (they still advance the offset).
        byte_off=$((byte_off + $(printf '%s' "$line" | wc -c) + 1))
        if [[ $line =~ ^\#\#\#[[:space:]]+([0-9]+) ]]; then
            entry=${BASH_REMATCH[1]}
        fi
        trimmed=${line#"${line%%[![:space:]]*}"}
        trigger=""
        case $trimmed in
            'ACK(blocked):'*|'ACK(blocked)：'*) trigger=ack_blocked; rest=$trimmed ;;
            'ACK(wontdo):'*|'ACK(wontdo)：'*) trigger=ack_wontdo; rest=$trimmed ;;
        esac
        # #42a: SIGNED override / R2-record line forms (phase-1). A COPY of
        # the line is stripped of leading `> ` (repeatable), `**` and
        # whitespace, then matched against the signed line-start forms;
        # the phase-0 ACK detection above is untouched.
        local signed=${trimmed}
        while :; do
            case $signed in
                '> '*) signed=${signed#'> '} ;;
                '**'*) signed=${signed#'**'} ;;
                *) break ;;
            esac
        done
        signed=${signed#"${signed%%[![:space:]]*}"}
        if [ -z "$trigger" ]; then
            local re_override='^外环\([^)]+\)·改判\('
            local re_r2='^外环\([^)]+\)·R2 记档\('
            if [[ $signed =~ $re_override ]]; then
                trigger=override
            elif [[ $signed =~ $re_r2 ]]; then
                trigger=r2_record
            fi
        fi
        if [ -n "$trigger" ]; then
            lsha=$(printf '%s' "$line" | sha256sum | cut -d' ' -f1)
            local kind=blocked
            [ "$trigger" = ack_wontdo ] && kind=wontdo
            [ "$trigger" = override ] && kind=override
            [ "$trigger" = r2_record ] && kind=r2
            ident="board:$rp#$entry#$kind#$lsha"
            # signed forms report the ORIGINAL line (contract: symptom 取原行前 200 字符)
            local symptom_base=${rest:-$trimmed}
            case $trigger in override|r2_record) symptom_base=$trimmed ;; esac
            local symptom=${symptom_base:0:200}
            add_candidate "$trigger" review "$rp" "$ident" "$line_no" "$line_off" "$(now_rfc3339)" "$symptom"
        fi
    done < "$BOARD"
}

# Events: python3 json.loads per line; escalation/turn_error fire;
# goal_transition fires only on the two exact detail strings.
harvest_events() { # realpath
    local rp=$1
    [ -z "$EVENTS" ] && return 0
    [ -f "$EVENTS" ] || { skip "$EVENTS"; return 0; }
    OLP_EVO_HARVEST_TS="$HARVEST_TS" OLP_EVO_EVENTS_OUT="/tmp/.olp_evo_events_out.$$" python3 - "$EVENTS" "$rp" <<'PY' || true
import hashlib, json, sys, datetime

path, rp = sys.argv[1], sys.argv[2]
with open(path, "rb") as f:
    data = f.read()
lines = data.decode("utf-8", "replace").split("\n")
# a trailing incomplete line (no newline) must not fire
if data and not data.endswith(b"\n"):
    lines = lines[:-1]
import os
out_path = os.environ.get("OLP_EVO_EVENTS_OUT", "/tmp/.olp_evo_events_out")
open(out_path, "w").close()
byte_off = 0
for i, line in enumerate(lines):
    line_off = byte_off
    byte_off += len(line.encode("utf-8")) + 1
    if not line.strip():
        continue
    try:
        d = json.loads(line)
    except Exception:
        sys.stderr.write(f"malformed: {path}:{i+1}\n")
        continue
    # 41-r5b ⑥: valid JSON that is NOT an object (null/list/number) would
    # crash d.get below (AttributeError, previously swallowed by || true
    # and silently dropping every later event) — treat as malformed.
    if not isinstance(d, dict):
        sys.stderr.write(f"malformed: {path}:{i+1}\n")
        continue
    kind = d.get("kind", "")
    trigger = None
    if kind in ("escalation", "turn_error"):
        trigger = kind
    elif kind == "goal_transition":
        detail = d.get("detail", "")
        if detail == "goal transitioned to `blocked`":
            trigger = "goal_blocked"
        elif detail == "goal transitioned to `budget_limited`":
            trigger = "goal_budget_limited"
    if not trigger:
        continue
    # 41-r5a ③: a missing ts must NOT bake now() into the identity (a
    # fresh stamp per run means the event never dedups). Identity uses
    # '-'; the line sha256 still distinguishes events.
    ts = d.get("ts", "") or "-"
    ref = d.get("goal_id") or d.get("slug") or d.get("session") or "-"
    lsha = hashlib.sha256(line.encode()).hexdigest()
    ident = f"events:{rp}#{ts}#{kind}#{ref}#{lsha}"
    symptom = line[:200]
    with open(out_path, "a") as out:
        harvest_ts = os.environ.get("OLP_EVO_HARVEST_TS", "")
        out.write(f"{trigger}|events|{rp}|{ident}|{i+1}|{line_off}|{harvest_ts}|{symptom}\n")
PY
}

# MCP audit board: regex-parsed lines; blocked→report_blocked,
# timeout→ask_outer_timeout; symptom never copies question/context/tried.
harvest_mcp() { # realpath
    local rp=$1
    [ -f "$MCP_BOARD" ] || { skip "$MCP_BOARD"; return 0; }
    # 41-r5b ⑦: same newline-terminated rule as the board/events.
    local line_no=0 ts kind detail ask_id trigger symptom lsha ident byte_off=0
    while IFS= read -r line; do
        line_no=$((line_no + 1))
        local line_off=$byte_off
        byte_off=$((byte_off + $(printf '%s' "$line" | wc -c) + 1))
        if [[ $line =~ ^-\ ([^[:space:]]+\ [^[:space:]]+)\ MCP\(ask_outer\)\ (blocked|timeout):\ (.*)$ ]]; then
            ts=${BASH_REMATCH[1]}
            kind=${BASH_REMATCH[2]}
            detail=${BASH_REMATCH[3]}
            ask_id='-'
            if [[ $detail =~ id=([0-9a-fA-F]+) ]]; then
                ask_id=${BASH_REMATCH[1]}
            fi
            # 41-r5b ⑧: take up to 80 chars after `reason=`, stopping at
            # the next ` key=` token or end of detail — not just one word.
            # 41-r5b ⑧: reason = up to 80 chars after `reason=`, stopping
            # at the NEXT ` key=` token. Bash ERE lacks lazy quantifiers
            # (glob `%% [a-z_]*=*` eats to the FIRST word boundary), so
            # cut with python (same interpreter the script already uses).
            local reason
            reason=$(printf '%s' "$detail" | python3 -c '
import re, sys
r = sys.argv[1][sys.argv[1].find("reason=") + len("reason="):]
m = re.search(r"\s[a-z_]+=", r)
print((r[: m.start()] if m else r)[:80])
' "$detail")
            if [ "$kind" = blocked ]; then trigger=report_blocked; else trigger=ask_outer_timeout; fi
            lsha=$(printf '%s' "$line" | sha256sum | cut -d' ' -f1)
            ident="mcp:$rp#$ts#$kind#$ask_id#$lsha"
            symptom="kind=$kind id=$ask_id reason=${reason:0:80}"
            add_candidate "$trigger" mcp "$rp" "$ident" "$line_no" "$line_off" "$(now_rfc3339)" "$symptom"
        fi
    done < "$MCP_BOARD"
}

# --- offsets & change detection ----------------------------------------
# Returns "<offset> <dev> <ino> <prefix_sha>" for a source key from the
# state FILE (41-r5a ①: the caller previously passed the JSON *text* while
# this helper treated it as a path — the bare except swallowed the error,
# so prev/dev/ino/prefix were never read and every rerun printed reset:).
source_state() { # state_file source_key
    python3 - "$1" "$2" <<'PY'
import json, sys
try:
    with open(sys.argv[1]) as f:
        d = json.load(f)
except (OSError, ValueError) as exc:
    sys.stderr.write(f"state-read-failed: {exc}\n")
    sys.exit(0)
src = d.get("sources", {}).get(sys.argv[2])
if src:
    print(src.get("offset", 0), src.get("dev", 0), src.get("ino", 0), src.get("prefix_sha256", ""))
PY
}

compute_offset() { # path previous_offset -> effective start offset
    local path=$1 prev=${2:-0}
    [ -f "$path" ] || { echo 0; return; }
    local size
    size=$(stat -c '%s' "$path")
    # only complete newline-terminated records count
    local complete=$size
    if [ "$size" -gt 0 ]; then
        local last_byte
        last_byte=$(dd if="$path" bs=1 skip=$((size - 1)) count=1 2>/dev/null | od -An -tuC | tr -d ' ')
        [ "$last_byte" != "10" ] && complete=0 # partial tail: no new complete boundary this run
        :
    fi
    echo "$complete"
}

# --- main --------------------------------------------------------------
if [ "$DRY_RUN" -eq 0 ]; then
    mkdir -p "$STATE_DIR" 2>/dev/null || die "cannot create state dir $STATE_DIR"
    exec 9>"$LOCK_FILE"
    flock -x 9
fi

# initial state (or empty)
NEXT_ID=1
SEEN_LIST=""
if [ -f "$STATE_FILE" ]; then
    NEXT_ID=$(json_get "$(cat "$STATE_FILE")" next_id)
    NEXT_ID=${NEXT_ID:-1}
fi

# Reconcile with the evolution board (authoritative): recover next_id and
# seen identities from what actually landed. Board identities AND the
# state's stored seen set (both sha256(identity)) feed the same list.
if [ -f "$STATE_FILE" ]; then
    while IFS= read -r seen_sha; do
        [ -n "$seen_sha" ] && SEEN_LIST+="$seen_sha"$'\n'
    done < <(python3 -c '
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    print("\n".join(d.get("seen", [])))
except Exception:
    pass
' "$STATE_FILE")
fi
if [ -f "$EVO_BOARD" ]; then
    while IFS= read -r line; do
        case $line in
            '### EVO-'*)
                # The id is followed directly by a fullwidth （, not a
                # space — slice NUMERIC PREFIX ONLY (arithmetic on the
                # raw remainder is a syntax error that would abort the
                # whole reconcile loop under set -e).
                local_id=${line#\#\#\# EVO-}
                local_id=${local_id%%[$'\uff08' ]*}
                # 41-r5a ④: a non-numeric id (e.g. `### EVO-draft`) must be
                # skipped with a warning, never abort the reconcile loop
                # (`value too great for base` kills set -e scripts).
                if ! [[ $local_id =~ ^[0-9]+$ ]]; then
                    echo "warn: non-numeric EVO id ignored in reconcile: $line" >&2
                    continue
                fi
                local_id=$((10#$local_id))
                if [ "$local_id" -ge "$NEXT_ID" ]; then
                    NEXT_ID=$((local_id + 1))
                fi
                ;;
            identity:*)
                # 41-r5a ②: state.seen stores sha256(identity) — normalize
                # board-side identities into the same space for compare.
                SEEN_LIST+="$(printf '%s' "${line#identity: }" | sha256sum | cut -d' ' -f1)"$'\n'
                ;;
        esac
    done < "$EVO_BOARD"
fi

seen_contains() {
    # 41-r5a ②: compare in sha256 space (state stores hashes).
    local want
    want=$(printf '%s' "${1%$'\r'}" | sha256sum | cut -d' ' -f1)
    local list=$SEEN_LIST
    # trim leading/trailing newlines so here-string does not yield empty rows
    while [ -n "$list" ] && [ "${list:0:1}" = $'\n' ]; do list=${list#?}; done
    while [ -n "$list" ] && [ "${list: -1}" = $'\n' ]; do list=${list%?}; done
    [ -z "$list" ] && return 1
    local line
    while IFS= read -r line; do
        line=${line%$'\r'}
        [ "$line" = "$want" ] && return 0
    done <<EOF
$list
EOF
    return 1
}

# Reset detection + candidate harvesting per source
BOARD_RP=$(realpath "$BOARD")
collect_source() { # source_key path harvest_fn
    local key=$1 path=$2 fn=$3
    [ -z "$path" ] && return 0
    if [ "$key" != review ] && [ ! -f "$path" ]; then
        skip "$path"
        return 0
    fi
    local prev=0 prev_dev=0 prev_ino=0 prev_prefix=""
    if [ -f "$STATE_FILE" ]; then
        read -r prev prev_dev prev_ino prev_prefix <<<"$(source_state "$STATE_FILE" "$key")"
    fi
    local rp
    rp=$(realpath "$path")
    local cur_dev cur_ino size
    read -r cur_dev cur_ino <<<"$(file_stat "$path")"
    size=$(stat -c '%s' "$path" 2>/dev/null || echo 0)
    if [ "$prev" != 0 ]; then
        if [ "$size" -lt "$prev" ] || [ "$cur_dev" != "$prev_dev" ] || [ "$cur_ino" != "$prev_ino" ]; then
            echo "reset: $path" >&2
            prev=0
        elif [ -n "$prev_prefix" ] && [ "$prev_prefix" != "$(prefix_sha "$path" "$prev")" ]; then
            echo "reset: $path" >&2
            prev=0
        fi
    fi
    "$fn" "$rp" "$prev"
}

# Harvest runs read the FULL source each time; offset bookkeeping below
# filters candidates whose line-number start-byte is before the cursor.
# For simplicity and crash-consistency, candidates from lines whose first
# byte < prev offset are dropped at commit time.

collect_source review "$BOARD" harvest_board
collect_source events "$EVENTS" harvest_events
collect_source mcp "$MCP_BOARD" harvest_mcp

# Pull events candidates written by the python helper.
EVT_OUT="/tmp/.olp_evo_events_out.$$"
if [ -f "$EVT_OUT" ]; then
    while IFS= read -r line; do
        CANDIDATES+="$line"$'\n'
    done <"$EVT_OUT"
    rm -f "$EVT_OUT"
fi

# --- emit cards ---------------------------------------------------------
emit_card() { # id trigger source realpath identity line ts symptom
    printf '### EVO-%04d（%s，harvest）\ntrigger: %s\nsource: %s %s\nidentity: %s\nenvelope: line=%s offset=%s ts=%s\nsymptom: %s\n' \
        "$((10#$1))" "$(now_rfc3339)" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9"
}

APPEND_TEXT=""
NEW_IDS=0
while IFS='|' read -r trigger source rp ident line offset ts symptom; do
    [ -z "$trigger" ] && continue
    if seen_contains "$ident"; then continue; fi
    card_id=$(printf '%04d' "$NEXT_ID")
    card=$(emit_card "$card_id" "$trigger" "$source" "$rp" "$ident" "$line" "${offset:-0}" "$ts" "$symptom")
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '%s\n' "$card"
    else
        APPEND_TEXT+="$card"$'\n'
        SEEN_LIST+="$ident"$'\n'
    fi
    NEXT_ID=$((NEXT_ID + 1))
    NEW_IDS=$((NEW_IDS + 1))
done <<<"$CANDIDATES"

if [ "$DRY_RUN" -eq 1 ]; then
    exit 0
fi

# Fault injection (tests only): after appending all cards, before state.
if [ "${OLP_EVO_TEST:-0}" = "1" ] && [ "${OLP_EVO_FAULT:-}" = "after-append" ] && [ -n "$APPEND_TEXT" ]; then
    printf '%s' "$APPEND_TEXT" | "$(dirname "$0")/olp-board-append.sh" "$EVO_BOARD" >/dev/null 2>&1 || true
    echo "fault-injected: after-append" >&2
    exit 70
fi

if [ -n "$APPEND_TEXT" ]; then
    printf '%s' "$APPEND_TEXT" | "$(dirname "$0")/olp-board-append.sh" "$EVO_BOARD"
fi
export OLP_EVO_BOARD="$EVO_BOARD"

# --- commit state (atomic) ----------------------------------------------
BOARD_SIZE=$(stat -c '%s' "$BOARD")
EVENTS_SIZE=0
[ -n "$EVENTS" ] && [ -f "$EVENTS" ] && EVENTS_SIZE=$(stat -c '%s' "$EVENTS")
MCP_SIZE=0
[ -f "$MCP_BOARD" ] && MCP_SIZE=$(stat -c '%s' "$MCP_BOARD")

# Complete-record boundary: trailing partial line excluded from offset.
effective_size() { # path
    local path=$1 size last
    [ -f "$path" ] || { echo 0; return; }
    size=$(stat -c '%s' "$path")
    if [ "$size" -gt 0 ]; then
        last=$(dd if="$path" bs=1 skip=$((size - 1)) count=1 2>/dev/null | od -An -tuC | tr -d ' ')
        if [ "$last" != "10" ]; then
            # find last newline before EOF
            python3 -c "import sys;d=open(sys.argv[1],'rb').read();i=d.rfind(b'\n');print(i+1 if i>=0 else 0)" "$path"
            return
        fi
    fi
    echo "$size"
}

python3 - "$STATE_FILE" "$NEXT_ID" "$BOARD" "$BOARD_SIZE" "$EVENTS" "$EVENTS_SIZE" "$MCP_BOARD" "$MCP_SIZE" <<'PY'
import hashlib, json, os, sys, tempfile

state_path, next_id = sys.argv[1], int(sys.argv[2])
pairs = [(sys.argv[3], int(sys.argv[4]), "review"),
         (sys.argv[5], int(sys.argv[6]), "events"),
         (sys.argv[7], int(sys.argv[8]), "mcp")]

def prefix_sha(path, offset):
    start = max(0, offset - 64)
    with open(path, "rb") as f:
        f.seek(start)
        return hashlib.sha256(f.read(offset - start)).hexdigest()

def effective(path, size):
    if not path or not os.path.isfile(path):
        return 0
    with open(path, "rb") as f:
        f.seek(0, 2)
        total = f.tell()
        if total and not open(path, "rb").read()[-1:] == b"\n":
            f.seek(0)
            data = f.read()
            i = data.rfind(b"\n")
            return i + 1 if i >= 0 else 0
    return size if size else total

sources = {}

def sha(s):
    return hashlib.sha256(s.encode()).hexdigest()

seen = []
# carry over previous seen identities (state) ...
if os.path.isfile(state_path):
    try:
        seen = list(json.load(open(state_path)).get("seen", []))
    except Exception:
        pass
# ... and re-derive from the evolution board (authoritative ledger):
# every identity line ever appended is still seen.
if os.environ.get("OLP_EVO_BOARD"):
    try:
        for line in open(os.environ["OLP_EVO_BOARD"]):
            if line.startswith("identity: "):
                seen.append(sha(line[len("identity: "):].rstrip("\n")))
    except OSError:
        pass
# ... plus the cards appended in THIS run (passed via the ledger again).

for path, size, key in pairs:
    if not path or not os.path.isfile(path):
        continue
    off = effective(path, size)
    try:
        s = os.stat(path)
        dev, ino = s.st_dev, s.st_ino
    except OSError:
        dev, ino = 0, 0
    sources[key] = {
        "path": os.path.realpath(path),
        "offset": off,
        "dev": dev,
        "ino": ino,
        "prefix_sha256": prefix_sha(path, off) if off else "",
    }

state = {"next_id": next_id, "seen": sorted(set(seen)), "sources": sources}
d = os.path.dirname(state_path)
os.makedirs(d, exist_ok=True)
fd, tmp = tempfile.mkstemp(dir=d, prefix="state.json.tmp.")
with os.fdopen(fd, "w") as f:
    json.dump(state, f)
    f.flush()
    os.fsync(f.fileno())
os.replace(tmp, state_path)
PY

echo "harvested: $NEW_IDS new card(s), next_id=$NEXT_ID" >&2
exit 0
