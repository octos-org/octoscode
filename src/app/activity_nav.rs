//! `activity_nav` — extracted from `app.rs` (#365 step 2). Items keep their
//! original names; `app.rs` glob-re-exports them so every call site is
//! unchanged. `use super::*` reaches the app module's remaining items.
use super::*;

pub(super) fn activity_navigator_kind_label(kind: ActivityNavigatorRowKind) -> String {
    let key = match kind {
        ActivityNavigatorRowKind::Session => "app.activity_navigator.kind.session",
        ActivityNavigatorRowKind::Message => "app.activity_navigator.kind.message",
        ActivityNavigatorRowKind::Orchestration => "app.activity_navigator.kind.orchestration",
        ActivityNavigatorRowKind::Task => "app.activity_navigator.kind.task",
        ActivityNavigatorRowKind::FileChange => "app.activity_navigator.kind.change",
        ActivityNavigatorRowKind::Activity => "app.activity_navigator.kind.activity",
        ActivityNavigatorRowKind::Approval => "app.activity_navigator.kind.approval",
    };
    t!(key).into_owned()
}

pub(super) fn activity_navigator_status_label(status: ActivityNavigatorStatus) -> String {
    let key = match status {
        ActivityNavigatorStatus::Running => "app.activity_navigator.status.running",
        ActivityNavigatorStatus::Blocked => "app.activity_navigator.status.blocked",
        ActivityNavigatorStatus::Failed => "app.activity_navigator.status.failed",
        ActivityNavigatorStatus::Done => "app.activity_navigator.status.done",
    };
    t!(key).into_owned()
}

pub(super) fn activity_navigator_filter_label(filter: ActivityNavigatorFilter) -> String {
    let key = match filter {
        ActivityNavigatorFilter::All => "app.activity_navigator.filter.all",
        ActivityNavigatorFilter::Running => "app.activity_navigator.filter.running",
        ActivityNavigatorFilter::Blocked => "app.activity_navigator.filter.blocked",
        ActivityNavigatorFilter::Failed => "app.activity_navigator.filter.failed",
        ActivityNavigatorFilter::Done => "app.activity_navigator.filter.done",
    };
    t!(key).into_owned()
}

fn activity_navigator_run_state_label(state: &SessionRunState) -> String {
    let key = match state {
        SessionRunState::Idle => "app.activity_navigator.run_state.idle",
        SessionRunState::InProgress => "app.activity_navigator.run_state.running",
        SessionRunState::Blocked { .. } => "app.activity_navigator.run_state.blocked",
        SessionRunState::Success => "app.activity_navigator.run_state.done",
        SessionRunState::Error { .. } => "app.activity_navigator.run_state.error",
    };
    t!(key).into_owned()
}

fn activity_navigator_activity_kind_label(kind: ActivityKind) -> String {
    let key = match kind {
        ActivityKind::Tool => "app.activity_navigator.value.tool",
        ActivityKind::Progress => "app.activity_navigator.value.progress",
        ActivityKind::Report => "app.activity_navigator.value.report",
        ActivityKind::Approval => "app.activity_navigator.value.approval",
        ActivityKind::Warning => "app.activity_navigator.value.warning",
        ActivityKind::Error => "app.activity_navigator.value.error",
    };
    t!(key).into_owned()
}

fn activity_navigator_value_label(value: &str) -> String {
    let locale = rust_i18n::locale();
    activity_navigator_value_label_for_locale(value, locale.as_ref())
}

pub(super) fn activity_navigator_value_label_for_locale(value: &str, locale: &str) -> String {
    // English is also the protocol/source language. Preserve its exact value
    // so localization never canonicalizes user-visible wire data (for example
    // changing the existing `modify` operation to `modified`).
    if !locale.starts_with("zh") {
        return value.to_string();
    }
    let key = match value.to_ascii_lowercase().as_str() {
        "active" => "app.activity_navigator.value.active",
        "pending" => "app.activity_navigator.value.pending",
        "running" => "app.activity_navigator.value.running",
        "completed" | "complete" | "done" => "app.activity_navigator.value.done",
        "failed" => "app.activity_navigator.value.failed",
        "cancelled" | "canceled" => "app.activity_navigator.value.cancelled",
        "interrupted" => "app.activity_navigator.value.interrupted",
        "unknown" => "app.activity_navigator.value.unknown",
        "user" => "app.activity_navigator.value.user",
        "assistant" => "app.activity_navigator.value.assistant",
        "system" => "app.activity_navigator.value.system",
        "tool" => "app.activity_navigator.value.tool",
        "add" | "added" | "create" | "created" => "app.activity_navigator.value.added",
        "modify" | "modified" | "update" | "updated" => "app.activity_navigator.value.modified",
        "delete" | "deleted" | "remove" | "removed" => "app.activity_navigator.value.deleted",
        "rename" | "renamed" => "app.activity_navigator.value.renamed",
        _ => return value.to_string(),
    };
    t!(key, locale = "zh").into_owned()
}

