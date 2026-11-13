#!/usr/bin/env bash
# olp-evo-harvest.sh — evolution loop phase-0 harvest (#41, SDD contract
# specs/task-req-olp-evo-p0.spec.md). Read-only shadow pilot: collects
# friction signals from three sources and appends identity-stable symptom
# cards to the evolution board. Never writes the review board.
#
# Usage: olp-evo-harvest.sh <repo-root> [--dry-run]
# Exit codes: 2 board missing; 70 injected fault; 0 ok; 1 other errors.
set -euo pipefail

# #12c: pin EVERY python3 child to UTF-8 regardless of LC_ALL / PYTHONUTF8
# inherited from the caller. Under LC_ALL=C + PYTHONUTF8=0, open() reads
# and stdout default to ASCII and the state-commit pass dies decoding the
# very cards this script wrote (EVOLUTION.md is UTF-8 by contract). This
# export overrides a hostile PYTHONUTF8=0 and needs no installed locale.
export PYTHONUTF8=1

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

# --- #12c capability preflight (all platforms) ---------------------------
# Production launches must not depend on the caller remembering
# scripts/verify.sh (or its PATH). EVERY required capability is validated
# before ANY write: GNU `stat -c '%s'`, GNU `realpath -m`, `sha256sum`,
# `flock`, and `python3` (utf8 mode). On Darwin only, a failed probe first
# tries discovery consistent with verify.sh — brew prefix, then the stock
# /opt/homebrew and /usr/local coreutils gnubin dirs (nothing installed,
# no override knob); Linux/other platforms take the PATH as-is so a
# missing tool (e.g. flock) still fails here. Unusable after discovery ⇒
# exit 2 with actionable guidance BEFORE state mkdir, lock, or any board
# write.
gnu_probe_succeeds() {
    command -v stat >/dev/null 2>&1 \
        && stat -c '%s' /dev/null >/dev/null 2>&1 \
        && command -v realpath >/dev/null 2>&1 \
        && realpath -m /dev/null >/dev/null 2>&1 \
        && sha256sum /dev/null >/dev/null 2>&1 \
        && command -v flock >/dev/null 2>&1 \
        && python3 -X utf8 -c 'import sys' >/dev/null 2>&1
}

if ! gnu_probe_succeeds; then
    if [ "$(uname -s)" = "Darwin" ]; then
        gnubin=""
        # Prefer the active brew prefix (any arch / custom location). The
        # assignment is guarded so a failing brew cannot trip set -e.
        if command -v brew >/dev/null 2>&1; then
            candidate="$(brew --prefix coreutils 2>/dev/null || true)"
            candidate="$candidate/libexec/gnubin"
            [ -d "$candidate" ] && gnubin="$candidate"
        fi
        if [ -z "$gnubin" ]; then
            for prefix in /opt/homebrew /usr/local; do
                candidate="$prefix/opt/coreutils/libexec/gnubin"
                if [ -d "$candidate" ]; then
                    gnubin="$candidate"
                    break
                fi
            done
        fi
        if [ -n "$gnubin" ]; then
            PATH="$gnubin:$PATH"
            export PATH
        fi
    fi
    # Re-probe AFTER discovery: only now is availability decided.
    if ! gnu_probe_succeeds; then
        echo "error: required tools not usable: GNU stat -c / realpath -m / sha256sum, flock, python3." >&2
        echo "  Run via scripts/verify.sh, or install once: brew install coreutils" >&2
        exit 2
    fi
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

# Hub guard BEFORE any mkdir/flock/board write: on a multi-machine
# deployment only the hub harvests (the EVO counter and the byte cursors
# are per-machine state). Unconfigured = single machine = no-op. A dry
# run writes nothing and stays available everywhere. Exit 3 off-hub.
if [ "$DRY_RUN" -eq 0 ]; then
    if [ -f "$(dirname "$0")/olp-hub-guard.sh" ]; then
        . "$(dirname "$0")/olp-hub-guard.sh"
        olp_hub_guard olp-evo-harvest || exit 3
    elif [ -n "${OLP_HUB_HOST:-}" ] \
        || [ -f "${OLP_HUB_HOST_FILE:-$HOME/.octos/outer/hub-host}" ]; then
        # A lone copy of this script without the guard library: a hub is
        # configured but cannot be checked, so refuse rather than guess.
        echo "error: olp-evo-harvest: hub 已配置,但找不到 olp-hub-guard.sh,未写入任何内容。" >&2
        exit 3
    fi
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

