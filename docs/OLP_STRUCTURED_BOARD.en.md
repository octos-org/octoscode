# OLP structured board (experimental, opt-in)

`schema="olp-board/v1"` adds a verifiable, append-only ledger to the existing
text OLP blackboard. It is an **explicit, experimental opt-in**: it does not
upgrade or replace the `olp/v2` protocol, and projects whose board carries no
event lines keep the legacy workflow unchanged. The repository's tracked
`.octos/loop.md` is not migrated. 中文版：[`OLP_STRUCTURED_BOARD.md`](OLP_STRUCTURED_BOARD.md).

## Install and choose a mode

Requires Python 3.9+, POSIX file semantics and `flock(2)`. In a **new project
without OLP files**, pick exactly one of the two modes; do not run the default
mode first and then try structured:

```bash
# Option A: default text board and hand-written ACK contract
bash /path/to/octoscode/scripts/olp-init.sh

# Option B (instead of A): structured loop, a board with the opt-in marker and
# no bare ACK placeholder, and an independent regular lock
OLP_BOARD_MODE=structured bash /path/to/octoscode/scripts/olp-init.sh
```

Init installs `olp-board-append.py`, `olp-board-event.py`,
`olp-board-inbox.py` and `olp-board-sentinel.py` side by side in
`~/.octos/outer/`. Re-running init never overwrites an existing loop, board,
lock, agent card or installed tool; structured mode only prints migration hints
for existing projects. Without Python, init says the structured tools are
unavailable and the legacy `board-append.sh` / `watch-board.sh` stay usable.
In structured mode the lock is added to `.gitignore` next to the board.

Do not convert an existing project by re-running init. Drain in-flight items,
freeze-copy the whole board and keep receipts, then update the loop by hand; the
new chain needs a new board and a new lock.

A fresh structured board reports `mode=legacy` until its first event. Until
then, the generated `Structured maintenance loop` and the board's own
`<!-- olp-board/v1 -->` line are the opt-in evidence, and the first dispatch
must use `item`.

## Paths and write boundaries

```bash
BOARD="$PWD/.octos/OUTER_LOOP_REVIEW.md"
TOOLS="$HOME/.octos/outer"
```

- The board and `<board>.lock` must already exist as regular files. A board
  path that is a symlink is refused: the legacy shell locks `<link>.lock`, so
  following the link would split the lock domain. A board with more than one
  hard link is refused for the same reason: each name locks its own
  `<name>.lock`. The legacy `olp-board-append.sh` also refuses a symlinked path
  to an opted-in board, or an opted-in board with more than one hard link, and
  never creates a lock beside the extra name.
- Writers hold the lock exclusively while they append the readable text, the
  `> OLP-EVENT <canonical-json>` line and a UTC `ts=` line, then fsync and read
  the range back. Readers (`state`, inbox, sentinel, `verify`) copy the bytes
  under a shared lock and replay after releasing it.
- The guarantee covers cooperating writers on the same lock. Writers that
  bypass the tools, and I/O failures, can still leave partial records; these
  surface as DRIFT (below) and never make `state` fail.
- When an error reports `may_have_appended=true`, inspect `state` and the bytes
  first. Never blindly retry, truncate or "roll back" the board. Argument errors
  of the two write entry points also print one machine JSON, with
  `may_have_appended=false`.
- If the board ends with a line that has no LF, every ordinary write is refused
  and `state` reports that line as `partial_tail` (offset/length/sha256),
  whatever it contains; an unfenced partial event line is also a
  `malformed_event`. After a human check, `void` it. If terminating the line
  would open a code fence (or it already sits inside an open one), the void
  itself would be hidden and is refused; then close the fence with
  `olp-board-append.py append --terminate-partial-line` and a body that starts
  with LF.
- The legacy `olp-board-append.sh` is unchanged on legacy boards. Once the
  board has an event line or the opt-in marker, it refuses body lines that
  start with `> OLP-EVENT ` and standalone `ts=` lines (judged exactly as
  `olp-board-append.py` does: only an optional CR may follow the timestamp).
  Like `olp-board-append.py`, it also refuses a body that does not end with LF
  and any append to a board that ends with a partial line (void that line
  first), so two appends can never splice one line. Other text is appended
  as before.
- `actor` is a logical name, not an OS identity, and nothing here enforces the
  R7 outer-duty lease.

## Events and queues

Lifecycle events: `item`, `receive`, `ack`, `review`. Reconciliation events:
`withdraw`, `resolve`, `void`. Display numbers are free-form strings; every
reference uses the 32-hex event ID.

| Queue | Meaning | Automatic execution? |
|---|---|---|
| `unreceived` | item posted, recipient has not recorded consumption | only by the loop, in ledger order, after `receive` |
| `received_pending` | consumed, no terminal ACK yet (restart reconciliation) | **no** — `execution_authorized=false` |
| `unreviewed_ack` | terminal ACK waiting for the author's review | review only |
| `escalated` | escalated to a human and not yet `resolve`d | not an approval |

Each item has at most one `receive`, one terminal `ack` and one `review`.
`return` and unblocking a `blocked` item require a new item; `wontdo` can be
accepted or escalated but not returned.