fn activity_navigator_field(key: &str, value: impl std::fmt::Display) -> String {
    t!(
        "app.activity_navigator.field",
        label = t!(key),
        value = value
    )
    .into_owned()
}

pub(super) fn activity_finalization_key(item: &ActivityItem, ordinal: usize) -> String {
    if let Some(identity) = item.stable_activity_identity() {
        return identity;
    }
    if let Some(turn_id) = item.turn_id.as_ref() {
        return format!(
            "turn:{}:{ordinal}:{}:{}",
            turn_id.0,
            item.kind.label(),
            item.title
        );
    }
    format!("activity:{ordinal}:{}:{}", item.kind.label(), item.title)
}

pub fn activity_navigator_model(app: &AppState) -> ActivityNavigatorModel {
    let mut rows = activity_navigator_all_rows(app);
    let query = app.activity_navigator.query.trim().to_ascii_lowercase();
    if !query.is_empty() {
        rows.retain(|row| row.search_text.contains(&query));
    }
    let counts = activity_navigator_counts(&rows);
    rows.retain(|row| activity_navigator_filter_matches(app.activity_navigator.filter, row.status));
    let selected = app
        .activity_navigator
        .selected
        .min(rows.len().saturating_sub(1));

    ActivityNavigatorModel {
        rows,
        counts,
        selected,
        query: app.activity_navigator.query.clone(),
        filter: app.activity_navigator.filter,
        search_active: app.activity_navigator.search_active,
    }
}

pub(super) fn activity_navigator_filter_matches(
    filter: ActivityNavigatorFilter,
    status: ActivityNavigatorStatus,
) -> bool {
    match filter {
        ActivityNavigatorFilter::All => true,
        ActivityNavigatorFilter::Running => status == ActivityNavigatorStatus::Running,
        ActivityNavigatorFilter::Blocked => status == ActivityNavigatorStatus::Blocked,
        ActivityNavigatorFilter::Failed => status == ActivityNavigatorStatus::Failed,
        ActivityNavigatorFilter::Done => status == ActivityNavigatorStatus::Done,
    }
}

pub(super) fn activity_navigator_counts(rows: &[ActivityNavigatorRow]) -> ActivityNavigatorCounts {
    let mut counts = ActivityNavigatorCounts {
        all: rows.len(),
        ..ActivityNavigatorCounts::default()
    };
    for row in rows {
        match row.status {
            ActivityNavigatorStatus::Running => counts.running += 1,
            ActivityNavigatorStatus::Blocked => counts.blocked += 1,
            ActivityNavigatorStatus::Failed => counts.failed += 1,
            ActivityNavigatorStatus::Done => counts.done += 1,
        }
        if row.kind == ActivityNavigatorRowKind::FileChange {
            counts.changes += 1;
        }
    }
    counts
}