# #12c: truncate by UNICODE CHARACTERS, independent of LC_ALL. Bash
# ${s:0:N} counts locale "characters" — under LC_ALL=C those are BYTES and
# the cut splits a UTF-8 sequence mid-character, producing an EVOLUTION.md
# that does not decode. python3 (already a hard dependency of this script)
# slices by code points in every locale; -X utf8 pins argv/stdout encoding
# so PYTHONUTF8=0 / PYTHONCOERCE_C_LOCALE=0 cannot restore locale dependence.
char_prefix() { # text limit
    # Fast path: no truncation needed when the text already fits. ${#s}
    # counts bytes under LC_ALL=C and characters under UTF-8 locales —
    # byte count upper-bounds character count, so shell-length <= limit
    # guarantees the text fits in BOTH regimes; returning it whole is
    # exact and avoids a python spawn per short card (a 400-card burst
    # would otherwise pay 400 interpreter startups inside the watcher's
    # polling window).
    if [ "${#1}" -le "$2" ]; then
        printf '%s' "$1"
        return 0
    fi
    python3 -X utf8 -c 'import sys; sys.stdout.write(sys.argv[1][:int(sys.argv[2])])' "$1" "$2"
}

file_stat() { stat -c '%d %i' "$1" 2>/dev/null || echo "0 0"; }

# prefix sha256 over [max(0,offset-64), offset)
prefix_sha() { # path offset
    local path=$1 offset=$2 start
    start=$(( offset > 64 ? offset - 64 : 0 ))
    dd if="$path" bs=1 skip="$start" count=$(( offset - start )) 2>/dev/null | sha256sum | cut -d' ' -f1
}