| Reconciliation | Who | Effect |
|---|---|---|
| `withdraw --item` | item author | withdraws an item nobody has received yet (e.g. a wrong `--to`) |
| `resolve --review [--next]` | item author | closes an escalation after the human decision, optionally pointing at an existing follow-up item |
| `void --target-file` | outer, after a human check (a convention: `actor` is not authenticated, so any actor can void) | quarantines one DRIFT entry or `partial_tail` as "not a ledger record" |

## Lifecycle

Body files are UTF-8. Item bodies end with LF; ACK, review, withdraw, resolve
and void explanations are one non-empty line. Keep each stdout receipt
(`offset`, `length`, digests, UTC time, `verified=true`).

```bash
python3 -B "$TOOLS/olp-board-event.py" item --board "$BOARD" --actor outer \
  --number 1 --title "First structured item" --to runtime \
  --body-file item.txt > item-receipt.json            # keep .event as ITEM_ID
python3 -B "$TOOLS/olp-board-event.py" receive --board "$BOARD" \
  --actor runtime --item "$ITEM_ID" > receive-receipt.json
python3 -B "$TOOLS/olp-board-event.py" ack --board "$BOARD" --actor runtime \
  --item "$ITEM_ID" --outcome done --commit "$SHA" --r2 verified \
  --body-file ack.txt > ack-receipt.json               # keep .event as ACK_ID
python3 -B "$TOOLS/olp-board-event.py" review --board "$BOARD" --actor outer \
  --ack "$ACK_ID" --decision accept --body-file review.txt

python3 -B "$TOOLS/olp-board-event.py" withdraw --board "$BOARD" --actor outer \
  --item "$ITEM_ID" --body-file why.txt
python3 -B "$TOOLS/olp-board-event.py" resolve --board "$BOARD" --actor outer \
  --review "$REVIEW_ID" --next "$NEXT_ITEM_ID" --body-file decision.txt

python3 -B "$TOOLS/olp-board-event.py" verify --receipt-file item-receipt.json
```

`--commit` takes a full 40- or 64-hex object name. `--r2` is `verified`,
`partially-verified` or `unverified` and must match what was actually run.
Plain notes go through `olp-board-append.py append` (it refuses event prefixes
and caller-supplied standalone `ts=` lines, fenced examples included; indent or
reword a timestamp line in an example, e.g. `ts: …`) and can be re-checked with
`olp-board-append.py verify --offset … --ts …`.

## Queries and watching

A single inbox query that finds nothing still exits 0 with `matched=false`;
errors exit 2; `--wait` timeouts exit 3.

```bash
python3 -B "$TOOLS/olp-board-inbox.py" --board "$BOARD" --for runtime --actor runtime
python3 -B "$TOOLS/olp-board-inbox.py" --board "$BOARD" --for outer \
  --wait --interval 2 --timeout 1800 --since-head "$LAST_HANDLED_HEAD"
python3 -B "$TOOLS/olp-board-sentinel.py" --board "$BOARD" --token 'ACK(' \
  --for outer --since-head "$LAST_HANDLED_HEAD" --interval 2 --timeout 1800
```

`--since-head <event id>` counts `received_pending`, `unreviewed_ack` and
`escalated` entries only when their triggering event comes after that event;
items a runtime has not received yet always count. Pass the `head` of an inbox
or sentinel output after you have handled or deliberately deferred **every**
message in it — never the head from your own write receipt — so an ACK or
escalation you left open does not wake you again, and a runtime is not woken by
its own in-flight item. An unknown head exits 2. Without the flag, a query or a
starting sentinel catches up on everything open.

Sentinel prefixes are `LEDGER-SIGNAL` (ledger change), `BOARD-SIGNAL` (text
wake-up only), `DRIFT`, `ERROR` and `TIMEOUT`; timeouts print `TIMEOUT: {...}`
and exit 3, so check both prefix and exit code. Text is matched on bytes,
so a line the legacy shell appended with bytes that are not UTF-8 still wakes
it (undecodable bytes are shown replaced); bytes that are not UTF-8 in a
token, actor or `--since-head` argument are shown escaped in sentinel and
inbox output and ready files. The ledger, not the text,
decides completion. Keep the existing `events.jsonl` negative sentry; the
sentinel never opens a turn, dispatches or executes anything.

## DRIFT and reconciliation

Any DRIFT sets `mode=mixed` and `dispatch_blocked=true`. Each entry carries
`kind`, `offset`, `length`, `sha256` and `reason`:

