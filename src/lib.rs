// i18n: load `locales/*.yml` (relative to the crate root) and generate the
// `t!` macro + `rust_i18n::set_locale` / `available_locales!`. English is the
// source/fallback locale; `--lang zh` (or OCTOS_LANG=zh) switches the UI.
// New strings: add a key under `locales/en.yml` and its `zh` translation.
#[macro_use]
extern crate rust_i18n;
rust_i18n::i18n!("locales", fallback = "en");

pub mod app;
pub mod autonomy;
pub mod backend_ensure;
pub mod cli;
pub mod client_event;
pub mod clipboard;
pub mod cmd;
pub mod event_loop;
pub mod file_picker;
pub mod highlight;
pub mod history;
pub mod insert_history;
pub mod keymap;
pub mod menu;
pub mod model;
pub mod olp_mcp;
pub mod outer_duty;
pub mod profiles;
pub mod sanitize;
pub mod splash;
pub mod store;
pub mod terminal_probe;
pub mod theme;
pub mod transport;
pub mod tui_terminal;
pub mod viewport;

#[cfg(test)]
mod i18n_tests {
    /// The i18n scaffold loads both locale files and resolves keys. Uses the
    /// per-call `locale =` override (NOT `set_locale`) so it can't mutate the
    /// process-global locale and flake other tests that assume English.
    #[test]
    fn resolves_keys_in_en_and_zh() {
        assert_eq!(
            &*t!("composer.placeholder", locale = "en"),
            "Ask Octos to change code..."
        );
        assert_eq!(
            &*t!("composer.placeholder", locale = "zh"),
            "让 Octos 帮你改代码……"
        );
    }

    /// Both shipped locales are registered (guards against a misnamed locale
    /// file silently dropping a language).
    #[test]
    fn ships_en_and_zh_locales() {
        let locales = rust_i18n::available_locales!();
        assert!(locales.contains(&"en"), "missing en: {locales:?}");
        assert!(locales.contains(&"zh"), "missing zh: {locales:?}");
    }

