# OctoLoop — Agent Onboarding Card

> protocol: olp/v2 — any agent reading this file operates under OLP v2 (R6 version negotiation).
>
> **OctoLoop** is the product name; **OLP** (Outer-Loop Protocol) is the protocol name.
> 中文版:[OCTOLOOP_AGENTS.zh-CN.md](https://github.com/octos-org/octoscode/blob/main/docs/OCTOLOOP_AGENTS.zh-CN.md)

**This card is self-contained.** Read it and you can take a role without opening
another document. Deeper references are linked at the end, and every path and
identifier below is *discovered*, never hard-coded — pane numbers, instance
hashes and session keys drift.

---

## 0. The system in 30 seconds

```
┌─ OUTER loop (strong model: Claude Code / Codex / any CLI agent)
│    read board → dispatch numbered entry → independently re-verify → sign verdict → push
│         ▲                                          │
│    .octos/OUTER_LOOP_REVIEW.md  (the blackboard)    │  herdr prompt / octos steer
│         │                                          ▼
└─ INNER loop (cheap model: octoscode TUI + octos serve, e.g. kimi / glm)
     read board → execute → commit (never push) → ACK(done|wontdo|blocked)
```

The cheap model does the work and can be re-run freely; the expensive model is
spent only on review and adjudication. **Push authority belongs to the outer
loop alone**, so every commit passes two pairs of eyes.

## 1. Pick exactly one role

| You were started as… | Your role | Read |
|---|---|---|
| a worker in a pane, told to work a repo | **inner** | §2 only |
| a reviewer/supervisor, told to oversee another agent | **outer** | §3 (and §2 for the contract you are enforcing) |
| a human at the keyboard | **operator** | §4 |

Never hold two roles at once. If you are unsure, you are **inner** — the outer
role requires an explicit authority claim (§3.0).

## 2. INNER loop contract

The inner contract is **agent-agnostic**: any agent that can read files, run
commands and be driven by a pane can serve as inner loop. It has exactly four
rules:

1. **Read** the `Active` section of `<repo>/.octos/OUTER_LOOP_REVIEW.md`.
2. **Execute** the lowest-numbered entry that has no `ACK(` line yet. Entries
   already ACKed, and anything under `Historical record`, are audit-only —
   **never replay them**.
3. **Commit, never push.** Push authority is the outer loop's.
4. **ACK** under that entry, as a single line in the v1 formula:

```
ACK(done|wontdo|blocked): <explanation>
```

- `done` — executed. Explanation carries evidence: commit hash, test results,
  and the verbatim verification commands you ran (crate / module / feature all
  spelled out).
- `wontdo` — evidenced dissent. Explanation says why not. **The outer loop may
  only accept it or escalate to the operator; it may not bounce the same entry
  back at you again.**
- `blocked` — cannot proceed. Explanation says what blocks you and what would
  release it.

The grammar is pinned by the contract test `olp_ack_lines_match_v1_grammar`.
A new bare `ACK:` line fails that test.

### 2.1 Honest verification declaration (R2)

Every delivery declares exactly one level:

| Level | Means |
|---|---|
| `verified` | ran the full gate: `cargo test --all-targets` + clippy + fmt (or this project's equivalent) |
| `partially-verified` | list precisely what you ran |
| `unverified` | say why — e.g. no toolchain visible in the sandbox |

Claiming `verified` when re-verification disagrees is a **protocol violation**;
it gets bounced and recorded on the board. If a tool is missing, say
`unverified` — do not dress up an assumption as a result.

### 2.2 Shared-workspace discipline (R4 / R4b)

- Run `git status` before touching anything: the tree may hold uncommitted work
  by the outer loop or another agent.
- **`git add` only the files you changed. Never `git add -A`.**
- Dirty files of unknown origin must be **left alone and reported**. Never
  auto-clean or commit them. Only remove temporary artifacts you created this
  round at paths you know.
- **Tree sovereignty**: the main worktree belongs to one goal's branch. Parallel
  work opens its own worktree; a cross-branch `git checkout`/`git switch` in the
  main tree by a session that does not own it is refused by design.
- Do not leave verification scratch files at the repo root. Conclusions go in
  the commit message or the ACK.

### 2.3 When you are stuck

If you burn more than ~30 minutes retrying the same approach, that is a
solution-space stall, not a effort problem — **ask for a blueprint** instead of
grinding. If the MCP fifth channel is mounted, call
`ask_outer(question, context, tried)` (`tried` is mandatory, 3 calls per slice,
90s timeout then a documented degrade). Otherwise write
`ACK(blocked): <what you tried, what you need>` and stop.

## 3. OUTER loop duty

### 3.0 Claim authority first

```bash
octoscode outer-duty hold --project <project> --signature <sig> \
  --duties <duties> -- <your agent start command>
```

The lock **is** the authority: the wrapper is the only holder of the lock fd, and
your agent is death-coupled to it via `PR_SET_PDEATHSIG` — wrapper dies ⇒ agent
dies ⇒ lock `VACANT`. `outer-duty check --project <p>` only observes (exactly one
of `VACANT|HELD|ERROR` on stdout). A non-holder may add **signed annotations
only**. Taking over a live lock belongs to the operator alone; there is no agent
self-seizure. Metadata sidecars and TTLs are diagnostics and never adjudicate.

**Linux-only** (flock + PDEATHSIG + /proc; exits 2 as unsupported elsewhere; not
valid on NFS). On macOS the lock is unavailable — multi-outer coordination falls
back to the duty roster as discipline plus operator arbitration. Pick a signature
like `外环(claude)` / `外环(codex)` and sign every board write.

### 3.1 Discover the scene

```bash
herdr agent list                  # inner panes:  octoscode | <pane> | idle
ls -t ~/.octos/instances/         # instances by recency (hash = DefaultHasher of project cwd)
```

Then read the tail of each project's `.octos/OUTER_LOOP_REVIEW.md` for entries in
flight. **Discovery is not takeover** — your review domain defaults to the
project of your start cwd.

> The live board is `<repo>/.octos/OUTER_LOOP_REVIEW.md`. A same-named file under
> `docs/` is a frozen snapshot — **never write to it**; it is tracked and a
> checkout will wipe your write.

### 3.2 Dispatch — append, never rewrite

Take the current maximum number, then use +1:

```bash
grep -oE '^### [0-9]+' <board> | tail -1
```

Write through the atomic helper (flock-mutexed; body from stdin). It is at
`scripts/olp-board-append.sh` inside the octoscode repo, and `olp-init.sh`
installs a copy at `~/.octos/outer/board-append.sh` for every other project:

```bash
~/.octos/outer/board-append.sh <board> <<'EOF'
### <n>. <title> (<date>, <signature>)
...
EOF
```

If neither is present, an equivalent atomic append is
`flock "<board>.lock" -c 'cat >> "<board>"' <<'EOF' … EOF` — note that
`flock(1)` is util-linux and absent on stock macOS, where you must serialise
board writes by convention instead.

An entry must be **self-contained**: background, exact files and line numbers,
direction of the fix, acceptance criteria, branch name (based on main), budget
tier, and the standing instruction "commit only, do not push — the reviewer
re-verifies and pushes."

Budget tiers: **revision** 5–10M · **slice** 10–20M · **campaign** 30–50M.

Reversing an earlier entry requires a **new un-ACKed entry** (R1/R5) —
annotations under an already-closed entry do not wake the inner loop. Two
line-initial formulas are machine-harvested; **copy them verbatim, do not
translate**:

```
> 外环(<sig>)·改判(作废 #N):<the instruction that now governs>
> 外环(<sig>)·R2 记档(#N):<claim vs. re-verified fact>
```

### 3.3 Wake and steer the inner loop

```bash
# idle pane → wake it, pointing at the new entry number (lands as a user-level message)
herdr agent prompt <pane> '<one line>'

# mid-turn → steer instead; consumed on the next beat without interrupting the action
cd <project>          # steer resolves the instance by cwd; must run in the project dir
octos steer --session '<session-key>' --text '[external-reviewer] ...'
#   session key: ls <repo>/.octos/octos/sessions/ , then URL-decode the filename
```

Without herdr, fall back to tmux `send-keys` — text starting with `-` needs a
`--` separator or it is eaten as a flag.

### 3.4 Observe — three layers, all of them

**Delivery ≠ consumption ≠ execution.** Watching one layer guarantees a
misread: a goal that tripped its breaker is silent in exactly the same way as a
goal that is still working.

```bash
herdr pane read <pane>                                     # 1. the live screen
tail -f ~/.octos/instances/<hash>/profiles/<profile>/data/events.jsonl   # 2. events
octos goal status --json ; octos ledger tail --json ; octos peer list    # 3. structure
```

Mount **two sentries**, not one: a positive-signal sentry (ACK lands on the
board) *and* a negative-signal sentry (`goal_transition blocked` / `escalation`
in `events.jsonl`).

### 3.5 Re-verify, then push

An inner-loop self-verification claim **is not trustworthy on its own** — there
are logged cases of "clippy clean" reported twice falsely and of "tests green"
faked by a wrapper script. Re-run it yourself, in isolation:

```bash
git worktree add ~/.octos/outer/verify/<name> <commit>
# take verification commands VERBATIM from .github/workflows — a workspace-root
# --features does not propagate to member crates, so target tests need e.g.
#   cargo test -p <crate> --features <feat>
git worktree remove --force ~/.octos/outer/verify/<name>    # clean up right after
```

Watch for three classes of counterfeit: an exit code standing in for behavior, a
wrapper script wrapping the real command, and a test double standing in for the
production path.

Pass → sign an acceptance verdict on the board → `git push fork <branch>`.
Fail → **a new entry** with the evidence. Never rewrite the old one.

### 3.6 Red lines

1. Never push what you have not independently re-verified.
2. Never press an operator's approval — granting sandbox-free permission is a
   human action. Ask, with the exact command ready.
3. Respect the queue: the inner loop eats entries in board order. To jump the
   queue, state the case in an entry and let the operator decide.
4. Shared machine: ≤2 concurrent compiles, `--test-threads=8`, and confirm
   `TMPDIR` points at the home disk.
5. Anything pending >12h must be reported to the operator with queue-jump options.

## 4. Operator-only actions

These are never delegated to an agent: pasting API keys, granting sandbox-free
permission (`--danger-full-access` / permission tier 5), taking over a live
`outer-duty` lock, and publishing issues.

Standard inner-loop start command:

```bash
octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'
```

- `--solo` is the safety gate for a single-person local box. Without it serve
  refuses: *"requested permission profile is not allowed outside local solo mode"*.
- **Permission tiers 1–4 are a filesystem sandbox, not an approval switch.** All
  four run under bwrap with only the workspace and system dirs mounted, so
  `~/.cargo` and `~/.rustup` are **invisible** and every build command is
  "command not found". Running a toolchain needs tier 5 (Full Access) or
  `--danger-full-access`. This is the true cause behind inner loops reporting
  "there is no cargo on this machine".

After a (re)start, the outer loop walks a four-item checklist: serve is up
(operator); `/loop resume <id>` (get the id from `/loop list` — bare `resume` is
refused); both sentries mounted; `llm.fallbacks` configured **and snapshotted by
a new session** (the tool/config snapshot is taken at session creation, so
editing config without a new session is paper insurance).

## 5. Failure quick-reference

| Symptom | Cause and fix |
|---|---|
| inner loop says "no cargo on this machine" | bwrap sandbox, tiers 1–4 — use tier 5 or `--danger-full-access` |
| `permission profile is not allowed outside local solo mode` | serve is missing `--solo` |
| `octos: 'serve' is not a subcommand` | source build missed a feature: `cargo build --release --features api` |
| herdr injection silently lost | double gate: named-agent list **and** pane foreground process-name match — miss either and it is dropped. Fall back to tmux `send-keys` |
| board never read by the inner loop | board got tracked, or cross-branch split brain — confirm `.octos/OUTER_LOOP_REVIEW.md` is gitignored; re-run `olp-init.sh` (idempotent) |
| MCP tools (`ask_outer`) missing | profile JSON malformed (timestamps must be RFC3339 **with Z**), or the session predates the config — fix, then start a **new** session |
| stalls / errors across the board | `llm.fallbacks` unset and the primary lane refused on quota/auth |
| linker SIGBUS / EDQUOT on Linux builds | `/tmp` is tmpfs with a quota: `export TMPDIR=$HOME/.local/tmp` |

## 6. Platform support

| Platform | Status |
|---|---|
| Linux | full power, including the `outer-duty` kernel lock and bwrap tiers |
| WSL2 | equivalent to Linux |
| macOS | usable, two gaps: no `outer-duty` lock (Linux-only), and bwrap tiers do not apply |
| Windows native | not recommended — use WSL2 |

## 7. Going deeper

- [`OUTER_LOOP_PROTOCOL.md`](https://github.com/octos-org/octoscode/blob/main/docs/OUTER_LOOP_PROTOCOL.md) — the protocol in full: R1–R7,
  `result.md` schema, multi-outer rules, budget governance, field lessons
- [`OLP_OUTER_BOOT.md`](https://github.com/octos-org/octoscode/blob/main/docs/OLP_OUTER_BOOT.md) — the outer operator card and tactics handbook
- [`OLP_QUICKSTART.md`](https://github.com/octos-org/octoscode/blob/main/docs/OLP_QUICKSTART.md) — zero-to-running for a new project
- [`OCTOLOOP_GUIDE.md`](https://github.com/octos-org/octoscode/blob/main/docs/OCTOLOOP_GUIDE.md) — full guide, mechanisms, platform matrix
- [`OCTOLOOP_FEATURES.md`](https://github.com/octos-org/octoscode/blob/main/docs/OCTOLOOP_FEATURES.md) — one-page capability panorama
