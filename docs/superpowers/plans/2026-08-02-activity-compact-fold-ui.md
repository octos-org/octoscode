# Activity Compact/Fold UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Claude-Code-style dense activity display: identical bare tool rows merge into one `⏺ Bash ×5` line, folded history renders as a prominent `◈ N more` row, scrollback batches compress to one digest line, and the harness row carries a one-line live summary — so a 100-action turn reads as a handful of lines instead of a wall.

**Architecture:** All rendering changes live in the transcript builder (`src/app/transcript_build.rs`) and the harness status line (`src/app.rs`), on top of the 2026-08-02 capsule-continuation mechanism (`tail_activity_turn` in `viewport.rs`). Scrollback is IMMUTABLE — compression must happen at write time; the repainting live tail can restyle freely. Full per-action detail stays reachable via `/activity` and Ctrl+O expand.

**Tech Stack:** Rust, ratatui 0.29, rust_i18n (`locales/en.yml` + `zh.yml`), inline `#[cfg(test)]` tests.

## Global Constraints

- Test command: `cargo test --lib` (1656 tests, must end green).
- reserve==render: any change to rendered row counts must keep every height-budget site in sync (`live_ui_height`, `chat_layout_areas`, render layouts) — the codebase's core invariant.
- Every user-visible string gets BOTH `locales/en.yml` and `locales/zh.yml` keys.
- Scrollback is immutable: never plan a change that edits already-flushed lines.
- TDD: failing test before implementation, per task.
- Commit style: `feat(tui): …`, one commit per task.

---

### Task 1: Run-length merge of identical bare tool rows

**Files:**
- Modify: `src/app/transcript_build.rs` (child-emission: the `for (idx, item) in items.iter()…` loop in `push_agent_task_group`, and the `append_children_only` early block added 2026-08-02)
- Modify: `locales/en.yml`, `locales/zh.yml` (`app.activity` section)
- Test: `src/app/tests.rs`

**Interfaces:**
- Produces: `fn push_agent_task_children(lines: &mut Vec<Line<'static>>, palette: Palette, items: &[&ActivityItem], first_offset: bool, expanded: bool, wrap_width: usize, show_output: bool)` — replaces both child loops; merges runs.
- Consumes: existing `tool_display_name(&str) -> String`, `tool_invocation_text(&ActivityItem) -> Option<String>`, `tool_card_bullet`, `push_agent_task_child`, `activity_is_failed`, `is_running_activity`.
- Merge key: two consecutive items merge iff same `tool_display_name`, BOTH have empty/absent `tool_invocation_text`, and same (failed?, running?) status. Rows with a real invocation (`Bash($ cargo test)`) never merge.

- [ ] **Step 1: Write the failing test** (next to the capsule tests in `src/app/tests.rs`; reuse `capsule_app` / `capsule_tool_item`, but build BARE items without `.with_detail(…)`)

```rust
fn bare_tool_item(turn_id: &TurnId, call_id: &str) -> ActivityItem {
    ActivityItem::new(ActivityKind::Tool, "shell", "complete")
        .with_turn(turn_id.clone())
        .with_tool_call(call_id)
        .with_success(true)
}

#[test]
fn consecutive_bare_tool_rows_merge_into_one_run_length_line() {
    // Claude-Code-style density: five argument-less Bash rows are one
    // "⏺ Bash ×5" line, not five identical lines.
    let turn_id = TurnId::new();
    let session_id = SessionKey("local:test".into());
    let mut app = capsule_app(&session_id, &turn_id);
    app.activity.clear();
    for id in ["c1", "c2", "c3", "c4", "c5"] {
        app.activity.push(bare_tool_item(&turn_id, id));
    }

    let text = rendered_text(&app);

    assert!(text.contains("Bash ×5"), "run-length merged row: {text}");
    assert_eq!(
        text.matches("⏺ Bash").count(),
        1,
        "exactly one merged Bash row: {text}"
    );
}

#[test]
fn rows_with_invocations_never_merge() {
    let turn_id = TurnId::new();
    let session_id = SessionKey("local:test".into());
    let mut app = capsule_app(&session_id, &turn_id);
    app.activity.clear();
    app.activity
        .push(capsule_tool_item(&turn_id, "c1", "cargo build"));
    app.activity
        .push(capsule_tool_item(&turn_id, "c2", "cargo test"));

    let text = rendered_text(&app);

    assert!(text.contains("cargo build") && text.contains("cargo test"));
    assert!(!text.contains("×2"), "distinct invocations stay separate: {text}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib consecutive_bare_tool_rows -- --nocapture`