pub(super) fn activity_navigator_all_rows(app: &AppState) -> Vec<ActivityNavigatorRow> {
    let mut rows = Vec::new();
    if let Some(row) = activity_navigator_run_state_row(app) {
        rows.push(row);
    }
    if let Some(row) = activity_navigator_approval_row(app) {
        rows.push(row);
    }
    if let Some(row) = activity_navigator_question_row(app) {
        rows.push(row);
    }

    for session_idx in activity_navigator_session_order(app) {
        let Some(session) = app.sessions.get(session_idx) else {
            continue;
        };
        if let Some(orchestration) = app.orchestration.get(&session.id)
            && orchestration.active
        {
            rows.push(activity_navigator_row(
                ActivityNavigatorRowKind::Orchestration,
                ActivityNavigatorStatus::Running,
                session.title.clone(),
                t!("app.activity_navigator.orchestration_active").into_owned(),
                vec![
                    activity_navigator_field("app.activity_navigator.label.session", &session.id.0),
                    activity_navigator_field(
                        "app.activity_navigator.label.phase",
                        orchestration.phase.clone().unwrap_or_else(|| {
                            t!("app.activity_navigator.value.active").into_owned()
                        }),
                    ),
                    t!(
                        "app.activity_navigator.running_agents",
                        count = orchestration.running_agents
                    )
                    .into_owned(),
                    t!(
                        "app.activity_navigator.pending_continuations",
                        count = orchestration.pending_continuations
                    )
                    .into_owned(),
                ],
                ActivityNavigatorRowLinks {
                    session_id: Some(session.id.clone()),
                    ..ActivityNavigatorRowLinks::default()
                },
            ));
        }
        for task in &session.tasks {
            rows.push(activity_navigator_task_row(session, task));
        }
        rows.extend(
            app.activity
                .iter()
                .filter(|item| activity_belongs_to_session(app, item, &session.id))
                .map(|item| activity_navigator_activity_row(session, item, None)),
        );
        for log in app
            .turn_activity_logs
            .iter()
            .filter(|log| log.session_id == session.id)
        {
            for item in &log.items {
                rows.push(activity_navigator_activity_row(
                    session,
                    item,
                    Some(log.turn_id.0.to_string()),
                ));
            }
        }
        rows.extend(
            session
                .messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message.role.as_str() != "system")
                .map(|(idx, message)| activity_navigator_message_row(session, idx, message)),
        );
    }

    rows
}

pub(super) fn activity_navigator_session_order(app: &AppState) -> Vec<usize> {
    let mut order = Vec::with_capacity(app.sessions.len());
    if app.selected_session < app.sessions.len() {
        order.push(app.selected_session);
    }
    order.extend((0..app.sessions.len()).filter(|idx| *idx != app.selected_session));
    order
}

pub(super) fn activity_navigator_run_state_row(app: &AppState) -> Option<ActivityNavigatorRow> {
    let (status, title) = match &app.run_state {
        SessionRunState::Idle => return None,
        SessionRunState::InProgress => (
            ActivityNavigatorStatus::Running,
            t!("app.activity_navigator.session_running"),
        ),
        SessionRunState::Blocked { .. } => (
            ActivityNavigatorStatus::Blocked,
            t!("app.activity_navigator.session_blocked"),
        ),
        SessionRunState::Success => (
            ActivityNavigatorStatus::Done,
            t!("app.activity_navigator.session_done"),
        ),
        SessionRunState::Error { .. } => (
            ActivityNavigatorStatus::Failed,
            t!("app.activity_navigator.session_error"),
        ),
    };
    let session = app.active_session();
    let detail = app.run_state.detail().unwrap_or(app.status.as_str());
    Some(activity_navigator_row(
        ActivityNavigatorRowKind::Session,
        status,
        title.into_owned(),
        session
            .map(|session| session.title.clone())
            .unwrap_or_else(|| t!("app.activity_navigator.no_active_session").into_owned()),
        vec![
            activity_navigator_field(
                "app.activity_navigator.label.state",
                activity_navigator_run_state_label(&app.run_state),
            ),
            activity_navigator_field("app.activity_navigator.label.status", &app.status),
            activity_navigator_field("app.activity_navigator.label.detail", detail),
        ],
        ActivityNavigatorRowLinks {
            session_id: session.map(|session| session.id.clone()),
            ..ActivityNavigatorRowLinks::default()
        },
    ))
}

pub(super) fn activity_navigator_approval_row(app: &AppState) -> Option<ActivityNavigatorRow> {
    let approval = app.approval.as_ref().filter(|approval| approval.visible)?;
    let session = app.active_session();
    Some(activity_navigator_row(
        ActivityNavigatorRowKind::Approval,
        ActivityNavigatorStatus::Blocked,
        approval.title.clone(),
        t!("app.activity_navigator.approval_required").into_owned(),
        vec![
            activity_navigator_field("app.activity_navigator.label.tool", &approval.tool_name),
            activity_navigator_field(
                "app.activity_navigator.label.kind",
                approval.approval_kind.as_deref().unwrap_or("unknown"),
            ),
            activity_navigator_field("app.activity_navigator.label.body", &approval.body),
        ],
        ActivityNavigatorRowLinks {
            session_id: session.map(|session| session.id.clone()),
            ..ActivityNavigatorRowLinks::default()
        },
    ))
}

