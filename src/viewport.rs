//! Inline-viewport driver: owns the scrollback-flush bookkeeping that turns
//! octoscode's "rebuild everything every frame" model into codex's "finalized
//! history → scrollback, live UI → inline viewport" model.
//!
//! The event loop ([`crate::event_loop`]) calls [`ScrollbackTracker::sync`] each
//! time it is about to draw. The tracker compares the committed message history
//! to what it has already pushed into the terminal's scrollback and returns the
//! *new* finalized lines to insert (and whether the prior scrollback must be
//! reset first, e.g. on a session switch or a hydrate that replaced history).
//!
//! Keeping this state in one small, unit-tested type — separate from the
//! escape-sequence emitter and the render code — is what makes the rearchitecture
//! reviewable: the "what is finalized" decision lives here, the "how to draw the
//! live UI" decision lives in [`crate::app`], and the "how to write scrollback"
//! mechanism lives in [`crate::insert_history`].

use ratatui::text::Line;

use crate::app::{self, CommittedFingerprint, LiveTurnFinalization};
use crate::model::AppState;
use crate::theme::Palette;

/// Tracks how much committed history has been flushed to terminal scrollback so
/// that each draw only appends the *newly finalized* lines.
#[derive(Debug, Default)]
pub struct ScrollbackTracker {
    /// Fingerprint of the committed history we last flushed.
    last: CommittedFingerprint,
    /// Number of committed messages already flushed for `last.session_id`.
    flushed_messages: usize,
    /// Active-turn content already streamed to scrollback while still live.
    active_live: Option<LiveTurnFinalization>,
    /// Recently completed live-turn watermarks kept long enough to dedupe the
    /// eventual committed assistant message / archived activity log.
    completed_live: Vec<LiveTurnFinalization>,
    /// Already-rendered archived activity, retained with the canonical logs
    /// independently of the short-lived reply streaming cache.
    archived_activity: Vec<LiveTurnFinalization>,
    /// Whether the last line pushed to scrollback was blank. Reply chunks stream
    /// in across many flushes; without carrying this, a chunk that ends on a
    /// blank followed by one that opens on a blank stacks into a 2-line gap at
    /// the seam (per-flush collapse can't see across flushes). Seeds the next
    /// flush's blank-run collapse so cross-flush seams close to one blank.
    last_flushed_ends_blank: bool,
    /// Whether the previous live tail had guarded sections (activity or pending
    /// messages) whose separator rows can become orphaned when that tail settles
    /// and shrinks away.
    live_tail_had_guarded_sections: bool,
    /// Turn whose flushed activity group is the CURRENT scrollback tail, if
    /// any. While it stays the tail, further settle batches of the same turn
    /// append child rows under the existing header instead of opening a new
    /// "Agent task completed" block per batch (the k3 header-spam). Any other
    /// insert (reply text, committed history) clears it, so continuation rows
    /// can never orphan under unrelated content.
    tail_activity_turn: Option<String>,
}

/// What the event loop should do with scrollback before drawing the viewport.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScrollbackUpdate {
    /// Lines to insert into scrollback above the inline viewport, in order.
    pub lines_to_insert: Vec<Line<'static>>,
    /// When true, the previously flushed scrollback is stale (session switch or
    /// a hydrate that replaced history). The caller cannot un-write real
    /// scrollback, but it should treat `lines_to_insert` as a fresh full
    /// re-flush of the (now-current) committed history rather than an append.
    pub reset: bool,
    /// Watermark to use when rendering the inline live tail for this draw.
    pub live_tail_finalization: Option<LiveTurnFinalization>,
}