Expected: FAIL — `⏺ Bash` appears 5 times, no `×5`.

- [ ] **Step 3: Implement**

Locale keys:

```yaml
# en.yml, under app.activity:
    repeat_suffix: " ×%{count}"
# zh.yml:
    repeat_suffix: " ×%{count}"
```

New helper in `transcript_build.rs` (place next to `push_agent_task_child`):

```rust
/// Emit a group's child rows with run-length merging: consecutive items that
/// share a display name, carry NO invocation text, and have the same
/// failed/running status collapse to one `⏺ Bash ×N` row. Rows with a real
/// invocation always render individually (the command IS the information).
fn push_agent_task_children(
    lines: &mut Vec<Line<'static>>,
    palette: Palette,
    items: &[&ActivityItem],
    suppress_first_connector: bool,
    expanded: bool,
    wrap_width: usize,
    show_output: bool,
) {
    let mut idx = 0;
    let mut emitted = 0usize;
    while idx < items.len() {
        let item = items[idx];
        let bare = tool_invocation_text(item)
            .map(|text| text.trim().is_empty())
            .unwrap_or(true);
        let mut run = 1;
        if bare {
            while idx + run < items.len() {
                let next = items[idx + run];
                let next_bare = tool_invocation_text(next)
                    .map(|text| text.trim().is_empty())
                    .unwrap_or(true);
                if next_bare
                    && tool_display_name(&next.title) == tool_display_name(&item.title)
                    && activity_is_failed(next) == activity_is_failed(item)
                    && is_running_activity(next) == is_running_activity(item)
                {
                    run += 1;
                } else {
                    break;
                }
            }
        }
        let first = emitted == 0 && !suppress_first_connector;
        if run > 1 {
            let (bullet, bullet_style) = tool_card_bullet(item, palette);
            let spans = clip_line_spans(
                vec![
                    Span::styled(TOOL_CARD_CHILD_INDENT, palette.border()),
                    Span::styled(format!("{bullet} "), bullet_style),
                    Span::styled(tool_display_name(&item.title), palette.text()),
                    Span::styled(
                        t!("app.activity.repeat_suffix", count = run).into_owned(),
                        palette.muted(),
                    ),
                ],
                wrap_width,
            );
            lines.push(Line::from(spans));
        } else {
            push_agent_task_child(lines, palette, item, first, expanded, wrap_width, show_output);
        }
        emitted += 1;
        idx += run;
    }
}
```

Replace the child loop in `push_agent_task_group` with:

```rust
push_agent_task_children(
    lines,
    palette,
    items,
    /* suppress_first_connector = */ false,
    expanded,
    wrap_width,
    !collapse_settled,
);
```

and the `append_children_only` block's loop with the same call using
`suppress_first_connector = true`.

- [ ] **Step 4: Run the full suite**

Run: `cargo test --lib`
Expected: PASS. Update any test that pinned per-row bare output (search failures for `⏺ Bash` count assertions) to the merged contract with a comment.

- [ ] **Step 5: Commit**

```bash
git add src/app/transcript_build.rs src/app/tests.rs locales/en.yml locales/zh.yml
git commit -m "feat(tui): run-length merge identical bare tool rows (⏺ Bash ×N)"
```

---

### Task 2: `◈ N more` fold row + terminal-height-adaptive display cap

**Files:**
- Modify: `src/app/transcript_build.rs` (search `older_actions` for the fold-row emission; search for the constant that slices `shown` from the full item list — the display cap)
- Modify: `locales/en.yml`, `locales/zh.yml`
- Test: `src/app/tests.rs`