| kind | When | Reconcile with |
|---|---|---|
| `malformed_event` | an unfenced `> OLP-EVENT ` line anywhere fails validation (bad JSON, non-canonical, a source that is not adjacent, does not start at a line boundary, overlaps text already paired or consumed, or was changed, wrong `ts=` boundary, broken chain, illegal transition, bad recovery/void); evidence is the line without its LF | `void` |
| `unpaired_item` | after opt-in, a `### number. title` line with no event (split at the first `. `; writers refuse numbers containing `. `, titles may contain it) | `item --recovery-file` with the same number and title, or `void` |
| `unpaired_ack` | after opt-in, an unpaired ACK line in v1 grammar (leading whitespace, a full-width colon and CRLF allowed) | `ack --recovery-file` with the same outcome, or `void` |
| `suspected_ack` | after opt-in, an unpaired hand-written ACK variant: `> ACK(`, `- ACK(`, `### ACK`, `**ACK(...)**`, inline `ACK(`, the retired `ACK:` (prose that merely starts with the word ACK is not flagged, but any `ACK(` is) | ACK recovery if the line contains `ACK(outcome)` with the same outcome, otherwise `void` |
| `unclosed_fence` | after opt-in, a code fence still open at end of file | append a closing line of the same kind, at least as long as the opener |

A malformed event line never breaks replay: it is excluded from the ledger,
later writers chain from the last valid head, and earlier receipts still
verify. Normative text before opt-in is never flagged, but malformed event
lines are flagged anywhere; voiding one on a board without any valid event
makes that void the first event, which opts the board in. **Put examples inside closed code
fences**: a quoted ACK is treated as a suspected hand-written ACK, because lanes
really do write `> ACK(...)` and `### ACK` variants and those must not be missed.
Normative lines and suspected ACKs are judged without NUL bytes, as bash 4+
reads them (it drops NUL), so a NUL splitting the keyword cannot hide a
hand-written ACK. macOS's stock bash 3.2 cuts the line at the NUL instead;
harvest then may not card that line before its recovery, while `state` still
reports the DRIFT and blocks dispatch.

Evolution harvest (`olp-evo-harvest.sh`) takes the structured path on a board
with event lines. It reads the board once, under the shared lock, through
`olp-board-event.py snapshot`, and the replay, the legacy scanner and the
structured scan all use that copy: text appended during a run waits, whole,
for the next run, and each ACK keeps one identity across runs. The `.lock`
beside the board and the adjacent event tool are therefore prerequisites:
when a board already has event lines and either is missing, harvest fails
non-zero and says which, instead of silently switching to the legacy scanner
(which would card the same ACK again under another identity). Legacy boards
without event lines are unaffected. The legacy shell appender does not check
UTF-8, so a legacy trigger line may carry other bytes; the structured path
still cards it. When bash reads a line with a NUL byte, bash 4+ drops the
byte and bash 3.2 cuts the line there; both keep the line count, so the
structured path places each legacy-scanner row by its line number on the
snapshot, and a recovering ACK adopts the identity the legacy scanner gave the
recovered line in the same run, so a NUL never cards one ACK twice whichever
bash runs harvest. A trigger quoted inside any formal
event's source is that event's own text, as replay treats it, and is not
carded. Candidate rows are `|`-separated and identities embed the board path,
so a board whose given or resolved path contains `|` or a newline makes the
structured harvest fail with one `error:` line. Any failed
harvest run exits with an `error:` line and removes
its temporary directory.

To reconcile, copy the exact `offset`, `length` and `sha256` of the entry (or
of `partial_tail`) from `state` into a file:

```json
{"offset":1234,"length":42,"sha256":"<64-lowercase-hex>"}
```

- **recovery** — the text really is an item or ACK record; a formal event of the
  same type adopts it (`--recovery-file`).
- **void** — the text is not a ledger record (a pasted event line, a partial
  write, an ACK variant without an outcome). The original bytes stay; the
  evidence goes to `quarantine_evidence`.

```bash
python3 -B "$TOOLS/olp-board-event.py" void --board "$BOARD" --actor outer \
  --target-file drift.json --body-file why.txt
```

Each span can be consumed once. If the board ends with a partial line, void
`partial_tail` **first** (the tool terminates it with an LF before appending;
see above for a line that would open a code fence); an
interrupted item write usually leaves two entries — the partial event line and
the heading before it — so void both and post the item again. Until an entry is
reconciled, do not re-execute the affected item, and never "ignore" drift
without recording a `void`.

## History, archiving and limits

The board is append-only and completion comes from replay; Active/history bytes
are never moved. Archive by freeze-copying the whole board and checking its
digest, and start a new board, lock and chain only when all four queues are
empty and no DRIFT is open (`withdraw`, `resolve` and `void` make that
reachable). Keep the old board, receipts and recovery/quarantine evidence.

- Only cooperating writers on the canonical path and lock are covered;
  symlinked and hard-linked boards are refused (the link count is checked when
  the path is resolved and again once the lock is held), but a writer that
  bypasses the tools and its lock is not constrained. Locks derive from paths,
  so links made and removed while a write runs are outside what they guard;
  nobody is assumed to manipulate board paths adversarially.
- No authentication and no R7 lease enforcement.
- `received_pending` is reconciliation evidence, not a re-execution queue.
- Every write replays the board twice under the exclusive lock (linear time).
- The extension stays experimental; the protocol version and the R1/R5 choices
  are for the maintainers to decide.

## Verification

The Rust integration tests drive the production CLIs, real temporary files,
the installed tools and the legacy shell lock:

```bash
cargo test --test olp_board_protocol
```

Before committing, still run the full `scripts/verify.sh` (fmt, clippy and
`cargo test --all-targets`).