    /// Every core slash-command description is an i18n key that resolves in
    /// both bundled locales. This guards against user-facing English literals
    /// bypassing localization in the command popup.
    #[test]
    fn core_command_descriptions_resolve_in_en_and_zh() {
        for command in crate::menu::registry::core_command_specs() {
            let key = command.description;
            assert!(
                key.starts_with("command.") && key.ends_with(".desc"),
                "/{} description must be a command.*.desc i18n key, got `{key}`",
                command.name
            );
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for /{} (`{key}`)",
                    command.name
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for /{} (`{key}`)",
                    command.name
                );
            }
        }
    }

    /// Regression set for the static English literals found by the command,
    /// menu, activity, and runtime-status localization audit.
    #[test]
    fn audited_user_facing_strings_resolve_in_en_and_zh() {
        let keys = [
            "app.activity.title",
            "app.activity.no_matches",
            "app.activity_navigator.navigator_suffix",
            "app.activity_navigator.search_active",
            "app.activity_navigator.query_label",
            "app.activity_navigator.empty_query",
            "app.activity_navigator.counts",
            "app.activity_navigator.filter_label",
            "app.activity_navigator.filter_label_compact",
            "app.activity_navigator.query_filter",
            "app.activity_navigator.results",
            "app.activity_navigator.detail",
            "app.activity_navigator.no_selected",
            "app.activity_navigator.orchestration_active",
            "app.activity_navigator.running_agents",
            "app.activity_navigator.pending_continuations",
            "app.activity_navigator.session_running",
            "app.activity_navigator.session_blocked",
            "app.activity_navigator.session_done",
            "app.activity_navigator.session_error",
            "app.activity_navigator.no_active_session",
            "app.activity_navigator.approval_required",
            "app.activity_navigator.question_pending",
            "app.activity_navigator.questions",
            "app.activity_navigator.empty_message",
            "app.activity_navigator.message_title",
            "app.activity_navigator.message_subtitle",
            "app.activity_navigator.content_heading",
            "app.activity_navigator.reasoning_heading",
            "app.activity_navigator.output_tail_heading",
            "app.activity_navigator.output_preview_heading",
            "app.activity_navigator.diff_preview_ready",
            "app.activity_navigator.diff_preview_pending",
            "app.activity_navigator.field",
            "app.activity_navigator.kind.session",
            "app.activity_navigator.kind.message",
            "app.activity_navigator.kind.orchestration",
            "app.activity_navigator.kind.task",
            "app.activity_navigator.kind.change",
            "app.activity_navigator.kind.activity",
            "app.activity_navigator.kind.approval",
            "app.activity_navigator.status.running",
            "app.activity_navigator.status.blocked",
            "app.activity_navigator.status.failed",
            "app.activity_navigator.status.done",
            "app.activity_navigator.filter.all",
            "app.activity_navigator.filter.running",
            "app.activity_navigator.filter.blocked",
            "app.activity_navigator.filter.failed",
            "app.activity_navigator.filter.done",
            "app.activity_navigator.run_state.idle",
            "app.activity_navigator.run_state.running",
            "app.activity_navigator.run_state.blocked",
            "app.activity_navigator.run_state.done",
            "app.activity_navigator.run_state.error",
            "app.activity_navigator.value.active",
            "app.activity_navigator.value.pending",
            "app.activity_navigator.value.running",
            "app.activity_navigator.value.done",
            "app.activity_navigator.value.failed",
            "app.activity_navigator.value.cancelled",
            "app.activity_navigator.value.interrupted",
            "app.activity_navigator.value.unknown",
            "app.activity_navigator.value.user",
            "app.activity_navigator.value.assistant",
            "app.activity_navigator.value.system",
            "app.activity_navigator.value.tool",
            "app.activity_navigator.value.progress",
            "app.activity_navigator.value.report",
            "app.activity_navigator.value.approval",
            "app.activity_navigator.value.warning",
            "app.activity_navigator.value.error",
            "app.activity_navigator.value.added",
            "app.activity_navigator.value.modified",
            "app.activity_navigator.value.deleted",
            "app.activity_navigator.value.renamed",
            "app.activity_navigator.label.session",
            "app.activity_navigator.label.phase",
            "app.activity_navigator.label.state",
            "app.activity_navigator.label.status",
            "app.activity_navigator.label.detail",
            "app.activity_navigator.label.tool",
            "app.activity_navigator.label.kind",
            "app.activity_navigator.label.body",
            "app.activity_navigator.label.question_id",
            "app.activity_navigator.label.message",
            "app.activity_navigator.label.role",
            "app.activity_navigator.label.tool_call",
            "app.activity_navigator.label.task",
            "app.activity_navigator.label.turn",
            "app.activity_navigator.label.file",
            "app.activity_navigator.label.type",
            "app.activity_navigator.label.operation",
            "app.activity_navigator.label.preview",
            "menu.common.no_status_supplied",
            "menu.common.local_offline",
            "menu.common.edit_as",
            "menu.common.test_named",
            "menu.common.delete_named",
            "menu.protocol.uses_method",
            "menu.protocol.requires_method",
            "menu.protocol.method_not_advertised",
            "menu.protocol.returned_no_items",
            "menu.protocol.no_cached_result",
            "menu.onboard.item.auth_send.desc",
            "menu.resume.title",
            "menu.resume.subtitle",
            "menu.resume.empty",
            "menu.resume.loading",
            "menu.resume.no_preview",
            "menu.resume.message_count",
            "menu.resume.search",
            "menu.resume.footer",
            "menu.rewind.title",
            "menu.rewind.subtitle",
            "menu.rewind.empty",
            "menu.rewind.drop_count",
            "menu.rewind.search",
            "menu.rewind.footer",
            "menu.cost.usage_totals",
            "menu.mcp.configured_servers",
            "menu.mcp.session_servers",
            "menu.mcp.refresh_first",
            "menu.model.profile_models",
            "menu.model.server_unavailable",
            "menu.skills.installed_skills",
            "menu.skills.refresh_first",
            "menu.status.requires_open_session",
            "menu.tools.configured_tools",
            "menu.tools.session_tools",
            "menu.tools.refresh_first",
            "status.peer_session_read_only",
            "status.statusline_save_not_wired",
            "status.terminal_title_save_not_wired",
            "status.resume_ambiguous",
            "status.resume_no_match",
            "status.resuming_session",
            "status.resume_loaded",
            "status.resume_parse_failed",
            "status.rewind_active_turn",
            "status.rewind_no_active_session",
            "status.rewind_wrong_session",
            "status.rewind_transcript_changed",
            "status.rewinding",
            "status.rewind_invalid_checkpoint",
            "status.rewind_checkpoint_min",
            "status.rewind_checkpoint_out_of_range",
            "status.rewind_complete",
            "status.turn_started_in",
            "status.tool_failed",
            "status.tool_completed",
            "status.warning_code_message",
            "status.router_status",
            "status.router_failover",
            "status.queue_empty",
            "status.queue_pending",
            "status.plan_updated",
            "status.tool_lifecycle",
            "status.file_attached",
            "status.turn_interrupted_reason",
            "status.background_completion_persisted",
            "status.session_event",
            "status.approval_auto_resolved",
            "status.approval_decided",
            "status.approval_cancelled",
            "status.task_output_cursor",
            "status.paste_ignored_dialog_open",
            "status.doctor_outcome.pass",
            "status.doctor_outcome.warn",
            "status.doctor_outcome.fail",
            "status.doctor_outcome.skip",
            "onboarding.validation.profile_name_required",
            "onboarding.validation.display_name_required",
            "onboarding.validation.display_name_too_long",
            "onboarding.validation.username_required",
            "onboarding.validation.username_too_long",
            "onboarding.validation.username_invalid_chars",
            "onboarding.validation.email_required",
            "onboarding.validation.email_invalid",
            "onboarding.validation.profile_collision",
            "onboarding.validation.local_profile_unsupported",
            "onboarding.validation.server_rejected_name",
            "onboarding.validation.server_rejected_username",
            "onboarding.validation.server_rejected_email",
            "onboarding.validation.server_rejected_fields",
            "onboarding.validation.local_profile_create_failed",
        ];
        for key in keys {
            let en = t!(key, locale = "en");
            let zh = t!(key, locale = "zh");
            assert_ne!(&*en, key, "missing English translation for `{key}`");
            assert_ne!(&*zh, key, "missing Chinese translation for `{key}`");
            assert!(
                !en.trim().is_empty(),
                "empty English translation for `{key}`"
            );
            assert!(
                !zh.trim().is_empty(),
                "empty Chinese translation for `{key}`"
            );
            assert_ne!(en, zh, "`{key}` should not display English in Chinese mode");
        }
    }

    /// #363/#364: the `@` file-picker menu + `!` shell-escape mode keys resolve
    /// in BOTH locales (rust-i18n echoes the key back on a miss).
    #[test]
    fn composer_escape_and_file_picker_keys_resolve_in_en_and_zh() {
        let keys = [
            "menu.file_picker.title",
            "menu.file_picker.search",
            "menu.file_picker.footer",
            "menu.file_picker.item.empty.label",
            "menu.file_picker.item.empty.desc",
            "menu.file_picker.item.truncated.label",
            "status.bang_mode_hint",
            "status.bang_mode_cancelled",
            "status.bang_cwd",
            "status.inserted_at_cursor",
            "status.file_picker_closed",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// #324: the session-switcher strings resolve in BOTH locales.
    #[test]
    fn sessions_popup_keys_resolve_in_en_and_zh() {
        let keys = [
            "command.sessions.desc",
            "menu.sessions.title",
            "menu.sessions.subtitle",
            "menu.sessions.footer",
            "menu.sessions.item.current",
            "menu.sessions.item.switch_desc",
            "menu.sessions.item.empty",
            "menu.sessions.item.empty_desc",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(&*value, key, "missing {locale} translation for {key}");
            }
        }
    }

    /// #1768: the /undo snapshot picker strings resolve in BOTH locales.
    #[test]
    fn undo_picker_keys_resolve_in_en_and_zh() {
        let keys = [
            "command.undo.desc",
            "status.undo_no_session",
            "menu.undo.title",
            "menu.undo.subtitle",
            "menu.undo.footer",
            "menu.undo.age.just_now",
            "menu.undo.item.refresh.label",
            "menu.undo.item.refresh.desc",
            "menu.undo.item.stale.label",
            "menu.undo.item.stale.desc",
            "menu.undo.item.unavailable.label",
            "menu.undo.item.unavailable.desc",
            "menu.undo.item.disabled.label",
            "menu.undo.item.disabled.desc",
            "menu.undo.item.empty.label",
            "menu.undo.item.empty.desc",
            "menu.undo.item.snap.desc",
            "menu.undo_confirm.title",
            "menu.undo_confirm.subtitle",
            "menu.undo_confirm.yes_desc",
            "menu.undo_confirm.item.empty.label",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// #395 + octos#1801 v2: the `/peer` + `/gather` command strings
    /// (registry descriptions + dispatch / kickoff / fan-in status lines)
    /// resolve in BOTH locales.
    #[test]
    fn peer_command_keys_resolve_in_en_and_zh() {
        let keys = [
            "command.peer.desc",
            "status.peer_usage",
            "status.peer_preparing",
            "status.peer_prepare_in_flight",
            "status.session_blocked_hint",
            "menu.sessions.item.blocked_reason",
            "status.peer_opening",
            "status.peer_started",
            "status.peer_switched",
            "status.peer_fleet_opening",
            "status.peer_staged_by_agent",
            "status.peer_staged_known",
            "command.gather.desc",
            "status.gather_requesting",
            "status.gather_no_peers",
            "status.gather_submitted",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// octoscode#532: the awaiting-fleet strings (status-bar segment, softened
    /// budget chip, dock landing progress, waiting-on-fleet summary card)
    /// resolve in BOTH locales.
    #[test]
    fn awaiting_fleet_keys_resolve_in_en_and_zh() {
        let keys = [
            "app.statusbar.awaiting_fleet",
            "app.autonomy.status_budget_limited_fleet",
            "app.hint.peer_dock_landed",
            "status.turn_waiting_on_fleet",
            "status.turn_missing_answer",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// octos#1807: the `turn/steer` status string resolves in BOTH locales
    /// (rust-i18n echoes the key back on a miss).
    #[test]
    fn turn_steer_keys_resolve_in_en_and_zh() {
        let keys = ["status.steered_into_turn"];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// PR384 fixes: the research-lane wizard strings (lane-aware save row,
    /// lane-key picker, saved status/target labels) resolve in BOTH locales.
    #[test]
    fn research_lane_wizard_keys_resolve_in_en_and_zh() {
        let keys = [
            "onboarding.provider.research_lane",
            "onboarding.provider.save_research_lane",
            "menu.onboard.item.save_research_lane.desc",
            "menu.research_lane_key.title",
            "menu.research_lane_key.subtitle",
            "menu.research_lane_key.occupied",
            "menu.research_lane_key.vacant",
            "menu.research_lane_key.item.cheap.desc",
            "menu.research_lane_key.item.strong.desc",
            "status.research_lane_saved",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// #362: the side-by-side diff view toggle strings (footer hints + status
    /// feedback) resolve in BOTH locales.
    #[test]
    fn diff_view_toggle_keys_resolve_in_en_and_zh() {
        let keys = [
            "app.diff.toggle_side_by_side_hint",
            "app.diff.toggle_unified_hint",
            "app.diff.side_by_side_too_narrow",
            "status.diff_view_side_by_side",
            "status.diff_view_unified",
            "status.diff_view_too_narrow",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }

    /// Recovery advice must name flags that ACTUALLY EXIST.
    ///
    /// This string shipped for a long time as
    /// "Start the TUI with `octos tui --target <stdio:...|ws://...>`", which is
    /// wrong twice over: `octos` has no `tui` subcommand, and there is no
    /// `--target` flag. It is the advice shown when the TUI cannot find a
    /// transport — i.e. to a user who is already stuck — so following it landed
    /// them on "unrecognized subcommand".
    ///
    /// Assert against the real clap definition rather than a hardcoded list, so
    /// this keeps working as flags are added or renamed.
    #[test]
    fn doctor_recovery_advice_names_only_real_cli_flags() {
        let command = crate::cli::cli_command();
        let real: std::collections::HashSet<String> = command
            .get_arguments()
            .filter_map(|arg| arg.get_long())
            .map(|long| format!("--{long}"))
            .collect();
        assert!(
            real.contains("--endpoint") && real.contains("--stdio-command"),
            "sanity: the flags this advice names must be discoverable from clap"
        );

        for locale in ["en", "zh"] {
            let advice = t!("status.doctor_no_transport_recovery", locale = locale);

            // Only flags addressed to THIS binary count. A quoted value like
            // `--stdio-command "octos serve --stdio"` carries the SERVER's
            // flags, which clap here knows nothing about, so drop quoted spans
            // before scanning.
            let mut outside = String::new();
            let mut in_quotes = false;
            for ch in advice.chars() {
                if ch == '"' {
                    in_quotes = !in_quotes;
                    continue;
                }
                if !in_quotes {
                    outside.push(ch);
                }
            }

            // Every `--flag` token the advice mentions must be a real long flag.
            for token in outside.split_whitespace() {
                let flag: String = token
                    .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                    .to_string();
                if !flag.starts_with("--") || flag.len() <= 2 {
                    continue;
                }
                assert!(
                    real.contains(&flag),
                    "{locale} recovery advice names `{flag}`, which is not a \
                     real CLI flag. Advice: {advice}"
                );
            }

            // `octos` is the SERVER binary and has no `tui` subcommand; the TUI
            // is its own binary. Pin the specific wrong invocation that shipped.
            assert!(
                !advice.contains("octos tui"),
                "{locale} recovery advice tells the user to run `octos tui`, \
                 which is not a subcommand. Advice: {advice}"
            );
        }
    }

    /// UX2 A.3/B.2: the new onboarding teaching-panel + workspace-step keys
    /// resolve in BOTH locales (rust-i18n echoes the key on a miss, so a typo or
    /// a missing `zh` translation would leave the dotted key in the output).
    #[test]
    fn onboarding_ux2_keys_resolve_in_en_and_zh() {
        let keys = [
            "onboarding.language.title",
            "onboarding.language.description",
            "onboarding.wizard.explain_title",
            "onboarding.wizard.explain.language",
            "onboarding.wizard.explain.profile",
            "onboarding.wizard.explain.provider",
            "onboarding.wizard.explain.connect",
            "onboarding.wizard.explain.save",
            "onboarding.wizard.explain.workspace",
            "onboarding.wizard.explain.activate",
            "onboarding.wizard.workspace_title",
            "onboarding.wizard.workspace_open_label",
            "onboarding.wizard.workspace_open_description",
            "onboarding.wizard.workspace_locked_reason",
            "onboarding.preview.provider.configured_title",
            "onboarding.preview.workspace.staged_title",
            "menu.lang.item.en.label",
            "menu.lang.item.zh.label",
            // Phase 2 nameable-profiles + Phase 3 startup picker keys.
            "onboarding.field.profile_name",
            "onboarding.field.profile_name_desc",
            "onboarding.value_suggested",
            "menu.profile_picker.title",
            "menu.profile_picker.subtitle",
            "menu.profile_picker.item.attach.desc",
            "menu.profile_picker.item.create.label",
            "menu.profile_picker.item.create.desc",
        ];
        for key in keys {
            for locale in ["en", "zh"] {
                let value = t!(key, locale = locale);
                assert_ne!(
                    &*value, key,
                    "missing {locale} translation for `{key}` (got the raw key back)"
                );
                assert!(
                    !value.trim().is_empty(),
                    "empty {locale} translation for `{key}`"
                );
            }
        }
    }
}