pub(super) fn activity_navigator_question_row(app: &AppState) -> Option<ActivityNavigatorRow> {
    let question = app
        .user_question
        .as_ref()
        .filter(|question| question.visible)?;
    let session = app.active_session();
    Some(activity_navigator_row(
        ActivityNavigatorRowKind::Approval,
        ActivityNavigatorStatus::Blocked,
        question.title.clone(),
        t!("app.activity_navigator.question_pending").into_owned(),
        vec![
            activity_navigator_field(
                "app.activity_navigator.label.question_id",
                question.question_id.0,
            ),
            t!(
                "app.activity_navigator.questions",
                count = question.questions.len()
            )
            .into_owned(),
        ],
        ActivityNavigatorRowLinks {
            session_id: session.map(|session| session.id.clone()),
            ..ActivityNavigatorRowLinks::default()
        },
    ))
}

pub(super) fn activity_navigator_message_row(
    session: &SessionView,
    idx: usize,
    message: &Message,
) -> ActivityNavigatorRow {
    let role = message.role.as_str();
    let role_label = activity_navigator_value_label(role);
    let content = message.content.trim();
    let title = if content.is_empty() {
        t!("app.activity_navigator.empty_message", role = &role_label).into_owned()
    } else {
        t!(
            "app.activity_navigator.message_title",
            role = &role_label,
            preview = truncate_terminal_line(&content.replace('\n', " "), 80)
        )
        .into_owned()
    };
    let mut detail = vec![
        activity_navigator_field("app.activity_navigator.label.session", &session.id.0),
        activity_navigator_field("app.activity_navigator.label.message", idx + 1),
        activity_navigator_field("app.activity_navigator.label.role", &role_label),
    ];
    if !content.is_empty() {
        detail.push(t!("app.activity_navigator.content_heading").into_owned());
        detail.extend(content.lines().take(10).map(|line| format!("  {line}")));
    }
    if let Some(reasoning) = message.reasoning_content.as_deref() {
        detail.push(t!("app.activity_navigator.reasoning_heading").into_owned());
        detail.extend(reasoning.lines().take(6).map(|line| format!("  {line}")));
    }
    if let Some(tool_call_id) = message.tool_call_id.as_deref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.tool_call",
            tool_call_id,
        ));
    }

    activity_navigator_row(
        ActivityNavigatorRowKind::Message,
        ActivityNavigatorStatus::Done,
        title,
        t!(
            "app.activity_navigator.message_subtitle",
            session = &session.title,
            number = idx + 1
        )
        .into_owned(),
        detail,
        ActivityNavigatorRowLinks {
            session_id: Some(session.id.clone()),
            ..ActivityNavigatorRowLinks::default()
        },
    )
}

pub(super) fn activity_navigator_task_row(
    session: &SessionView,
    task: &TaskView,
) -> ActivityNavigatorRow {
    let status = match task.state {
        TaskRuntimeState::Pending | TaskRuntimeState::Running => ActivityNavigatorStatus::Running,
        TaskRuntimeState::Completed => ActivityNavigatorStatus::Done,
        TaskRuntimeState::Failed | TaskRuntimeState::Cancelled => ActivityNavigatorStatus::Failed,
    };
    let mut detail = vec![
        activity_navigator_field("app.activity_navigator.label.session", &session.id.0),
        activity_navigator_field("app.activity_navigator.label.task", task.id.0),
        activity_navigator_field(
            "app.activity_navigator.label.state",
            activity_navigator_value_label(task_state_label(task.state)),
        ),
    ];
    if let Some(runtime_detail) = task.runtime_detail.as_ref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.detail",
            runtime_detail,
        ));
    }
    if !task.output_tail.trim().is_empty() {
        detail.push(t!("app.activity_navigator.output_tail_heading").into_owned());
        detail.extend(
            task.output_tail
                .lines()
                .take(8)
                .map(|line| format!("  {line}")),
        );
    }

    activity_navigator_row(
        ActivityNavigatorRowKind::Task,
        status,
        task.title.clone(),
        format!(
            "{} · {}",
            session.title,
            activity_navigator_value_label(task_state_label(task.state))
        ),
        detail,
        ActivityNavigatorRowLinks {
            session_id: Some(session.id.clone()),
            task_id: Some(task.id.clone()),
            turn_id: task.turn_id.as_ref().map(|turn| turn.0.to_string()),
        },
    )
}

