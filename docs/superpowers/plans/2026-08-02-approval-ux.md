# Approval UX (Silent-Wait → Sudden-Approval) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kill the observed failure mode (2026-08-02 16:55–17:00, turn 019fc1ae): a spawned sub-agent ran 30+ bash iterations for 5 minutes while the main view showed only a frozen `Orchestrating… Spawn` line, then an approval materialized silently as unbordered muted text. After this plan: sub-agent rows show live progress, decision arrival rings the terminal bell, and the approval card is a bordered, risk-colored surface with requester context.

**Architecture:** Three TUI-side changes plus one regression pin. Store stays pure (no I/O): arrival side-effects use the existing `flush_pending_clipboard` pattern — a state flag drained by the event loop. Rendering changes live in `transcript_build.rs` (approval card) and `app.rs` (sub-agent chip rows). Root-cause evidence: `push_inline_approval_card` (`src/app/transcript_build.rs:2189`) emits loose indented muted lines; repo-wide grep shows NO terminal bell anywhere (only OSC 52 clipboard writes in `clipboard.rs`); `running_subagent_titles_for_chip` (`src/app.rs:2976`) surfaces only static titles.

**Tech Stack:** Rust, ratatui 0.29, rust_i18n (en+zh), crossterm raw writes for BEL.

## Global Constraints

- Test command: `cargo test --lib` (must end green).
- Store methods perform NO terminal I/O — side-effects go through state flags drained by `event_loop.rs` (`flush_pending_clipboard` is the template).
- Every user-visible string gets `locales/en.yml` AND `locales/zh.yml` keys.
- reserve==render: the approval card renders in the live tail, whose height flows through `live_tail_height_with_finalization` — card height changes must keep reservation and render identical (the card builder is shared, so this holds automatically; do not add render-only lines outside the shared builder).
- Commit style: `feat(tui): …` / `fix(tui): …`, one commit per task.

---

### Task 1: Terminal bell on decision arrival

**Files:**
- Modify: `src/model.rs` (AppState: new field)
- Modify: `src/store.rs` (approval / user-question arrival chokepoints)
- Modify: `src/event_loop.rs` (drain the flag next to `flush_pending_clipboard`)
- Test: `src/store.rs` `mod tests`

