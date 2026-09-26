# OLP Quickstart — from zero to a running dual loop

For **third-party users**: you have a code repository and you want the dual
loop — *cheap model does the work on the inside, strong model reviews from the
outside* (OLP, [Outer-Loop Protocol](OUTER_LOOP_PROTOCOL.md)) — running on your
own machine. This page is the shortest path only. Protocol detail, discipline
and field lessons live in the protocol doc.

> 中文版:[OLP_QUICKSTART.md](OLP_QUICKSTART.md)

## 0. What this is (30 seconds)

```
┌─ OUTER loop (strong model: Claude Code / Codex / any CLI agent)
│    read board → dispatch a fix order → independently re-verify → sign verdict → push
│         ▲                                     │
│    .octos/OUTER_LOOP_REVIEW.md (the board)     │ steer / prompt
│         │                                     ▼
└─ INNER loop (octoscode TUI + octos serve, running a cheap model such as kimi)
     read board → execute → commit (no push) → ACK(done|wontdo|blocked)
```

The inner model is cheap and can be re-run freely; the outer model is expensive
and is spent only on review and adjudication. **Push authority belongs to the
outer loop alone**, so every commit passes two pairs of eyes.

**On the names**: **OLP** is the protocol name (used by the docs and the ACK
formula); **OctoLoop** is the product name — the one-command wrapper from the
user's point of view (`.claude/skills/octoloop`, three modes: init / outer /
inner; capability panorama in [`OCTOLOOP_FEATURES.md`](OCTOLOOP_FEATURES.md)).

An agent taking a role for the first time should read the self-contained
onboarding card instead of this page:
[`OCTOLOOP_AGENTS.md`](OCTOLOOP_AGENTS.md).

## 1. Dependencies

| Dependency | Required? | Notes |
|---|---|---|
| Linux / macOS | yes | Windows can run the TUI; serve's sandbox tiers (bwrap) are a Linux feature |
| Network | first run only | First start downloads the octos server to `~/.octos/bin`; offline, see the README failure table |
| Node **or** Homebrew **or** shell installer | pick one | Only to install binaries. **Rust is not needed** — only a source build needs Rust 1.85+ |
| Inner-loop model API key | yes | The cheap tier, e.g. Moonshot (kimi). Paste it in the onboarding wizard |
| Outer-loop agent CLI | yes | Claude Code or Codex, on the subscription you already have |
| herdr (terminal workspace manager) | no, recommended | Lets the outer loop drive the inner pane programmatically via `herdr agent prompt`; without it, fall back to tmux `send-keys`. **Source**: <https://github.com/hagency-org/herdr>. octoscode pane detection (`--kind octoscode`) currently lives on the `feat/octoscode-agent` branch — build from there; follow master once it merges |
| bwrap (bubblewrap) | usually preinstalled on Linux | The filesystem sandbox behind permission tiers 1–4; semantics in §0b |

## 2. The one-command path

```bash
# ① install the TUI (the server is pulled on first start; no resident background service)
npm install -g @octos-org/octoscode

# ② lay the OLP scaffolding in your project (idempotent; never overwrites an existing file)
cd your-project/
curl -fsSL https://raw.githubusercontent.com/octos-org/octoscode/main/scripts/olp-init.sh | bash
#   (or clone this repo and run scripts/olp-init.sh)

# ③ start the inner loop
octoscode --stdio-command 'octos serve --stdio --solo'
```

`olp-init.sh` does six things: a dependency check (what is missing and how to
install it, in one screen); generates `.octos/loop.md` (the inner maintenance
loop) and `.octos/OUTER_LOOP_REVIEW.md` (the board template); adds the board to
`.gitignore` (it is branch-independent — tracking it causes cross-branch split
brain); writes `AGENTS.md`, the self-contained onboarding card, so any agent
that lands in the project can take a role (set `OLP_INIT_LANG=zh` for the
Chinese card); installs the board sentry and the atomic board-append helper
under `~/.octos/outer/`; and prints the start command with a next-step list.

Run it from a repo checkout to get every artifact. Run it standalone through
`curl | bash` and the helper scripts are skipped — the script says so on the
line, rather than pretending.

**The honest limits of "one command"** — two things the script deliberately does
not do for you, because they are the operator's explicit decisions:

1. **API keys**: paste them yourself in the onboarding wizard on first entry to
   the TUI (three fields, five minutes).
2. **Granting sandbox-free permission**: see the next section.

### 0b. Permission-tier semantics (read this first; it saves two days)

Permission tiers **1–4 (Default / Read Only / Workspace Write / Never Ask) all
run inside the bwrap filesystem sandbox** — only the workspace and system
directories are mounted, so `~/.cargo` and `~/.rustup` are **invisible** to the
agent and every build command is "command not found". To let the inner loop run
a toolchain such as cargo or npm, you must either:

- pick tier **5, Full Access** (no sandbox) in the permission menu, or
- start serve with `--danger-full-access` (equivalent to
  `OCTOS_DANGER_FULL_ACCESS=1`). The standard command:

```bash
octoscode --stdio-command 'octos serve --stdio --solo --danger-full-access'
```

`--solo` is the safety gate for a single-person local box: a permissive
permission profile is allowed only under an explicit solo opt-in. The symptom of
forgetting it is *"requested permission profile is not allowed outside local solo
mode"*.

Going sandbox-free is a security decision. The operator makes it by hand — do
not let any agent press it for you.