pub(super) fn activity_navigator_activity_row(
    session: &SessionView,
    item: &ActivityItem,
    archived_turn_id: Option<String>,
) -> ActivityNavigatorRow {
    if let Some(mutation) = FileMutationActivity::from_item(item) {
        return activity_navigator_file_change_row(session, item, mutation, archived_turn_id);
    }

    let status = activity_navigator_activity_status(item);
    let turn_id = archived_turn_id.or_else(|| item.turn_id.as_ref().map(|turn| turn.0.to_string()));
    let mut detail = vec![
        activity_navigator_field("app.activity_navigator.label.session", &session.id.0),
        activity_navigator_field(
            "app.activity_navigator.label.kind",
            activity_navigator_activity_kind_label(item.kind),
        ),
        activity_navigator_field("app.activity_navigator.label.status", &item.status),
    ];
    if let Some(turn_id) = turn_id.as_ref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.turn",
            turn_id,
        ));
    }
    if let Some(tool_call_id) = item.tool_call_id.as_ref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.tool_call",
            tool_call_id,
        ));
    }
    if let Some(item_detail) = item.detail.as_ref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.detail",
            item_detail,
        ));
    }
    if let Some(output) = item
        .output_preview
        .as_ref()
        .filter(|output| !output.is_empty())
    {
        detail.push(t!("app.activity_navigator.output_preview_heading").into_owned());
        detail.extend(output.lines().take(8).map(|line| format!("  {line}")));
    }

    activity_navigator_row(
        ActivityNavigatorRowKind::Activity,
        status,
        item.title.clone(),
        format!("{} · {}", session.title, item.status),
        detail,
        ActivityNavigatorRowLinks {
            session_id: Some(session.id.clone()),
            task_id: None,
            turn_id,
        },
    )
}

pub(super) fn activity_navigator_file_change_row(
    session: &SessionView,
    item: &ActivityItem,
    mutation: FileMutationActivity,
    archived_turn_id: Option<String>,
) -> ActivityNavigatorRow {
    let status = activity_navigator_activity_status(item);
    let turn_id = archived_turn_id.or_else(|| item.turn_id.as_ref().map(|turn| turn.0.to_string()));
    let badge = diff_file_type_badge(&mutation.path);
    let preview = if mutation.preview_ready {
        t!("app.activity_navigator.diff_preview_ready")
    } else {
        t!("app.activity_navigator.diff_preview_pending")
    };
    let mut detail = vec![
        activity_navigator_field("app.activity_navigator.label.session", &session.id.0),
        activity_navigator_field("app.activity_navigator.label.file", &mutation.path),
        activity_navigator_field("app.activity_navigator.label.type", badge),
        activity_navigator_field(
            "app.activity_navigator.label.operation",
            activity_navigator_value_label(&mutation.operation),
        ),
        activity_navigator_field("app.activity_navigator.label.preview", &preview),
        activity_navigator_field("app.activity_navigator.label.status", &item.status),
    ];
    if let Some(turn_id) = turn_id.as_ref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.turn",
            turn_id,
        ));
    }
    if let Some(item_detail) = item.detail.as_ref() {
        detail.push(activity_navigator_field(
            "app.activity_navigator.label.detail",
            item_detail,
        ));
    }

    activity_navigator_row(
        ActivityNavigatorRowKind::FileChange,
        status,
        format!(
            "{badge} {} {}",
            activity_navigator_value_label(&mutation.operation),
            compact_file_path(&mutation.path)
        ),
        format!(
            "{badge} · {} · {preview}",
            activity_navigator_value_label(&mutation.operation)
        ),
        detail,
        ActivityNavigatorRowLinks {
            session_id: Some(session.id.clone()),
            task_id: None,
            turn_id,
        },
    )
}