json_get() { # json key -> value via python3 stdlib
    python3 -B - "$1" "$2" <<'PY'
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
        # Cluster forms. In a multi-machine deployment a unit cannot write
        # under an entry: its ACK, the outer loop's return and the unit's
        # negative-sentinel lines are appended at the end of the board and
        # name their entry as #N. They attach to THAT entry, not to the
        # heading they happen to sit under.
        local card_entry=$entry
        if [ -z "$trigger" ]; then
            local re_inner='^内环\([^)]+\)·(#([0-9]+)[[:space:]]*)?ACK\((blocked|wontdo)\)'
            local re_inner_tail='ACK\((blocked|wontdo)\)(\([^)]*\))?(:|：)[[:space:]]*#([0-9]+)'
            local re_returned='^外环\([^)]+\)·打回\(#([0-9]+)\)'
            local re_alert='^单元\([^)]+\)·负哨\(#?([^)]*)\)'
            if [[ $signed =~ $re_inner ]]; then
                # Read the captures before another =~ overwrites them; an
                # unmatched optional group is unset under `set -u`.
                trigger=ack_${BASH_REMATCH[3]}
                local inner_n=${BASH_REMATCH[2]:-}
                if [ -n "$inner_n" ]; then
                    card_entry=$inner_n
                elif [[ $signed =~ $re_inner_tail ]]; then
                    card_entry=${BASH_REMATCH[4]}
                fi
                rest=$trimmed
            elif [[ $signed =~ $re_returned ]]; then
                trigger=returned
                card_entry=${BASH_REMATCH[1]}
            elif [[ $signed =~ $re_alert ]]; then
                trigger=unit_alert
                local alert_task=${BASH_REMATCH[1]:-}
                if [[ $alert_task =~ ^[0-9]+$ ]]; then
                    card_entry=$alert_task
                fi
            fi
        fi
        # A unit's MCP audit lines (report_blocked, an ask_outer that timed
        # out) reach the hub's board signed by the unit. They are the same
        # signals the MCP-board source yields on a single machine, so they
        # become the same triggers with the same symptom shape.
        if [ -z "$trigger" ]; then
            local re_unit_mcp='^单元\(([^)]+)\)·-[[:space:]]+([^[:space:]]+[[:space:]]+[^[:space:]]+)[[:space:]]+MCP\(ask_outer\)[[:space:]]+(blocked|timeout):[[:space:]]+(.*)$'
            if [[ $signed =~ $re_unit_mcp ]]; then
                local mcp_unit=${BASH_REMATCH[1]} mcp_kind=${BASH_REMATCH[3]} mcp_detail=${BASH_REMATCH[4]}
                local mcp_id='-'
                if [[ $mcp_detail =~ id=([0-9a-fA-F]+) ]]; then
                    mcp_id=${BASH_REMATCH[1]}
                fi
                local mcp_reason
                mcp_reason=$(python3 -c '
import re, sys
r = sys.argv[1][sys.argv[1].find("reason=") + len("reason="):]
m = re.search(r"\s[a-z_]+=", r)
print((r[: m.start()] if m else r)[:80])
' "$mcp_detail")
                local mcp_trigger=ask_outer_timeout
                [ "$mcp_kind" = blocked ] && mcp_trigger=report_blocked
                lsha=$(printf '%s' "$line" | sha256sum | cut -d' ' -f1)
                add_candidate "$mcp_trigger" review "$rp" "board:$rp#unit:$mcp_unit#mcp_$mcp_kind#$lsha" \
                    "$line_no" "$line_off" "$(now_rfc3339)" \
                    "kind=$mcp_kind id=$mcp_id unit=$mcp_unit reason=$(char_prefix "$mcp_reason" 80)"
            fi
        fi
        if [ -n "$trigger" ]; then
            lsha=$(printf '%s' "$line" | sha256sum | cut -d' ' -f1)
            local kind=blocked
            [ "$trigger" = ack_wontdo ] && kind=wontdo
            [ "$trigger" = override ] && kind=override
            [ "$trigger" = r2_record ] && kind=r2
            [ "$trigger" = returned ] && kind=returned
            [ "$trigger" = unit_alert ] && kind=alert
            ident="board:$rp#$card_entry#$kind#$lsha"
            # signed forms report the ORIGINAL line (contract: symptom 取原行前 200 字符)
            local symptom_base=${rest:-$trimmed}
            case $trigger in override|r2_record|returned|unit_alert) symptom_base=$trimmed ;; esac
            # #12c: cut by Unicode characters, not locale bytes (LC_ALL=C
            # would otherwise split a UTF-8 sequence mid-character).
            local symptom
            symptom=$(char_prefix "$symptom_base" 200)
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
    OLP_EVO_HARVEST_TS="$HARVEST_TS" OLP_EVO_EVENTS_OUT="/tmp/.olp_evo_events_out.$$" python3 -B - "$EVENTS" "$rp" <<'PY' || true
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
    if kind in ("escalation", "turn_error", "fallback_switch", "malformed_exhausted"):
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
            symptom="kind=$kind id=$ask_id reason=$(char_prefix "$reason" 80)"
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
    python3 -B - "$1" "$2" <<'PY'
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
                # #12c: extract LEADING DIGITS portably. The previous
                # ${local_id%%[$'\uff08' ]*} used a \uXXXX escape Bash 3.2
                # (Darwin /bin/bash) does not support: it produced the
                # literal 6-char string '\uff08', the fullwidth （ was
                # never stripped, and reconcile broke. A regex capture of
                # leading digits works on every bash this script runs under.
                local_id=${line#\#\#\# EVO-}
                if [[ $local_id =~ ^([0-9]+) ]]; then
                    local_id=${BASH_REMATCH[1]}
                else
                    local_id=""
                fi
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
    # 44-r1: harvest may be invoked from an installed watcher in an
    # unrelated dir — locate the append helper next to the HARVEST script
    # (i.e. the repo's scripts/) when the copy isn't beside us.
    _append_sh="$(dirname "$0")/olp-board-append.sh"
    if [ ! -f "$_append_sh" ]; then
        _append_sh="$(pwd)/scripts/olp-board-append.sh"
    fi
    printf '%s' "$APPEND_TEXT" | "$_append_sh" "$EVO_BOARD" >/dev/null 2>&1 || true
    echo "fault-injected: after-append" >&2
    exit 70
fi

if [ -n "$APPEND_TEXT" ]; then
    _append_sh="$(dirname "$0")/olp-board-append.sh"
    if [ ! -f "$_append_sh" ]; then
        _append_sh="$(pwd)/scripts/olp-board-append.sh"
    fi
    printf '%s' "$APPEND_TEXT" | "$_append_sh" "$EVO_BOARD"
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

python3 -B - "$STATE_FILE" "$NEXT_ID" "$BOARD" "$BOARD_SIZE" "$EVENTS" "$EVENTS_SIZE" "$MCP_BOARD" "$MCP_SIZE" <<'PY'
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