**Interfaces:**
- Produces: `AppState.pending_decision_bell: bool` (set on arrival, cleared by the event loop after writing `\x07` to stdout).
- Consumes: the arrival sites — search `src/store.rs` for where `self.state.approval = Some(` and `self.state.user_question = Some(` are assigned with `visible: true` (the event-application path, NOT test fixtures).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn decision_arrival_arms_the_terminal_bell() {
    // 2026-08-02 trap: an approval materialized after 5 silent minutes with
    // zero salience — no bell, no flash. Arrival must arm exactly one bell.
    let (mut store, _approval_id) = store_with_visible_approval();
    assert!(
        store.state.pending_decision_bell,
        "approval arrival must arm the bell"
    );
    // Draining is the event loop's job; simulate it.
    store.state.pending_decision_bell = false;
    // A redraw or unrelated event must NOT re-arm.
    store.state.status = "tick".into();
    assert!(!store.state.pending_decision_bell);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib decision_arrival_arms -- --nocapture`
Expected: FAIL — field does not exist (compile red).

- [ ] **Step 3: Implement**

`model.rs` — next to `ctrl_c_quit_armed`:

```rust
/// Armed when an approval or AskUserQuestion ARRIVES (becomes visible);
/// the event loop drains it by writing BEL to the terminal exactly once.
/// Store stays I/O-free — same pattern as the pending-clipboard flush.
pub pending_decision_bell: bool,
```

(add `pending_decision_bell: false,` to the constructor).

`store.rs` — at each arrival chokepoint that assigns a VISIBLE approval or
question from a server event, add:

```rust
self.state.pending_decision_bell = true;
```

`event_loop.rs` — next to the `flush_pending_clipboard(&mut store)` call in
the main loop:

```rust
if store.state.pending_decision_bell {
    store.state.pending_decision_bell = false;
    // BEL: audible/visual notify per terminal settings; harmless when the
    // terminal has bells disabled. Raw write — no frame implications.
    use std::io::Write as _;
    let _ = write!(io::stdout(), "\x07");
    let _ = io::stdout().flush();
}
```

`store_with_visible_approval` (test fixture) constructs the state directly —
if the test from Step 1 fails because the fixture bypasses the chokepoint,
route the fixture through the real event-application path or set the flag in
the same place the fixture sets `visible: true`, mirroring production.

- [ ] **Step 4: Run the full suite** — `cargo test --lib` → PASS.

- [ ] **Step 5: Commit**

```bash
git add src/model.rs src/store.rs src/event_loop.rs
git commit -m "feat(tui): terminal bell when an approval or question arrives"
```

---

### Task 2: Bordered, risk-colored approval card with requester context

**Files:**
- Modify: `src/app/transcript_build.rs:2189` (`push_inline_approval_card`)
- Modify: `locales/en.yml`, `locales/zh.yml` (`app.approval` section)
- Test: `src/app/tests.rs`

**Interfaces:**
- Produces: same signature `push_inline_approval_card(lines, palette, approval)` — output shape changes to a bordered card. New locale keys `app.approval.risk_chip` (`"risk: %{risk}"` / `"风险: %{risk}"`) and `app.approval.from_task` (`"from: %{title}"` / `"来自: %{title}"`).
- Consumes: `ApprovalModalState` fields already in use (`tool_name`, `title`, `risk`, `approval_kind`), `approval_modal_lines`, `approval_action_labels`, `push_command_row`; border glyphs follow the existing table-border style in the same file (`┌ ─ ┐ │ └ ┘`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn approval_card_renders_bordered_with_risk_chip() {
    // The old card was loose muted text that blended into the stream — the
    // user's verdict after hitting it live: "授权 ui 做的很不好".
    let (app_store, _id) = store_with_visible_approval();
    let text = rendered_text(&app_store.state);

    assert!(text.contains("┌"), "card has a top border: {text}");
    assert!(text.contains("└"), "card has a bottom border: {text}");
    assert!(
        text.contains("risk: high") || text.contains("风险: high"),
        "risk renders as an explicit chip: {text}"
    );
    assert!(
        text.contains("Approval Requested"),
        "title survives the restyle: {text}"
    );
}
```

(If `store_with_visible_approval` lives in `store.rs` tests only, build the
same `ApprovalModalState { risk: Some("high".into()), … }` fixture inline —
copy the exact fixture from
`pending_decision_card_stays_visible_when_the_live_tail_overflows`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib approval_card_renders_bordered -- --nocapture`
Expected: FAIL — no border glyphs in current output.

- [ ] **Step 3: Implement**

Rewrite `push_inline_approval_card`:

```rust
pub(super) fn push_inline_approval_card(
    lines: &mut Vec<Line<'static>>,
    palette: Palette,
    approval: &ApprovalModalState,
) {
    let danger = Style::default().fg(palette.danger).add_modifier(Modifier::BOLD);
    lines.push(Line::from(""));
    // ┌─ ⚠ Approval Requested ── bash · risk: high ─┐  (danger-tinted border)
    let mut header = vec![
        Span::styled("  ┌─ ", danger),
        Span::styled("⚠ ", danger),
        Span::styled(
            t!("app.approval.title").to_string(),
            palette.title().add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(tool) = approval.tool_name_display() {
        header.push(Span::styled(format!(" ── {tool}"), palette.muted()));
    }
    if let Some(risk) = approval.risk.as_deref() {
        header.push(Span::styled(
            format!(" · {}", t!("app.approval.risk_chip", risk = risk)),
            danger,
        ));
    }
    header.push(Span::styled(" ─┐", danger));
    lines.push(Line::from(header));

    for line in approval_modal_lines(approval, palette) {
        push_prefixed_line(lines, "  │ ", palette.muted(), line);
    }
    for action in approval_action_labels(approval) {
        lines.push(Line::from(vec![
            Span::styled("  │ ", danger),
            Span::styled(action, palette.selected()),
        ]));
    }
    if approval.diff_preview_id().is_some() {
        lines.push(Line::from(vec![
            Span::styled("  │ ", danger),
            Span::styled(t!("app.approval.action_diff").to_string(), palette.selected()),
        ]));
    }
    lines.push(Line::from(Span::styled("  └──", danger)));
}
```

Notes for the implementer: `tool_name_display()` may not exist — use
`approval.tool_name` directly (it is a `String` in the fixture; adapt to the
real field type). Do NOT try to draw a full-width right border (the card body
wraps at arbitrary widths); the left rail `│` plus top/bottom caps is the
established compromise elsewhere in this file.

Locale additions:

```yaml
# en.yml app.approval:
    risk_chip: "risk: %{risk}"
    from_task: "from: %{title}"
# zh.yml:
    risk_chip: "风险: %{risk}"
    from_task: "来自: %{title}"
```

(`from_task` is emitted as the first body line when the active turn has
exactly one running sub-agent task — reuse
`running_subagent_titles_for_chip(app, turn_id)`; plumb `app` in via the one
existing call site of `push_inline_approval_card`, which already has it.)

- [ ] **Step 4: Run the full suite** — `cargo test --lib` → PASS. Update the
  `pending_decision_card_stays_visible_when_the_live_tail_overflows`
  assertions if they pinned the old unbordered copy (the card-survival
  property itself must keep passing).

- [ ] **Step 5: Commit**

```bash
git add src/app/transcript_build.rs src/app/tests.rs locales/en.yml locales/zh.yml
git commit -m "feat(tui): bordered risk-colored approval card with requester context"
```

---

### Task 3: Live sub-agent progress on the orchestrating chip

**Files:**
- Modify: `src/model.rs` (AppState: task first-seen clock)
- Modify: `src/app.rs:2976` (`running_subagent_titles_for_chip` → return richer rows)
- Modify: `src/app/transcript_build.rs` (the sub-agent row emission in `push_agent_task_group` — search `subagent_titles`)
- Modify: `locales/en.yml`, `locales/zh.yml`
- Test: `src/app/tests.rs`

**Interfaces:**
- Produces: `running_subagent_rows_for_chip(app, turn_id) -> Vec<String>` replacing `running_subagent_titles_for_chip` — each row is `"{title} · {elapsed}"` (e.g. `写第3章 · 4m12s`), where elapsed comes from `AppState.task_first_seen: HashMap<String, Instant>` recorded when a task first appears in `pending|running` state (mirror of the `PeerMeta.created` pattern). Keep the old fn name as a thin wrapper if other call sites exist.
- Consumes: `session.tasks` (`task.title`, `task.state`, `task.turn_id`, task id field — use whatever id the `TaskView` carries; check `task_state_label` usages), `format_duration_ms` (exists in `transcript_build.rs`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn running_subagent_row_shows_elapsed_time() {
    // 2026-08-02 trap: "Orchestrating… Spawn" sat frozen for 5 minutes with
    // zero feedback. The row must at least show the task is aging.
    let mut app = /* reuse the fixture from
        live_ui_height_reserves_agent_strip_row — a session with one
        running task attributed to the active turn */;
    app.task_first_seen.insert(
        task_id.clone(),
        Instant::now() - Duration::from_secs(272),
    );

    let text = rendered_text(&app);

    assert!(
        text.contains("4m32s") || text.contains("4m 32s"),
        "sub-agent row carries elapsed time: {text}"
    );
}
```

(Adapt the fixture to the real `TaskView` shape — copy from the existing
test at `src/app/tests.rs` search `tasks: vec![TaskView {`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib running_subagent_row_shows -- --nocapture`
Expected: FAIL — `task_first_seen` missing (compile red).

- [ ] **Step 3: Implement**

`model.rs`:

```rust
/// Client-side first-seen clock per task id, for the sub-agent chip's
/// elapsed display. Populated when a task first shows up pending/running
/// (server events carry no wall-clock the TUI can trust across hosts).
pub task_first_seen: std::collections::HashMap<String, std::time::Instant>,
```

Populate at the store's task-event application site (search `session.tasks`
assignment / merge in `store.rs`): `entry(task_id).or_insert_with(Instant::now)`
for tasks in `pending|running`.

`app.rs` — rename/extend:

```rust
fn running_subagent_rows_for_chip(
    app: &AppState,
    turn_id: Option<&octos_core::ui_protocol::TurnId>,
) -> Vec<String> {
    // …same filtering as today (keep the codex P2 scoping comment)…
        .map(|task| {
            let elapsed = app
                .task_first_seen
                .get(&task.id)
                .map(|seen| format_duration_ms(seen.elapsed().as_millis() as u64))
                .map(|d| format!(" · {d}"))
                .unwrap_or_default();
            format!("{}{elapsed}", task.title)
        })
        .collect()
}
```

The chip's row renderer (`push_agent_task_group` sub-agent loop) needs no
change — it already prints the string it is given plus the `running` suffix.
The elapsed refreshes automatically: the live chip repaints on the animation
cadence while a turn is active.

- [ ] **Step 4: Run the full suite** — `cargo test --lib` → PASS.

- [ ] **Step 5: Commit**

```bash
git add src/model.rs src/store.rs src/app.rs src/app/tests.rs
git commit -m "feat(tui): sub-agent chip rows show elapsed time while running"
```

---

### Task 4: Regression pin — approvals flip the status bar to Waiting

**Files:**
- Test: `src/app/tests.rs`

**Interfaces:** none new — pins existing `render_status` behavior (`waiting_on_operator` in `src/app/render.rs`) for the spawn-approval flow.

- [ ] **Step 1: Write the test** (may already pass — that is the point of a pin)

```rust
#[test]
fn spawn_originated_approval_flips_state_to_waiting() {
    // The 16:59 screenshot showed "Working (5m 7s…)" while an approval chip
    // sat clipped at the bar's tail. Approvals arrive on the MASTER session
    // (log: session_id=kimi:local:tui#coding), so Waiting must fire.
    let (store, _id) = store_with_visible_approval();
    let text = rendered_text(&store.state);
    assert!(
        text.contains("Waiting"),
        "a visible approval for the active session must show Waiting: {text}"
    );
}
```

- [ ] **Step 2: Run** — `cargo test --lib spawn_originated_approval -- --nocapture`
Expected: PASS (pin) — if it FAILS, the fixture's `run_state` is not
`InProgress|Blocked`; set `store.state.set_run_state_in_progress()` in the
fixture path and re-check, since that mirrors the live flow.

- [ ] **Step 3: Commit**

```bash
git add src/app/tests.rs
git commit -m "test(tui): pin Waiting state for spawn-originated approvals"
```

---

## Out of scope (explicit)

- OSC 9 / OSC 777 desktop notifications (bell first; escalate only if users ask).
- Streaming the sub-agent's full output into the main transcript (Tab peek and `/ps` remain the drill-down; the ambient row + digest work is in the companion plan `2026-08-02-activity-compact-fold-ui.md` Task 3/4).
- Server-side changes (approval payloads already carry tool/risk/kind — enough for the card).