impl ScrollbackTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the COMMITTED flush watermark only, so the next [`Self::sync`]
    /// re-emits the entire committed history — while PRESERVING the live-turn
    /// watermarks (`active_live` / `completed_live`). Used when a `/btw`
    /// aside is dismissed: the viewport shrink strands a blank band that a
    /// committed re-flush fills, but the main turn may still be STREAMING —
    /// wiping the live watermarks would re-emit its already-streamed rows
    /// (they survive on screen; only the old viewport region was cleared)
    /// and duplicate them in scrollback (codex P2 on #288). The preserved
    /// `completed_live` also keeps the committed re-flush deduped against
    /// content that already streamed live.
    pub fn mark_committed_flush_stale(&mut self) {
        self.last = CommittedFingerprint::default();
        self.flushed_messages = 0;
        self.archived_activity.clear();
        self.last_flushed_ends_blank = false;
        // The committed re-flush will write below the old tail; a group header
        // up there is no longer the attach point for continuation rows.
        self.tail_activity_turn = None;
    }

    /// Forget everything already flushed, so the next [`Self::sync`] re-emits
    /// the ENTIRE committed history (plus any already-streamed live-turn
    /// content) as a fresh first flush.
    ///
    /// Used when the terminal takes the full viewport reset path on a resize
    /// (width change either direction, or terminal-height shrink — see
    /// `Terminal::resize_viewport_to`): that reset clears the whole visible
    /// screen, erasing the transcript rows this tracker had flushed there.
    /// Without a re-flush the chat visually vanishes — a bare composer on an
    /// empty screen — with the pre-resize copy reachable only by scrolling
    /// real scrollback, wrapped at the old width.
    pub fn mark_flushed_stale(&mut self) {
        *self = Self::new();
    }

    /// Reconcile the tracker against the current app state and return the lines
    /// to push into scrollback. `wrap_width` is the inline-viewport width.
    pub fn sync(
        &mut self,
        app: &AppState,
        palette: Palette,
        wrap_width: usize,
    ) -> ScrollbackUpdate {
        let fingerprint = app::committed_messages_fingerprint(app);
        let previous_live_tail_had_guarded_sections = self.live_tail_had_guarded_sections;
        let (previous_live, next_live) = self.reconcile_active_live(app);
        let mut lines_to_insert = Vec::new();
        let mut reset = false;
        let mut activity_coverage = self.archived_activity.clone();
        for live in &self.completed_live {
            if let Some(old) = activity_coverage
                .iter_mut()
                .find(|old| old.session_id == live.session_id && old.turn_id == live.turn_id)
            {
                // Preserve reply streaming ownership while adding archived
                // activity keys for logs whose render anchor moves later.
                old.reply_flushed_text = live.reply_flushed_text.clone();
                old.summary_flushed |= live.summary_flushed;
                for key in &live.activity_flushed_keys {
                    if !old.activity_flushed_keys.contains(key) {
                        old.activity_flushed_keys.push(key.clone());
                    }
                }
                old.activity_flushed_items = old.activity_flushed_keys.len();
            } else {
                activity_coverage.push(live.clone());
            }
        }

        // No active session, or no committed messages yet → no committed
        // history to flush. Live-turn deltas below may still insert lines.
        if fingerprint.message_count == 0 {
            // Keep the session id so a later first message is treated as an
            // append, not a reset (avoids a spurious reset on the first flush).
            self.last = fingerprint;
            self.flushed_messages = 0;
        } else {
            // A fresh tracker (nothing flushed yet) treats its first flush as an
            // append of the whole current history, not a reset — there is no
            // prior scrollback to invalidate.
            let first_flush = self.flushed_messages == 0 && self.last.session_id.is_empty();
            let is_extension = first_flush
                || (fingerprint.session_id == self.last.session_id
                    && fingerprint.message_count >= self.flushed_messages
                    && is_prefix_preserved(&self.last, &fingerprint, self.flushed_messages));
            if is_extension {
                // Append only the messages we have not flushed yet. If a live
                // turn was already streamed into scrollback, skip the covered
                // prefix when the committed assistant/log catches up.
                let committed_start = self.flushed_messages;
                lines_to_insert.extend(app::finalized_late_activity_lines_for_coverages(
                    app,
                    palette,
                    wrap_width,
                    &activity_coverage,
                    committed_start,
                ));
                lines_to_insert.extend(app::finalized_history_lines_range_dedup_live(
                    app,
                    palette,
                    wrap_width,
                    committed_start,
                    &activity_coverage,
                ));
                self.flushed_messages = fingerprint.message_count;
                self.last = fingerprint;
                self.refresh_completed_live_coverage(app, Some(committed_start));
            } else {
                // Discontinuity: session switch or hydrate replaced history. We
                // cannot remove already-written scrollback, but we re-flush the
                // full current history so the up-to-date content is selectable
                // below the (now-stale) prior block. Rare (reconnect / session
                // switch).
                //
                // goal_05 candidate (a): defer syntect highlighting for this
                // full rebuild — cold-cache highlighting of every code block in
                // history measured at 88% of the rebuild cost and 31-38% of the
                // whole load. The scrollback snapshot shows code blocks in the
                // plain fallback style; the live viewport and the pager rebuild
                // every frame and highlight normally.
                lines_to_insert.extend(crate::highlight::with_deferred_highlight(|| {
                    app::finalized_history_lines(app, palette, wrap_width)
                }));
                self.flushed_messages = fingerprint.message_count;
                self.last = fingerprint;
                self.completed_live.clear();
                reset = true;
            }
        }
        self.archived_activity = if self.flushed_messages == 0 {
            // No committed rendering ran in this frame. In particular, an
            // unanchored late log must not be acknowledged before history arrives.
            Vec::new()
        } else {
            app::committed_activity_coverages(app, if reset { &[] } else { &activity_coverage })
        };

        let mut delta_activity_turn = None;
        if let Some(next) = next_live.as_ref() {
            let baseline = previous_live
                .clone()
                .unwrap_or_else(|| LiveTurnFinalization {
                    session_id: next.session_id.clone(),
                    turn_id: next.turn_id.clone(),
                    reply_flushed_text: String::new(),
                    activity_flushed_items: 0,
                    activity_flushed_keys: Vec::new(),
                    summary_flushed: false,
                });
            // Capsule continuation is only offered when nothing was inserted
            // ahead of this delta in the SAME buffer (committed lines would
            // land between the old header and the new child rows).
            let append_to_flushed_group = lines_to_insert.is_empty()
                && self.tail_activity_turn.as_deref() == Some(next.turn_id.as_str());
            if next.activity_flushed_keys.len() > baseline.activity_flushed_keys.len() {
                delta_activity_turn = Some(next.turn_id.clone());
            }
            lines_to_insert.extend(app::finalized_live_turn_lines_between(
                app,
                palette,
                wrap_width,
                &baseline,
                next,
                append_to_flushed_group,
            ));
        }
        self.active_live = next_live.filter(LiveTurnFinalization::has_flushed_content);
        self.live_tail_had_guarded_sections =
            app::live_tail_has_guarded_sections(app, self.active_live.as_ref());

        // A single flush concatenates committed history + live-turn deltas, each
        // of which guards only its own separators; their seam can stack into a
        // multi-line gap. Collapse runs on the combined buffer — seeded with
        // whether the previously flushed scrollback line was blank, so blanks
        // stacked across the many small reply-streaming flushes also close to a
        // single blank. On a reset the prior scrollback is stale, so don't carry
        // the seam across it.
        let seam_seed = !reset && self.last_flushed_ends_blank;
        let drop_orphaned_leading_blank_run = !reset
            && previous_live_tail_had_guarded_sections
            && !self.live_tail_had_guarded_sections;
        self.last_flushed_ends_blank = app::collapse_blank_runs_seeded_orphan_guard(
            &mut lines_to_insert,
            seam_seed,
            drop_orphaned_leading_blank_run,
        );

        // Track whether the scrollback tail is now an activity group (the
        // activity section is always the LAST content of a flush buffer). A
        // flush that wrote anything else moves the tail off the group; an
        // empty flush leaves it in place.
        if reset {
            self.tail_activity_turn = None;
        }
        if !lines_to_insert.is_empty() {
            self.tail_activity_turn = delta_activity_turn;
        }

        ScrollbackUpdate {
            lines_to_insert,
            reset,
            live_tail_finalization: self.active_live.clone(),
        }
    }

    fn reconcile_active_live(
        &mut self,
        app: &AppState,
    ) -> (Option<LiveTurnFinalization>, Option<LiveTurnFinalization>) {
        let previous = self.active_live.take();
        let matching_previous = match (previous, app.active_turn()) {
            (Some(previous), Some((session_id, turn_id)))
                if previous.matches_turn(session_id, turn_id) =>
            {
                Some(previous)
            }
            (Some(previous), _) => {
                self.archive_completed_live(previous);
                None
            }
            (None, _) => None,
        };
        let next = app::next_live_turn_finalization(app, matching_previous.as_ref());
        (matching_previous, next)
    }

    fn archive_completed_live(&mut self, finalization: LiveTurnFinalization) {
        if !finalization.has_flushed_content() {
            return;
        }
        if let Some(existing) = self.completed_live.iter_mut().find(|existing| {
            existing.session_id == finalization.session_id
                && existing.turn_id == finalization.turn_id
        }) {
            *existing = finalization;
        } else {
            self.completed_live.push(finalization);
        }
        const MAX_COMPLETED_LIVE_TURNS: usize = 8;
        if self.completed_live.len() > MAX_COMPLETED_LIVE_TURNS {
            let excess = self.completed_live.len() - MAX_COMPLETED_LIVE_TURNS;
            self.completed_live.drain(0..excess);
        }
    }

    fn refresh_completed_live_coverage(&mut self, app: &AppState, reply_start: Option<usize>) {
        for coverage in &mut self.completed_live {
            if let Some(start) = reply_start
                && app::committed_reply_matches_live_finalization(app, start, coverage)
            {
                coverage.reply_flushed_text.clear();
            }
            if let Some(committed_keys) =
                app::committed_activity_keys_for_live_finalization(app, coverage)
            {
                for key in committed_keys {
                    if !coverage.activity_flushed_keys.contains(&key) {
                        coverage.activity_flushed_keys.push(key);
                    }
                }
                coverage.activity_flushed_items = coverage.activity_flushed_keys.len();
            }
        }
    }
}