**Interfaces:**
- Produces: `fn activity_display_cap(terminal_height: u16) -> usize` — `5` when height < 30, else `12` (mirrors `min_transcript_height`'s breakpoint).
- Fold row copy changes from muted `... +N older action(s)` to highlighted `◈ N more · Ctrl+O`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn folded_activity_renders_prominent_more_row_with_expand_hint() {
    // 12-item fixture already exists:
    // render_large_completed_turn_activity_log_is_compact_by_default
    // pins "... +9 more". Re-pin to the new fold style.
    let turn_id = TurnId::new();
    let session_id = SessionKey("local:test".into());
    let mut app = capsule_app(&session_id, &turn_id);
    app.activity.clear();
    for i in 0..15 {
        app.activity
            .push(capsule_tool_item(&turn_id, &format!("c{i}"), &format!("cmd-{i}")));
    }

    let text = rendered_text(&app);

    assert!(text.contains("◈"), "fold row uses the ◈ glyph: {text}");
    assert!(text.contains("more"), "fold row says N more: {text}");
    assert!(text.contains("Ctrl+O"), "fold row advertises expand: {text}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib folded_activity_renders_prominent -- --nocapture`
Expected: FAIL — current copy is `... +N older action(s)` with no ◈/Ctrl+O.

- [ ] **Step 3: Implement**

Locale:

```yaml
# en.yml app.activity:
    fold_more: "◈ %{count} more · Ctrl+O"
# zh.yml:
    fold_more: "◈ 还有 %{count} 条 · Ctrl+O 展开"
```

At the fold-row emission site (where `app.activity.older_actions` is used),
swap key and style:

```rust
lines.push(Line::from(Span::styled(
    format!(
        "{}{}",
        TOOL_CARD_CHILD_INDENT,
        t!("app.activity.fold_more", count = hidden)
    ),
    palette.selected(),
)));
```

Add the adaptive cap next to the fold site and use it wherever the `shown`
slice is cut (replace the existing constant):

```rust
/// Rows of settled activity shown before folding into `◈ N more`. Short
/// terminals keep the fold tight; tall terminals can afford more context.
/// Mirrors `min_transcript_height`'s 30-row breakpoint.
fn activity_display_cap(terminal_height: u16) -> usize {
    if terminal_height < 30 { 5 } else { 12 }
}
```

(The call site already threads a height — if it only has `wrap_width`, pass
the height down from the caller the same way `agent_strip_height` receives
it. Keep the old constant as the fallback where no height is in scope.)

- [ ] **Step 4: Run the full suite**

Run: `cargo test --lib`
Expected: PASS after updating `render_large_completed_turn_activity_log_is_compact_by_default`'s `"... +9 more"` pin to the new copy.

- [ ] **Step 5: Commit**

```bash
git add src/app/transcript_build.rs src/app/tests.rs locales/en.yml locales/zh.yml
git commit -m "feat(tui): prominent ◈ N more fold row with adaptive display cap"
```

---

### Task 3: Scrollback batch digest line

**Files:**
- Modify: `src/app/transcript_build.rs` (`finalized_live_turn_lines_between` — the `continuation` branch added 2026-08-02)
- Test: `src/app/tests.rs` (next to `same_turn_activity_delta_appends_children_without_repeating_the_header`)

**Interfaces:**
- Produces: `fn activity_batch_digest(items: &[&ActivityItem]) -> Option<String>` — `Some("Bash ×4 · Read ×1")` when EVERY item is a settled, successful, invocation-less tool row and `items.len() >= 3`; `None` otherwise (fall back to per-row emission so failures/commands stay visible in the immutable archive).
- Consumes: `tool_display_name`, `tool_invocation_text`, `activity_is_failed`, `is_running_activity`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn continuation_batch_of_bare_successes_flushes_as_one_digest_line() {
    let turn_id = TurnId::new();
    let session_id = SessionKey("local:test".into());
    let mut app = capsule_app(&session_id, &turn_id);
    let palette = Palette::for_theme(ThemeName::Slate);
    let first = next_live_turn_finalization(&app, None).expect("first watermark");

    for id in ["c2", "c3", "c4", "c5"] {
        app.activity.push(bare_tool_item(&turn_id, id));
    }
    let second = next_live_turn_finalization(&app, Some(&first)).expect("second watermark");
    let batch = line_texts(&finalized_live_turn_lines_between(
        &app, palette, 100, &first, &second, true,
    ));

    assert_eq!(
        batch.len(),
        1,
        "4 bare successes flush as ONE digest row: {batch:#?}"
    );
    assert!(batch[0].contains("Bash ×4"), "digest names the tools: {batch:#?}");
}

#[test]
fn continuation_batch_with_a_failure_keeps_per_row_detail() {
    let turn_id = TurnId::new();
    let session_id = SessionKey("local:test".into());
    let mut app = capsule_app(&session_id, &turn_id);
    let palette = Palette::for_theme(ThemeName::Slate);
    let first = next_live_turn_finalization(&app, None).expect("first watermark");

    for id in ["c2", "c3"] {
        app.activity.push(bare_tool_item(&turn_id, id));
    }
    app.activity.push(
        ActivityItem::new(ActivityKind::Tool, "shell", "complete")
            .with_turn(turn_id.clone())
            .with_tool_call("c4")
            .with_success(false),
    );
    let second = next_live_turn_finalization(&app, Some(&first)).expect("second watermark");
    let batch = line_texts(&finalized_live_turn_lines_between(
        &app, palette, 100, &first, &second, true,
    ));

    assert!(
        batch.len() >= 3,
        "a failed row forbids the digest — the archive keeps detail: {batch:#?}"
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib continuation_batch_of_bare -- --nocapture`
Expected: FAIL — currently 4 individual rows (or 1 merged row from Task 1; the digest asserts the `Bash ×4` SINGLE-line shape via the continuation path, which Task 1 already gives for same-name runs — this test additionally pins mixed-name digests; keep it even if it passes for the same-name case, then extend the fixture with one `read_file` item to force the mixed case: expected `Bash ×4 · Read ×1`).

- [ ] **Step 3: Implement**

```rust
/// One-line digest for a scrollback continuation batch: `Bash ×4 · Read ×1`.
/// Only for batches of >=3 settled, successful, invocation-less rows — any
/// failure, running item, or real command keeps per-row detail, because the
/// immutable archive is the audit trail.
fn activity_batch_digest(items: &[&ActivityItem]) -> Option<String> {
    if items.len() < 3 {
        return None;
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for item in items {
        let bare = tool_invocation_text(item)
            .map(|text| text.trim().is_empty())
            .unwrap_or(true);
        if !bare || activity_is_failed(item) || is_running_activity(item) {
            return None;
        }
        let name = tool_display_name(&item.title);
        match counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => *c += 1,
            None => counts.push((name, 1)),
        }
    }
    Some(
        counts
            .into_iter()
            .map(|(name, count)| format!("{name} ×{count}"))
            .collect::<Vec<_>>()
            .join(" · "),
    )
}
```

In the `continuation` branch of `finalized_live_turn_lines_between`, before
calling `push_finalized_activity_items_section`:

```rust
if continuation {
    if let Some(digest) = activity_batch_digest(&new_activity) {
        let palette_bullet = Style::default().fg(palette.success);
        lines.push(Line::from(vec![
            Span::styled(TOOL_CARD_CHILD_INDENT, palette.border()),
            Span::styled("⏺ ", palette_bullet),
            Span::styled(digest, palette.muted()),
        ]));
        strip_lines_background(&mut lines);
        return lines;
    }
}
```

- [ ] **Step 4: Run the full suite**

Run: `cargo test --lib`
Expected: PASS (capsule tests from 2026-08-02 use items WITH invocations, so they keep per-row flushing and stay green).

- [ ] **Step 5: Commit**

```bash
git add src/app/transcript_build.rs src/app/tests.rs
git commit -m "feat(tui): flush bare-success continuation batches as one digest line"
```

---

### Task 4: One-line live summary on the harness row

**Files:**
- Modify: `src/app.rs` (`harness_status_lines`, after the `running_agents` segment)
- Modify: `locales/en.yml`, `locales/zh.yml`
- Test: `src/app/tests.rs`

**Interfaces:**
- Produces: a `· N actions` segment on the harness status row whenever the active turn has settled activity, sourced from `flow_activity_items(app).len()`.
- Consumes: existing `app.statusbar.agents` segment pattern in `harness_status_lines`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn harness_row_summarizes_live_action_count() {
    let turn_id = TurnId::new();
    let session_id = SessionKey("local:test".into());
    let mut app = capsule_app(&session_id, &turn_id);
    for id in ["c2", "c3", "c4"] {
        app.activity.push(bare_tool_item(&turn_id, id));
    }

    let text = rendered_text(&app);

    assert!(
        text.contains("4 actions") || text.contains("4 个动作"),
        "harness row carries the running action count: {text}"
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib harness_row_summarizes -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Implement**

Locale:

```yaml
# en.yml app.statusbar:
    actions: "%{count} actions"
# zh.yml:
    actions: "%{count} 个动作"
```

In `harness_status_lines`, directly after the `running_agents` push:

```rust
let action_count = flow_activity_items(app).len();
if action_count > 0 {
    spans.push(Span::styled(
        format!(" · {}", t!("app.statusbar.actions", count = action_count)),
        palette.muted().bg(palette.surface),
    ));
}
```

(`flow_activity_items` lives in `transcript_build.rs` as `fn` private to the
`app` module tree — it is callable from `app.rs` via `transcript_build::`;
make it `pub(super)` if it is not already.)

- [ ] **Step 4: Run the full suite**

Run: `cargo test --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/app/tests.rs locales/en.yml locales/zh.yml
git commit -m "feat(tui): live action-count summary on the harness status row"
```

---

## Out of scope (explicit)

- A user-facing `activity-style: compact|full` config toggle — merging only affects information-free duplicate rows, so no toggle is needed (YAGNI). Revisit only if users ask for the verbose archive back.
- Editing already-flushed scrollback (impossible — immutable).
- `/activity` navigator changes — it already provides full detail and is the escape hatch this plan relies on.