## 3. Inner-loop lane configuration (optional; the key to saving money)

octos's `sub_providers` splits models into lanes so mechanical work runs on the
cheap tier (templates and the selection matrix are in appendix B of the protocol
doc):

```toml
[sub_providers.cheap]
model = "kimi/kimi-k2-turbo"
description = "Mechanical, low-risk, reversible: test diagnosis, log triage, ACK lookup, format shuffling."

[sub_providers.strong]
model = "anthropic/claude-opus"
description = "Long-chain reasoning and cross-file judgement: review grading, dispute adjudication, multi-step debugging."
```

**The main conversation lane** is separate — it lives in `config.llm` of the
profile at `~/.octos/profiles/<id>.json`, while `sub_providers` only feeds
pipeline nodes:

```json
{
  "llm": {
    "primary": { "provider": "zai-coding", "model": "glm-5.3" },
    "fallbacks": [
      { "provider": "moonshot-coding", "model": "k3" },
      { "provider": "deepseek",        "model": "deepseek-chat" }
    ]
  }
}
```

- `primary` — the resident main lane.
- `fallbacks[]` — the automatic degrade sequence when a provider cuts you off
  (quota or auth refusal switches lane by lane). Order is priority.
- Changes take effect in a **new session** — tools and config are snapshotted
  when the session is created. Timestamp fields in the profile JSON must be
  RFC3339 **with `Z`**, or the whole profile fails to parse.
- **What it feels like**: when a provider cuts off, the conversation does not
  stop — the status bar flashes a degrade notice once and responses continue;
  the main lane returns on its own afterwards, with no restart.

## 4. Minimal outer-loop onboarding (any CLI agent, three steps)

Hand your outer agent (Claude Code or Codex, either works) this paragraph and it
is onboarded:

> You are this project's outer-loop reviewer. ① Read `AGENTS.md` (the
> self-contained card) and `.octos/OUTER_LOOP_REVIEW.md`. ② To dispatch work,
> **append** a dated, numbered entry to the board — append only, never rewrite.
> ③ When the inner loop finishes it writes `ACK(done|wontdo|blocked): <notes>`
> under the entry: `done` must be **independently re-verified** (run the tests in
> an isolated git worktree, never touching the inner loop's tree), and `wontdo`
> you may only accept or escalate to the operator — never bounce the same entry
> back. ④ Once accepted, **you** push; the inner loop never pushes. ⑤ With
> multiple outer agents, sign your annotations and escalate disputes to the
> operator.

Drive and observation channels, all scriptable, so the outer loop can rotate
them on its own:

```bash
# downward: drop one outer-loop opinion into the running session in real time
#           (arrives at an independent user-message level)
octos steer --session '<session-key>' --text '[external-reviewer] ...'

# upward: structured observation
octos goal status --json      # the goal state machine
octos ledger tail --json      # the delivery ledger
tail -f .../data/events.jsonl # steer_consumed / escalation / goal_transition …
```

herdr users get a second injection channel: `herdr agent list` to see the panes,
`herdr agent prompt <pane> '<text>'` to deliver at user-message level.

## 5. Smoke test (two minutes)

1. Say hello in the TUI and confirm the main-tier model answers.
2. Have the inner loop ACK the first board entry (the "board enabled" entry that
   `olp-init.sh` generates) — that closes the read/write loop.
3. With herdr: `herdr agent list` should show `octoscode | <pane> | idle`.

## 6. Failure quick-reference

| Symptom | Cause and fix |
|---|---|
| `octos: 'serve' is not a subcommand` | A source build missed a feature: `cargo build --release --features api` (release binaries are unaffected) |
| inner loop says "there is no cargo on this machine" | The bwrap sandbox of tiers 1–4 — see §0b: tier 5 or `--danger-full-access` |
| `permission profile is not allowed outside local solo mode` | serve is missing `--solo` |
| first-run server download fails | Offline or behind a proxy: install by hand with `npm i -g @octos-org/octos`; `OCTOSCODE_NO_AUTO_INSTALL=1` disables auto-install |
| linker SIGBUS / EDQUOT when building a large project on Linux | `/tmp` is tmpfs and may carry a quota, and rust-lld's temporary files are large: `export TMPDIR=$HOME/.local/tmp` (create it, then put this in your shell profile) |
| herdr injection silently lost | A double gate: the named-agent list **and** a pane foreground process-name match. Miss either and it is dropped. Fall back to tmux `send-keys` (text starting with `-` needs a `--` separator) |

## 7. Going deeper

- [`OCTOLOOP_AGENTS.md`](OCTOLOOP_AGENTS.md) — the self-contained onboarding
  card: role selection, ACK grammar, dispatch, wake, observation,
  re-verification, red lines. This is what you hand an agent.
- [`OUTER_LOOP_PROTOCOL.md`](OUTER_LOOP_PROTOCOL.md) — the protocol in full: ACK
  grammar, `result.md` schema, multi-outer rules, budget governance, the
  complete set of field lessons
- [`OLP_OUTER_BOOT.md`](OLP_OUTER_BOOT.md) — the outer operator card and tactics
  handbook
- [`OCTOLOOP_GUIDE.md`](OCTOLOOP_GUIDE.md) — full guide, mechanisms, platform
  matrix
- README, "Quickstart (solo onboarding)" — the single-loop path, screen by
  screen, with no outer loop