/// Validate exactly the already-flushed dialogue prefix even when new rows
/// append. Activity metadata has its own delta coverage; it is not a rewrite
/// of immutable user/assistant messages.
fn is_prefix_preserved(
    last: &CommittedFingerprint,
    next: &CommittedFingerprint,
    flushed: usize,
) -> bool {
    next.message_count >= flushed
        && (flushed == 0
            || last
                .message_prefix_hashes
                .get(flushed - 1)
                .is_some_and(|hash| next.message_prefix_hashes.get(flushed - 1) == Some(hash)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ThemeName;
    use crate::model::{
        ActivityItem, ActivityKind, AppState, LiveReply, TurnActivityLog, TurnPromptAnchor,
    };
    use octos_core::Message;
    use octos_core::SessionKey;
    use octos_core::app_ui::AppUiSession;
    use octos_core::ui_protocol::TurnId;

    fn palette() -> Palette {
        Palette::for_theme(ThemeName::Slate)
    }

    fn session_with(messages: Vec<Message>) -> AppUiSession {
        AppUiSession {
            id: SessionKey("local:test".into()),
            title: "t".into(),
            profile_id: None,
            messages,
            tasks: Vec::new(),
            live_reply: None,
        }
    }

    fn state(messages: Vec<Message>) -> AppState {
        AppState::new(vec![session_with(messages)], 0, "ready".into(), None, false)
    }

    #[test]
    fn first_flush_emits_all_committed_messages() {
        let app = state(vec![Message::user("hi"), Message::assistant("hello there")]);
        let mut tracker = ScrollbackTracker::new();
        let update = tracker.sync(&app, palette(), 60);
        assert!(!update.reset);
        assert!(
            !update.lines_to_insert.is_empty(),
            "expected committed lines to flush"
        );
    }

    /// Peer operator console (FIX-4 reverted, user-found empty-view bug):
    /// switching focus to a peer — even a FINISHED one (committed messages,
    /// `live_reply == None`, so no live tail to render) — must show that peer's
    /// transcript, NOT an empty view. On a session switch the tracker is
    /// `mark_flushed_stale` (the menu-close path), so it re-syncs against the
    /// now-focused peer; that sync must flush the peer's committed history.
    #[test]
    fn focused_finished_peer_flushes_its_committed_transcript() {
        let master = AppUiSession {
            id: SessionKey("local:main".into()),
            title: "master".into(),
            profile_id: None,
            messages: vec![Message::user("master q"), Message::assistant("master a")],
            tasks: Vec::new(),
            live_reply: None,
        };
        let peer = AppUiSession {
            id: SessionKey("local:main#peer-refactor".into()),
            title: "peer".into(),
            profile_id: None,
            messages: vec![
                Message::user("peer question"),
                Message::assistant("peer answer text"),
            ],
            tasks: Vec::new(),
            live_reply: None, // FINISHED: no live tail.
        };
        let mut app = AppState::new(vec![master, peer], 1, "ready".into(), None, false);
        app.opened_peer_sessions
            .insert(SessionKey("local:main#peer-refactor".into()));
        assert!(
            app.focused_session_is_peer(),
            "focused on the finished peer"
        );

        // A session switch stales the tracker (menu-close `mark_flushed_stale`);
        // a fresh tracker models that. The re-sync must flush the peer's history.
        let mut tracker = ScrollbackTracker::new();
        let update = tracker.sync(&app, palette(), 60);
        let flushed: String = update
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        assert!(
            flushed.contains("peer question") && flushed.contains("peer answer text"),
            "a focused finished peer's committed transcript is flushed, not empty; got: {flushed}"
        );
    }

    #[test]
    fn mark_committed_flush_stale_preserves_live_watermarks() {
        // A /btw dismissal re-flushes committed history while the main turn
        // may still be streaming: the live watermarks must survive so (a)
        // already-streamed live rows are not re-emitted (they survive on
        // screen — only the old viewport region is cleared on shrink) and
        // (b) the committed re-flush stays deduped against content that
        // already streamed live.
        let mut tracker = ScrollbackTracker::new();
        let app = state(vec![Message::user("hi"), Message::assistant("a1")]);
        let first = tracker.sync(&app, palette(), 60);
        assert!(!first.lines_to_insert.is_empty());

        let live = LiveTurnFinalization {
            session_id: "local:test".into(),
            turn_id: "turn-1".into(),
            reply_flushed_text: "streamed so far".into(),
            activity_flushed_items: 2,
            activity_flushed_keys: vec!["k1".into(), "k2".into()],
            summary_flushed: false,
        };
        tracker.active_live = Some(live.clone());
        tracker.completed_live = vec![live.clone()];
        tracker.live_tail_had_guarded_sections = true;

        tracker.mark_committed_flush_stale();

        assert_eq!(
            tracker.active_live.as_ref().map(|l| l.turn_id.as_str()),
            Some("turn-1"),
            "active live watermark must survive"
        );
        assert_eq!(
            tracker.completed_live.len(),
            1,
            "completed live dedup watermarks must survive"
        );
        assert!(tracker.live_tail_had_guarded_sections);
        assert_eq!(tracker.flushed_messages, 0, "committed watermark reset");
    }

    #[test]
    fn mark_flushed_stale_reflushes_the_whole_transcript() {
        // The width-change full viewport reset clears the visible screen,
        // erasing the transcript rows already flushed there. After
        // mark_flushed_stale the next sync must re-emit the ENTIRE committed
        // history (as a plain flush, not a reset — the screen was already
        // cleared by the terminal), so the chat reappears freshly wrapped.
        let mut tracker = ScrollbackTracker::new();
        let app = state(vec![Message::user("hi"), Message::assistant("a1")]);
        let first = tracker.sync(&app, palette(), 60);
        assert!(!first.lines_to_insert.is_empty());

        let settled = tracker.sync(&app, palette(), 60);
        assert!(
            settled.lines_to_insert.is_empty(),
            "no growth -> nothing to flush"
        );

        tracker.mark_flushed_stale();
        let reflushed = tracker.sync(&app, palette(), 50);
        assert!(!reflushed.reset, "the terminal reset already cleared");
        assert_eq!(
            reflushed.lines_to_insert.len(),
            app::finalized_history_lines(&app, palette(), 50).len(),
            "must re-emit the full committed history at the new width"
        );

        // ...and the tracker keeps working incrementally afterwards.
        let after = tracker.sync(&app, palette(), 50);
        assert!(after.lines_to_insert.is_empty());
    }

    #[test]
    fn appending_a_message_flushes_only_the_new_one() {
        let mut tracker = ScrollbackTracker::new();
        let app1 = state(vec![Message::user("hi"), Message::assistant("a1")]);
        let first = tracker.sync(&app1, palette(), 60);
        let first_count = first.lines_to_insert.len();
        assert!(first_count > 0);

        let app2 = state(vec![
            Message::user("hi"),
            Message::assistant("a1"),
            Message::user("again"),
            Message::assistant("a2"),
        ]);
        let second = tracker.sync(&app2, palette(), 60);
        assert!(!second.reset, "append should not reset");
        assert!(
            !second.lines_to_insert.is_empty(),
            "expected the new messages to flush"
        );
        // The second flush is only the 2 new messages, so it is smaller than a
        // full re-flush of all 4 messages would be.
        let full = app::finalized_history_lines(&app2, palette(), 60);
        assert!(
            second.lines_to_insert.len() < full.len(),
            "append flush ({}) should be smaller than full ({})",
            second.lines_to_insert.len(),
            full.len()
        );
    }

    #[test]
    fn continuation_coverage_never_consumes_another_turn_with_the_same_prefix() {
        let turn = TurnId::new();
        let other = TurnId::new();
        let prefix = "Background result is ready.\n\n";
        let mut app = state(vec![
            Message::user("start task"),
            Message::assistant("foreground ack"),
        ]);
        let session_id = app.sessions[0].id.clone();
        app.turn_prompt_anchors.push(TurnPromptAnchor {
            session_id: session_id.clone(),
            turn_id: turn.clone(),
            content: "start task".into(),
            anchor_index: 0,
            prior_matching_user_count: 0,
        });
        // Both continuations share a user anchor. Identity must beat a prefix
        // even when the other turn commits first in the same flush.
        app.sessions[0]
            .messages
            .push(Message::assistant_with_thread(
                format!("{prefix}Other result."),
                octos_core::ThreadId::new(other.0.to_string()),
            ));
        app.sessions[0]
            .messages
            .push(Message::assistant_with_thread(
                format!("{prefix}Own result."),
                octos_core::ThreadId::new(turn.0.to_string()),
            ));
        let coverage = LiveTurnFinalization {
            session_id: session_id.0,
            turn_id: turn.0.to_string(),
            reply_flushed_text: prefix.into(),
            ..Default::default()
        };
        let lines =
            app::finalized_history_lines_range_dedup_live(&app, palette(), 80, 2, &[coverage]);
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.find("Background result is ready.").unwrap() < text.find("Other result.").unwrap(),
            "other turn must retain its prefix: {text}"
        );
        assert_eq!(
            text.matches("Background result is ready.").count(),
            1,
            "only the matching turn was already flushed: {text}"
        );
    }

    #[test]
    fn legacy_reply_with_shared_anchor_retains_ambiguous_prefix() {
        let turn = TurnId::new();
        let other = TurnId::new();
        let mut app = state(vec![
            Message::user("shared request"),
            Message::assistant("Same prefix.\n\nOther answer."),
        ]);
        let session_id = app.sessions[0].id.clone();
        for turn_id in [turn.clone(), other] {
            app.turn_prompt_anchors.push(TurnPromptAnchor {
                session_id: session_id.clone(),
                turn_id,
                content: "shared request".into(),
                anchor_index: 0,
                prior_matching_user_count: 0,
            });
        }
        let coverage = LiveTurnFinalization {
            session_id: session_id.0,
            turn_id: turn.0.to_string(),
            reply_flushed_text: "Same prefix.\n\n".into(),
            ..Default::default()
        };
        let lines =
            app::finalized_history_lines_range_dedup_live(&app, palette(), 80, 1, &[coverage]);
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("Same prefix."))
        );
    }

    #[test]
    fn server_continuation_reuses_prompt_anchor_without_reflushing_live_prefix() {
        // Server-initiated continuations (background child completion / scatter
        // join synthesis) do not add a new user row. Their turn anchor therefore
        // points at the same user row as the foreground turn immediately before
        // them. The old reply-index inference resolved that anchor to the FIRST
        // assistant after the user row ("foreground ack"), rejected the live
        // continuation coverage, and flushed the continuation a second time at
        // commit even though its canonical prefix was already in scrollback.
        let session_id = SessionKey("local:test".into());
        let turn_id = TurnId::new();
        let answer = "Background result is ready.\n\nNo follow-up work remains.";
        let mut app = state(vec![
            Message::user("start background task"),
            Message::assistant("foreground ack"),
        ]);
        app.sessions[0].live_reply = Some(LiveReply {
            turn_id: turn_id.clone(),
            text: answer.into(),
        });
        app.turn_prompt_anchors.push(TurnPromptAnchor {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            content: "start background task".into(),
            anchor_index: 0,
            prior_matching_user_count: 0,
        });

        let mut tracker = ScrollbackTracker::new();
        let live = tracker.sync(&app, palette(), 80);
        let live_text = live
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            live_text.contains("Background result is ready."),
            "the completed first paragraph should have streamed to scrollback: {live_text:?}"
        );

        app.sessions[0].live_reply = None;
        app.sessions[0]
            .messages
            .push(Message::assistant_with_thread(
                answer,
                octos_core::ThreadId::new(turn_id.0.to_string()),
            ));
        let committed = tracker.sync(&app, palette(), 80);
        let committed_text = committed
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");

        assert!(
            !committed_text.contains("Background result is ready."),
            "the live-flushed canonical prefix must not be emitted twice: {committed_text:?}"
        );
        assert!(
            committed_text.contains("No follow-up work remains."),
            "only the unflushed suffix should be emitted at commit: {committed_text:?}"
        );
    }

    /// P2-16: a coverage archived for a turn that ERRORED must not dedup a
    /// later turn's committed reply on direct-prefix evidence. T1 streams
    /// "Background result is ready." then errors; its committed row is the
    /// failure card (no shared prefix), so T1's coverage survives. T2 later
    /// commits a fresh reply that starts with the same paragraph: it must
    /// reach scrollback in full, not be skipped as "already flushed for T1"
    /// with its suffix rendered bullet-less under T1's stale block.
    #[test]
    fn errored_turn_coverage_does_not_dedup_a_later_same_prefix_reply() {
        let session_id = SessionKey("local:test".into());
        let errored_turn = TurnId::new();
        let shared_prefix = "Background result is ready.";
        let mut app = state(vec![
            Message::user("start background task"),
            Message::assistant("foreground ack"),
        ]);
        app.sessions[0].live_reply = Some(LiveReply {
            turn_id: errored_turn.clone(),
            text: format!("{shared_prefix}\n\nstill streaming"),
        });
        app.turn_prompt_anchors.push(TurnPromptAnchor {
            session_id: session_id.clone(),
            turn_id: errored_turn.clone(),
            content: "start background task".into(),
            anchor_index: 0,
            prior_matching_user_count: 0,
        });
        let joined = |update: &ScrollbackUpdate| -> String {
            update
                .lines_to_insert
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>()
                .join("")
        };

        let mut tracker = ScrollbackTracker::new();
        let live = tracker.sync(&app, palette(), 80);
        assert!(
            joined(&live).contains(shared_prefix),
            "precondition: T1's completed first paragraph streamed to scrollback"
        );

        // T1 errors: the failure card commits; T1's coverage survives.
        app.sessions[0].live_reply = None;
        app.sessions[0].messages.push(Message::assistant(
            "Session Summary\n- Result: Turn failed before producing a final answer.",
        ));
        app.mark_turn_errored(&session_id, &errored_turn);
        let _ = tracker.sync(&app, palette(), 80);

        // T2 commits a fresh reply that happens to share the first paragraph.
        let later_turn = TurnId::new();
        app.sessions[0].messages.push(Message::user("run it again"));
        app.turn_prompt_anchors.push(TurnPromptAnchor {
            session_id: session_id.clone(),
            turn_id: later_turn,
            content: "run it again".into(),
            anchor_index: 3,
            prior_matching_user_count: 0,
        });
        app.sessions[0].messages.push(Message::assistant(format!(
            "{shared_prefix}\n\nSecond run, nothing else changed."
        )));
        let committed = joined(&tracker.sync(&app, palette(), 80));

        assert!(
            committed.contains(shared_prefix),
            "a later turn's same-prefix reply is a fresh answer, not the errored turn's flushed prefix: {committed:?}"
        );
        assert!(
            committed.contains("Second run, nothing else changed."),
            "the fresh reply's suffix renders with it: {committed:?}"
        );
    }

    #[test]
    fn no_new_messages_flushes_nothing() {
        let mut tracker = ScrollbackTracker::new();
        let app = state(vec![Message::user("hi"), Message::assistant("a1")]);
        let _ = tracker.sync(&app, palette(), 60);
        let again = tracker.sync(&app, palette(), 60);
        assert!(again.lines_to_insert.is_empty());
        assert!(!again.reset);
    }

    #[test]
    fn late_activity_log_archive_appends_activity_without_reflushing_messages() {
        let mut tracker = ScrollbackTracker::new();
        let mut app = state(vec![
            Message::user("build the site"),
            Message::assistant("done"),
        ]);
        let _ = tracker.sync(&app, palette(), 60);

        let session_id = app.sessions[0].id.clone();
        let turn_id = TurnId::new();
        app.turn_activity_logs.push(TurnActivityLog {
            session_id,
            turn_id: turn_id.clone(),
            request: Some("build the site".into()),
            anchor_index: Some(0),
            items: vec![
                ActivityItem::new(ActivityKind::Tool, "shell", "complete")
                    .with_turn(turn_id)
                    .with_detail("cargo test")
                    .with_success(true),
            ],
        });

        let update = tracker.sync(&app, palette(), 60);
        let text = update
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(!update.reset, "late activity is not a message rewrite");
        assert!(!text.contains("build the site") && !text.contains("done"));
        assert!(
            text.contains("Agent task completed") && text.contains("Bash($ cargo test"),
            "reflush should include archived activity log: {text:?}"
        );
    }

    #[test]
    fn late_activity_after_ten_turns_appends_only_new_activity_and_keeps_rewrite_guard() {
        let mut app = state(vec![
            Message::user("OLDPROMPT"),
            Message::assistant("OLDANSWER"),
        ]);
        let session = app.sessions[0].id.clone();
        let parent = TurnId::new();
        app.turn_activity_logs.push(TurnActivityLog {
            session_id: session.clone(),
            turn_id: parent.clone(),
            request: Some("OLDPROMPT".into()),
            anchor_index: Some(0),
            items: vec![
                ActivityItem::new(ActivityKind::Tool, "Spawn", "complete")
                    .with_turn(parent.clone())
                    .with_session(session.clone())
                    .with_tool_call("parent-call"),
            ],
        });
        let mut tracker = ScrollbackTracker::new();
        let _ = tracker.sync(&app, palette(), 100);
        for n in 0..10 {
            let turn = TurnId::new();
            app.sessions[0]
                .messages
                .push(Message::user(format!("PROMPT{n}")));
            app.sessions[0].live_reply = Some(LiveReply {
                turn_id: turn,
                text: format!("ANSWER{n}\n\n"),
            });
            let _ = tracker.sync(&app, palette(), 100);
            app.sessions[0]
                .messages
                .push(Message::assistant(format!("ANSWER{n}\n\n")));
            app.sessions[0].live_reply = None;
            let _ = tracker.sync(&app, palette(), 100);
        }
        assert!(tracker.completed_live.len() <= 8);
        app.turn_activity_logs[0].items.push(
            ActivityItem::new(ActivityKind::Progress, "LATEBACKGROUND", "completed")
                .with_turn(parent)
                .with_session(session)
                .with_tool_call("parent-call"),
        );
        let update = tracker.sync(&app, palette(), 100);
        let text = update
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(
            !update.reset,
            "same-message history cannot reset because an old activity log gained a row"
        );
        assert!(
            text.contains("LATEBACKGROUND"),
            "new activity must still render: {text}"
        );
        assert!(
            !text.contains("OLDPROMPT") && !text.contains("OLDANSWER") && !text.contains("PROMPT9"),
            "{text}"
        );
        assert!(
            tracker
                .sync(&app, palette(), 100)
                .lines_to_insert
                .is_empty()
        );
        app.sessions[0].messages[1].content = "GENUINEREWRITE".into();
        app.turn_activity_logs[0].items.push(ActivityItem::new(
            ActivityKind::Progress,
            "another late row",
            "completed",
        ));
        assert!(
            tracker.sync(&app, palette(), 100).reset,
            "real message rewrite must win over activity delta"
        );
        app.sessions[0].messages.truncate(1);
        assert!(
            tracker.sync(&app, palette(), 100).reset,
            "rollback shrink must still reset"
        );
    }

    #[test]
    fn late_activity_and_new_messages_share_a_frame_without_losing_delta_or_rewrite_guard() {
        let mut app = state(vec![
            Message::user("PREFIXPROMPT"),
            Message::assistant("PREFIXANSWER"),
        ]);
        let session = app.sessions[0].id.clone();
        let turn = TurnId::new();
        app.turn_activity_logs.push(TurnActivityLog {
            session_id: session.clone(),
            turn_id: turn.clone(),
            request: Some("PREFIXPROMPT".into()),
            anchor_index: Some(0),
            items: vec![
                ActivityItem::new(ActivityKind::Tool, "read_file", "complete")
                    .with_tool_call("original")
                    .with_turn(turn.clone()),
            ],
        });
        let mut tracker = ScrollbackTracker::new();
        let _ = tracker.sync(&app, palette(), 100);
        app.turn_activity_logs[0].items.push(
            ActivityItem::new(ActivityKind::Progress, "SAMEFRAMELATE", "completed")
                .with_canonical_activity_id("late-id")
                .with_turn(turn),
        );
        app.sessions[0]
            .messages
            .extend([Message::user("NEWPROMPT"), Message::assistant("NEWANSWER")]);
        let update = tracker.sync(&app, palette(), 100);
        let text = update
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!update.reset);
        for expected in ["SAMEFRAMELATE", "NEWPROMPT", "NEWANSWER"] {
            assert_eq!(text.matches(expected).count(), 1, "{text}");
        }
        assert!(!text.contains("PREFIXPROMPT") && !text.contains("PREFIXANSWER"));
        app.turn_activity_logs[0].items[0].duration_ms = Some(99);
        let metadata = tracker.sync(&app, palette(), 100);
        assert!(!metadata.reset);
        assert!(
            metadata.lines_to_insert.is_empty(),
            "metadata-only update does not replay finished messages/activity"
        );
        app.sessions[0].messages[0].content = "REWRITTENPREFIX".into();
        app.sessions[0]
            .messages
            .push(Message::assistant("GROWINGHISTORY"));
        assert!(
            tracker.sync(&app, palette(), 100).reset,
            "growing history cannot hide a rewritten committed prefix"
        );
    }

    #[test]
    fn archived_activity_anchor_move_on_first_answer_does_not_repeat_old_tool() {
        let mut app = state(vec![Message::user("ANCHORPROMPT")]);
        let session = app.sessions[0].id.clone();
        let turn = TurnId::new();
        app.turn_activity_logs.push(TurnActivityLog {
            session_id: session.clone(),
            turn_id: turn.clone(),
            request: Some("ANCHORPROMPT".into()),
            anchor_index: Some(0),
            items: vec![
                ActivityItem::new(ActivityKind::Tool, "read_file", "complete")
                    .with_tool_call("anchor-call")
                    .with_turn(turn.clone())
                    .with_output_preview("OLD_TOOL_ONCE"),
            ],
        });
        app.attach_turn_summary(&session, &turn, 75, 1);
        let mut tracker = ScrollbackTracker::new();
        let first = tracker.sync(&app, palette(), 100);
        let text = |update: &ScrollbackUpdate| {
            update
                .lines_to_insert
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        assert!(text(&first).contains("OLD_TOOL_ONCE"));
        assert!(text(&first).contains("Ran for"));
        app.sessions[0]
            .messages
            .push(Message::assistant("FIRSTANSWER"));
        let second = tracker.sync(&app, palette(), 100);
        assert!(!second.reset);
        assert!(text(&second).contains("FIRSTANSWER"));
        assert!(
            !text(&second).contains("OLD_TOOL_ONCE"),
            "canonical answer moves old log's render anchor but must not reemit its tool"
        );
        assert!(
            !text(&second).contains("Ran for"),
            "moving an already rendered log must not repeat its committed footer"
        );
    }

    #[test]
    fn late_summary_without_activity_is_appended_once_without_replaying_dialogue() {
        let mut app = state(vec![
            Message::user("LATEFOOTERPROMPT"),
            Message::assistant("LATEFOOTERANSWER"),
        ]);
        let session = app.sessions[0].id.clone();
        let turn = TurnId::new();
        app.turn_activity_logs.push(TurnActivityLog {
            session_id: session.clone(),
            turn_id: turn.clone(),
            request: Some("LATEFOOTERPROMPT".into()),
            anchor_index: Some(0),
            items: Vec::new(),
        });
        let text = |update: &ScrollbackUpdate| {
            update
                .lines_to_insert
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        let mut tracker = ScrollbackTracker::new();
        assert!(!text(&tracker.sync(&app, palette(), 100)).contains("Ran for"));
        app.attach_turn_summary(&session, &turn, 75, 0);
        let late = tracker.sync(&app, palette(), 100);
        assert!(!late.reset);
        assert_eq!(text(&late).matches("Ran for").count(), 1);
        assert!(!text(&late).contains("LATEFOOTERPROMPT"));
        assert!(!text(&late).contains("LATEFOOTERANSWER"));
        assert!(
            tracker
                .sync(&app, palette(), 100)
                .lines_to_insert
                .is_empty()
        );
    }

    #[test]
    fn archived_running_tool_is_not_acknowledged_before_its_terminal_output() {
        let mut app = state(vec![
            Message::user("PENDINGTOOLPROMPT"),
            Message::assistant("PENDINGTOOLANSWER"),
        ]);
        let session = app.sessions[0].id.clone();
        let turn = TurnId::new();
        app.turn_activity_logs.push(TurnActivityLog {
            session_id: session.clone(),
            turn_id: turn.clone(),
            request: Some("PENDINGTOOLPROMPT".into()),
            anchor_index: Some(0),
            items: vec![
                ActivityItem::new(ActivityKind::Tool, "read_file", "running")
                    .with_tool_call("pending-tool")
                    .with_turn(turn.clone()),
            ],
        });
        let mut tracker = ScrollbackTracker::new();
        tracker.completed_live.push(LiveTurnFinalization {
            session_id: session.0,
            turn_id: turn.0.to_string(),
            ..Default::default()
        });
        let first = tracker.sync(&app, palette(), 100);
        assert!(!first.reset);
        app.turn_activity_logs[0].items[0].status = "complete".into();
        app.turn_activity_logs[0].items[0].output_preview = Some("TERMINALTOOLOUTPUT".into());
        let completed = tracker.sync(&app, palette(), 100);
        let output = completed
            .lines_to_insert
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!completed.reset);
        assert!(output.contains("TERMINALTOOLOUTPUT"));
        assert!(!output.contains("PENDINGTOOLANSWER"));
        assert!(
            tracker
                .sync(&app, palette(), 100)
                .lines_to_insert
                .is_empty()
        );
    }

    #[test]
    fn unanchored_archived_activity_requires_exact_owner_and_retains_once_coverage() {
        let mut app = state(vec![
            Message::user("UNANCHOREDPROMPT"),
            Message::assistant("UNANCHOREDANSWER"),
        ]);
        let session = app.sessions[0].id.clone();
        let known = TurnId::new();
        let unknown = TurnId::new();
        for (turn, marker) in [(&known, "OWNEDLEGACYTOOL"), (&unknown, "UNOWNEDTOOL")] {
            app.turn_activity_logs.push(TurnActivityLog {
                session_id: session.clone(),
                turn_id: turn.clone(),
                request: None,
                anchor_index: None,
                items: vec![
                    ActivityItem::new(ActivityKind::Tool, "read_file", "complete")
                        .with_tool_call(marker)
                        .with_turn(turn.clone())
                        .with_output_preview(marker),
                ],
            });
        }
        let text = |update: &ScrollbackUpdate| {
            update
                .lines_to_insert
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        let mut tracker = ScrollbackTracker::new();
        tracker.completed_live.push(LiveTurnFinalization {
            session_id: session.0.clone(),
            turn_id: known.0.to_string(),
            ..Default::default()
        });
        let awaiting_history = std::mem::take(&mut app.sessions[0].messages);
        assert!(
            tracker
                .sync(&app, palette(), 100)
                .lines_to_insert
                .is_empty()
        );
        assert!(
            tracker.archived_activity.is_empty(),
            "a frame with no history did not render or acknowledge the late log"
        );
        app.sessions[0].messages = awaiting_history;
        let first = tracker.sync(&app, palette(), 100);
        assert!(text(&first).contains("OWNEDLEGACYTOOL"));
        assert!(!text(&first).contains("UNOWNEDTOOL"));
        assert_eq!(tracker.archived_activity.len(), 1);
        // Coverage follows the retained log, not the shorter live-reply cache.
        tracker.completed_live.clear();
        assert!(
            tracker
                .sync(&app, palette(), 100)
                .lines_to_insert
                .is_empty()
        );
        app.turn_activity_logs[0].anchor_index = Some(0);
        assert!(
            tracker
                .sync(&app, palette(), 100)
                .lines_to_insert
                .is_empty()
        );
        // Once its own anchor becomes available, the unknown log is rendered;
        // it was not falsely acknowledged while another turn had coverage.
        app.turn_activity_logs[1].anchor_index = Some(0);
        let newly_owned = tracker.sync(&app, palette(), 100);
        assert!(!newly_owned.reset);
        assert!(text(&newly_owned).contains("UNOWNEDTOOL"));
        assert!(!text(&newly_owned).contains("OWNEDLEGACYTOOL"));
        assert!(!text(&newly_owned).contains("UNANCHOREDANSWER"));
    }

    #[test]
    fn session_switch_triggers_reset_and_full_reflush() {
        let mut tracker = ScrollbackTracker::new();
        let app1 = state(vec![Message::user("hi"), Message::assistant("a1")]);
        let _ = tracker.sync(&app1, palette(), 60);

        let other = AppUiSession {
            id: SessionKey("local:other".into()),
            title: "o".into(),
            profile_id: None,
            messages: vec![Message::user("q"), Message::assistant("a")],
            tasks: Vec::new(),
            live_reply: None,
        };
        let app2 = AppState::new(vec![other], 0, "ready".into(), None, false);
        let update = tracker.sync(&app2, palette(), 60);
        assert!(update.reset, "switching sessions should reset scrollback");
        assert!(!update.lines_to_insert.is_empty());
    }

    #[test]
    fn empty_session_flushes_nothing() {
        let mut tracker = ScrollbackTracker::new();
        let app = state(vec![]);
        let update = tracker.sync(&app, palette(), 60);
        assert!(update.lines_to_insert.is_empty());
        assert!(!update.reset);
    }
}