pub(super) fn activity_navigator_activity_status(item: &ActivityItem) -> ActivityNavigatorStatus {
    let status = item.status.to_ascii_lowercase();
    if item.kind == ActivityKind::Approval {
        ActivityNavigatorStatus::Blocked
    } else if item.kind == ActivityKind::Error
        || item.success == Some(false)
        || matches!(
            status.as_str(),
            "failed" | "error" | "cancelled" | "canceled"
        )
    {
        ActivityNavigatorStatus::Failed
    } else if crate::model::activity_status_is_running(&item.status) {
        ActivityNavigatorStatus::Running
    } else {
        ActivityNavigatorStatus::Done
    }
}

pub(super) fn activity_belongs_to_session(
    app: &AppState,
    item: &ActivityItem,
    session_id: &SessionKey,
) -> bool {
    if item.session_id.as_ref() == Some(session_id) {
        return true;
    }
    if let Some(turn_id) = item.turn_id.as_ref() {
        return app
            .turn_activity_logs
            .iter()
            .any(|log| &log.session_id == session_id && log.turn_id == *turn_id)
            || app
                .active_session()
                .is_some_and(|session| &session.id == session_id);
    }
    app.active_session()
        .is_some_and(|session| &session.id == session_id)
}

pub(super) fn activity_navigator_row(
    kind: ActivityNavigatorRowKind,
    status: ActivityNavigatorStatus,
    title: String,
    subtitle: String,
    detail_lines: Vec<String>,
    links: ActivityNavigatorRowLinks,
) -> ActivityNavigatorRow {
    let mut search_text = format!("{} {} {} {}", kind.label(), status.label(), title, subtitle);
    for detail in &detail_lines {
        search_text.push(' ');
        search_text.push_str(detail);
    }
    if let Some(session_id) = links.session_id.as_ref() {
        search_text.push(' ');
        search_text.push_str(&session_id.0);
    }
    if let Some(task_id) = links.task_id.as_ref() {
        search_text.push(' ');
        search_text.push_str(&task_id.0.to_string());
    }
    if let Some(turn_id) = links.turn_id.as_ref() {
        search_text.push(' ');
        search_text.push_str(turn_id);
    }
    search_text = search_text.to_ascii_lowercase();

    ActivityNavigatorRow {
        kind,
        status,
        title,
        subtitle,
        detail_lines,
        session_id: links.session_id,
        task_id: links.task_id,
        turn_id: links.turn_id,
        search_text: search_text.to_ascii_lowercase(),
    }
}

pub fn activity_navigator_areas(area: Rect) -> ActivityNavigatorAreas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(vertical[1]);

    ActivityNavigatorAreas {
        toolbar: vertical[0],
        list: body[0],
        detail: body[1],
        hint: vertical[2],
    }
}

pub(super) fn activity_log_render_index(session: &SessionView, anchor_index: usize) -> usize {
    session
        .messages
        .iter()
        .enumerate()
        .skip(anchor_index.saturating_add(1))
        .take_while(|(_, message)| message.role.as_str() != "user")
        .find(|(_, message)| message.role.as_str() == "assistant")
        .map(|(idx, _)| idx)
        .unwrap_or(anchor_index)
}

pub(super) fn activity_status_icon(item: &ActivityItem, palette: Palette) -> (&'static str, Style) {
    if is_running_activity(item) {
        // Animate the marker for in-progress rows (galaxy spinner) so a row
        // like "Background work started for run_pipeline" visibly reads as
        // still-running rather than a static dot. Uses the shared
        // process-clock spinner (not terminal SGR blink, which is unreliable /
        // distracting and inconsistently supported).
        (spinner_frame(), palette.selected())
    } else if activity_is_failed(item) {
        ("✗", Style::default().fg(palette.danger))
    } else if activity_is_completed(item) {
        ("✓", Style::default().fg(palette.success))
    } else {
        ("•", palette.muted())
    }
}

pub(super) fn activity_is_completed(item: &ActivityItem) -> bool {
    matches!(item.success, Some(true))
        || matches!(
            item.status.as_str(),
            "complete" | "completed" | "done" | "success"
        )
}

pub(super) fn activity_is_failed(item: &ActivityItem) -> bool {
    matches!(item.success, Some(false)) || matches!(item.status.as_str(), "failed" | "error")
}
