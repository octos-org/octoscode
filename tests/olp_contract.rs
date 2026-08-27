//! Contract tests for OLP v1 (`specs/task-r-olp-proto-v1.spec.md`, REQ-OLP-PROTO).
//!
//! These are pure documentation contracts: they pin the ACK grammar, the
//! lane-template TOML block, the result.md schema field list, and the
//! protocol-version references across `docs/OUTER_LOOP_PROTOCOL.md`,
//! `docs/OUTER_LOOP_REVIEW.md`, and `AGENTS.md`. No octos dependency.
//!
//! Historical ACK lines predate the v1 grammar and are NOT rewritten: the
//! exemption is an exact-content whitelist (`LEGACY_ACK_WHITELIST`) of the 20
//! legacy `ACK:` lines frozen at v1 adoption (2026-08-24, bounded by the
//! 2026-08-25 freeze announcement). Any NEW `ACK:` line fails the contract —
//! the whitelist, not a prefix rule, is the boundary.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// v1 ACK grammar: `ACK(done|wontdo|blocked): <non-empty explanation>`.
///
/// The grammar matches a single line. The explanation must be non-empty
/// (whitespace-only does not count).
fn ack_line_matches_v1(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("ACK(") else {
        return false;
    };
    let Some(paren_end) = rest.find(')') else {
        return false;
    };
    let status = &rest[..paren_end];
    if !matches!(status, "done" | "wontdo" | "blocked") {
        return false;
    }
    let Some(explanation) = rest[paren_end + 1..].strip_prefix(':') else {
        return false;
    };
    !explanation.trim().is_empty()
}

/// Extract every candidate ACK line from the blackboard: lines whose trimmed
/// form starts with `ACK`. This includes both legacy `ACK:` lines and the
/// v1 `ACK(status):` form.
fn blackboard_ack_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.trim_start().starts_with("ACK"))
        .map(|line| line.trim().to_string())
        .collect()
}

/// Legacy `ACK:` lines exempt from the v1 grammar, frozen at v1 adoption
/// (2026-08-24; the 2026-08-25 blackboard freeze announcement draws the same
/// boundary — legacy stock is never rewritten, but NEW legacy-form lines are
/// NOT exempt). The exemption is an exact line-content whitelist of the 20
/// `ACK:` lines present in the frozen `docs/OUTER_LOOP_REVIEW.md` snapshot:
/// any `ACK:` line NOT on this list fails the contract. Matching is exact
/// (after trimming), never prefix — a prefix match would re-open the hole
/// where any future `ACK:` line passes unchecked.
const LEGACY_ACK_WHITELIST: &[&str] = &[
    "ACK: 已完成(commit 5551e67)。`play_inner()` 用 `Palette::for_theme(*theme).accent` 动态取色,经 `color_to_sgr()` 转义为 SGR 序列——单一事实来源是 `theme.rs`,无第二张色表。",
    "ACK: 已完成(commit c39550e)。`play_inner()` 在算 `final_color` 时检查 `NO_COLOR`——如果设了,`final_color` 为空串(无色),和 ttfx 的 `config.no_color` 一致。",
    "ACK: 知晓。外环已代修的部分(92128bd 测试签名跟进、c72f606 rustfmt、122f9e1 items-after-tests)已确认。FINAL_VERIFICATION.md 将在本 ACK 后删除。",
    "ACK: (a) 协议版本号: olp/v0。(b) 黑板上编号最小的、尚无 ACK 的条目编号: 第 1 条(现已补 ACK)。",
    "ACK: 全部完成。(1) 第 1、2 条 ACK 已补(theme-aware accent 在 5551e67,NO_COLOR 在 c39550e);(2) 第 3 条 ACK 已确认外环代修部分(92128bd、c72f606、122f9e1);(3) FINAL_VERIFICATION.md 已删除;(4) goal_02 已转为 complete。",
    "ACK: 知晓。实施优化时按场景分流——常规启动(已有 profile/会话)先画帧异步握手,first-launch(探测不到本地 profile)保留等待。为两种场景各写一个契约测试。",
    "ACK: 已过时——两个 handoff 实际已完成并提交,无需重派:(1) `implement-startup-optimization` 完成启动优化(commit e939fae + 0f2c863 rustfmt),由 `verify-startup-optimization` 验证(2/2 新契约测试、146/146 event_loop 测试、1922/1922 lib 测试通过);(2) `verify-pager-scroll-consistency` 完成 pager 验证(11/11 契约测试通过,报告写入 docs/PAGER_SCROLL_VERIFY_REPORT.md),pager 改动已提交(commit 7c26e07)。两个 peer 的 result 均在黑板上(peer_gather 可读),goal_02/03/04 均已正确标记 complete。第 8 条的整改已另派 peer `fix-pager-scroll-clamp` 执行。",
    "ACK: 已按 `agent_view_scroll_max` 先例完成整改(peer `fix-pager-scroll-clamp`)。",
    "ACK: 已完成(peer `implement-terminal-resilience-v2`)——(1) **问题 2(paste 超时兜底)**:从 stash 拆出复用,落地 `UNBRACKETED_PASTE_MAX_WINDOW`(200ms 上限窗口)+ `unbracketed_paste_started` 字段 + `reset_paste_state()`,focus/resize/suspend/resume 各接缝均重置 paste 状态;契约测试 `paste_timeout_restores_enter_submit_semantics` 与 `focus_and_resize_reset_stuck_paste_state` 已加入(`matches!` 消息参数笔误已修正)。(2) **问题 1(SIGTSTP/SIGCONT)**:按批复改用 **signal-hook flag 模式**——`SuspendFlags` 经 `signal_hook::flag::register` 注册 SIGTSTP/SIGCONT 原子 flag(handler 仅原子写,满足 async-signal-safe);主循环轮询:TSTP → `restore_terminal_for_suspend()`(禁 raw、禁 bracketed paste/focus/mouse、显示光标,主线程执行故无 async-signal-safe 约束)→ `low_level::unregister` 恢复默认处置 + `rustix::process::kill_process` 自举真正挂起 → 恢复后重新注册 flag handler;CONT → `resume_after_sigcont()`(re-enable raw mode + bracketed paste + focus + 按 `wants_mouse_capture` 策略重放 mouse capture + `invalidate_viewport()` + `terminal.clear()` 全量重绘 + 几何重排)。**Cargo.toml**:`signal-hook = \"0.3.18\"`(lock 现值 pin,零树增量),rustix 加 `process` feature(自举 SIGTSTP;`termios` 一并列入以备 handler 路径评估,均 safe)。**验证状态:已验证(master 侧)**——`cargo fmt` 干净;`cargo test --all-targets` 2035 通过 / 1 失败(唯一失败是预存的 `onboarding_saved_provider_contract::saved_provider_without_key_keeps_draft_guidance`,onboarding #562,与本次改动无关);`cargo clippy --all-targets -- -D warnings` 干净。期间 master 代修两处笔误:`rustix::process::pid` → `getpid()`(E0432);测试里 `seam` move-后借用 → `seam.clone()`(E0382);并修正 paste 超时语义——上限窗口判定须优先于 `next_event_waiting` 且超窗后重置 burst(`should_insert_unbracketed_paste_newline`),否则 `next_event_waiting=true` 会短路超时检查。stash@{0} 保留未 pop(rustix 路线 hunks 已丢弃重写)。",
    "ACK: 已删除\"方法 1\"一节(commit 待提交),只保留代码分析(方法 2)。测量方法确认有误——`timeout` 的 `real` 时间等于 timeout 参数本身,不携带启动耗时信息。结论(splash 2.15-4.15s)来自代码分析(SPLASH_EFFECTS 注释 + SPLASH_HOLD),不来自错误的测量。",
    "ACK: 已完成(commit 52af9e1)。**第 1 步测量**:peer `measure-session-load-perf` 完成 7 处插桩(perf_probe 模块,std-only 零依赖),但环境无 cargo 无法采样——master 用进程内测量探针(2000 msgs/1000 rust 代码块/冷缓存)采得实测分解:A(hydrate apply)2.9-19.4ms 可忽略;B(全量构建)debug 768-803ms / release 585ms,其中 syntect 占 B 的 88%;C(写 scrollback)debug 1006-1056ms / release 194ms;总耗时 debug 1.85s / release 0.79s。**第 2 步按数据选候选 a**(syntect 是最大子项,命中外环预判):`highlight.rs` 新增 thread_local DEFER_HIGHLIGHT + with_deferred_highlight(),viewport.rs discontinuity 分支用它包裹 finalized_history_lines;deferred 时纯色渲染、不读不写 BLOCK_CACHE(避免污染);视口/pager 每帧重建高亮不受影响。**前后对比**(同探针同规模):B 段 debug -92%、release -98.6%;syntect miss 归零;总耗时 debug -41%、release -74%。**R2 声明**:fmt 干净、clippy --all-targets -D warnings 干净、test --all-targets 2107/0 全绿;实测对比为同机同进程同探针前后两测;未覆盖真实 TTY 交互 hydrate;热点 2(逐帧指纹哈希)未动(契约只做候选 a)。插桩/探针文件按契约不提交,已从工作区移除;result.md 随 commit 入库备查。",
    "ACK: 已完成(commit e036ee2 主体 + 76bb758 复验整改,peer `fix-duplicate-hydrate` 实施)。**派发点定位**:确认两个——`hydrate_session_state_command`(session/opened / phantom probe 路径)与 `resume_session_command`(/resume 活跃会话恢复路径)。**去重方案**:`AppState.hydrate_in_flight: HashSet<SessionKey>`——两个 producer 派发前 `insert`,已在途则拒发第二发;清除时机:hydrate 结果落地、可归因的 error 帧到达、backend 重启(在途请求随旧子进程死亡)。**include 集合(复验整改)**:外环指出\"等价\"论断不成立——去重后 open 路径先发即赢,其 include 缺 `pending_questions` 会导致重启后 parked question 的 modal 静默丢失;已整改为 open 路径 include 并入 `pending_questions`(76bb758,上游无 hydrate_sections 常量,字面量与 resume 路径一致),并更新预存 include 集断言(4→5 sections)。**契约测试 7 个**:startup_double_hydrate_dedupes_to_one_request_and_one_apply、hydrate_in_flight_dedupes_open_path_after_resume、hydrate_answer_re_arms_dispatch、hydrate_error_clears_in_flight_marker、backend_relaunch_clears_in_flight_hydrates、open_path_hydrate_includes_pending_questions(include 携带)、hydrated_parked_question_surfaces_as_modal(重启后 parked question 呈现为可见 modal——复验要求的绑定测试)。**验证**:cargo test --all-targets 2114 passed / 0 failed;clippy --all-targets -D warnings 干净;fmt 干净。master 代修两处:复验测试的 `UserQuestionRequestedEvent::new` 参数个数与类型标注、多余的 `]` 笔误(e036ee2 阶段)。",
    "ACK: 知晓,仅记档不动手——octos 上游事项,随 OLP workstream 排期。",
    "ACK:",
    "ACK:",
    "ACK: P0 收尾完成(2026-08-24 凌晨,cargo 窗口恢复后补跑):`cargo build --release` 通过(36.8s 编译,Finished release),`cargo test --lib` **1938 passed / 0 failed / 1 ignored**(3.27s)——main 同步到 origin/main(876fd4a,含上游 #569-577)+ 黑板 11-15 条恢复(b0e0e72)后的树全绿。运维单六项全部闭合(存档分支 archive/local-main-20260824、reset、黑板恢复 commit、build、test、状态报告)。",
    "ACK:",
    "ACK: 已完成(分支 feat/olp-proto-v1,peer `implement-olp-proto-v1` 实施)。**片 1**(ed72951):ACK 定式语法 `ACK(done|wontdo|blocked): <说明>` 写入 OLP 文档与 AGENTS.md,协议头双处同步升 olp/v1,R1 修订落笔(wontdo 外环只能接受或升级,不得重复打回)。**片 2**(2c20085):result.md frontmatter v1 schema 附录(六字段 slug/outcome/updated_unix/turn/verified/protocol + 未知字段忽略约定)+ sub_providers cheap/strong 车道模板与双环搭配矩阵。**片 3**(5bab83d + master 收尾 f076e8d/5bbf665):`tests/olp_contract.rs` 五个契约测试全落地——ACK v1 语法(历史行豁免清单)、非法状态词拒绝、车道模板 TOML 可解析且 description 非空、双处版本一致 olp/v1、schema 字段清单一致;Cargo.lock 补 toml 依赖。**存量 ACK 行处理**:契约不含糊(豁免清单 + v1 生效日期分界),按候选方案直接实施,未写异议。**验证**(master 侧,cargo 窗口):cargo test --all-targets 2126 passed / 0 failed;clippy --all-targets -D warnings 干净;fmt 干净(f076e8d 补 fmt 修正)。",
    "ACK:",
    "ACK:",
];

fn is_exempt_legacy_ack(line: &str) -> bool {
    let trimmed = line.trim();
    // v1 lines start with `ACK(` and never hit this branch.
    trimmed.starts_with("ACK:") && LEGACY_ACK_WHITELIST.contains(&trimmed)
}

#[test]
fn olp_ack_lines_match_v1_grammar() {
    let blackboard = read("docs/OUTER_LOOP_REVIEW.md");
    let ack_lines = blackboard_ack_lines(&blackboard);
    assert!(!ack_lines.is_empty(), "blackboard must contain ACK lines");
    let mut violations = Vec::new();
    for line in &ack_lines {
        if ack_line_matches_v1(line) || is_exempt_legacy_ack(line) {
            continue;
        }
        violations.push(line.clone());
    }
    assert!(
        violations.is_empty(),
        "ACK lines violating v1 grammar (and not exempt legacy `ACK:` lines):\n{}",
        violations.join("\n")
    );
}

/// The exemption is a FROZEN whitelist, not a prefix rule: a legacy-form
/// `ACK:` line that is NOT on the v1-adoption whitelist must be rejected —
/// this is what makes adding a new `ACK:` line to the blackboard fail the
/// contract (#22a: the old `starts_with("ACK:")` exemption had no boundary
/// and let any future legacy line pass).
#[test]
fn olp_ack_exemption_is_bounded_whitelist() {
    // Every whitelisted legacy line is exempt.
    for line in LEGACY_ACK_WHITELIST {
        assert!(
            is_exempt_legacy_ack(line),
            "whitelisted legacy line must be exempt: {line}"
        );
    }
    // The frozen snapshot's 20 legacy `ACK:` lines are exactly the whitelist.
    let blackboard = read("docs/OUTER_LOOP_REVIEW.md");
    let legacy_on_board: Vec<String> = blackboard_ack_lines(&blackboard)
        .into_iter()
        .filter(|l| l.starts_with("ACK:"))
        .collect();
    assert_eq!(
        legacy_on_board.len(),
        LEGACY_ACK_WHITELIST.len(),
        "legacy `ACK:` lines on the frozen board must equal the whitelist \
         (new legacy-form lines are forbidden; remove the line or use the v1 form):\n{}",
        legacy_on_board.join("\n")
    );
    // A NEW legacy-form line (not on the whitelist) is NOT exempt.
    assert!(!is_exempt_legacy_ack(
        "ACK: 已完成(commit ffffff)。新写的旧格式行"
    ));
    // The bare placeholder `ACK:` IS whitelisted (4 frozen occurrences) —
    // leading/trailing whitespace is normalized before matching.
    assert!(is_exempt_legacy_ack("ACK:"));
    assert!(is_exempt_legacy_ack("  ACK:  "));
    // But a placeholder with ANY content appended is a new line — not exempt.
    assert!(!is_exempt_legacy_ack("ACK: 占位补内容"));
    // A whitelisted line with an appended suffix is a different line — not exempt.
    let suffixed = format!("{}（追加）", LEGACY_ACK_WHITELIST[0]);
    assert!(!is_exempt_legacy_ack(&suffixed));
    // v1-form lines never consult the whitelist.
    assert!(!is_exempt_legacy_ack("ACK(done): v1 form"));
}

#[test]
fn olp_ack_rejects_unknown_status() {
    // Known-good v1 lines parse.
    assert!(ack_line_matches_v1("ACK(done): shipped in commit abc123"));
    assert!(ack_line_matches_v1("ACK(wontdo): 异议:证据链不足"));
    assert!(ack_line_matches_v1(
        "  ACK(blocked): cargo 不可用,等待工具链窗口"
    ));
    // Unknown status words are rejected.
    assert!(!ack_line_matches_v1("ACK(finished): done-ish"));
    assert!(!ack_line_matches_v1("ACK(rejected): nope"));
    assert!(!ack_line_matches_v1("ACK(DONE): wrong case"));
    assert!(!ack_line_matches_v1("ACK(): empty status"));
    // Malformed shapes are rejected.
    assert!(!ack_line_matches_v1("ACK: legacy form is not v1"));
    assert!(!ack_line_matches_v1("ACK(done) missing colon"));
    assert!(!ack_line_matches_v1("ACK(done):   ")); // empty explanation
}

#[test]
fn olp_lane_template_parses() {
    let protocol = read("docs/OUTER_LOOP_PROTOCOL.md");
    // Locate the appendix B TOML fence and extract its body.
    let appendix_marker = "## 附录 B";
    let appendix_start = protocol
        .find(appendix_marker)
        .expect("OUTER_LOOP_PROTOCOL.md must contain appendix B (lane template)");
    let appendix = &protocol[appendix_start..];
    let fence_start = appendix
        .find("```toml\n")
        .expect("appendix B must contain a ```toml fenced block");
    let body = &appendix[fence_start + "```toml\n".len()..];
    let fence_end = body.find("```").expect("toml fence must be closed");
    let toml_body = &body[..fence_end];

    let parsed: toml::Value = toml_body.parse().expect("lane template TOML must parse");
    let sub_providers = parsed
        .get("sub_providers")
        .and_then(|v| v.as_table())
        .expect("lane template must define [sub_providers.<lane>] tables");
    assert!(
        !sub_providers.is_empty(),
        "lane template must declare at least one lane"
    );
    for (lane, config) in sub_providers {
        let description = config
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("lane `{lane}` must have a description string"));
        assert!(
            !description.trim().is_empty(),
            "lane `{lane}` description must be non-empty"
        );
    }
}

#[test]
fn olp_version_consistent_across_docs() {
    let agents = read("AGENTS.md");
    let protocol = read("docs/OUTER_LOOP_PROTOCOL.md");
    for (rel, text) in [
        ("AGENTS.md", agents.as_str()),
        ("docs/OUTER_LOOP_PROTOCOL.md", protocol.as_str()),
    ] {
        assert!(
            text.contains("olp/v1"),
            "{rel} must reference protocol version olp/v1"
        );
    }
    // Neither file's own protocol declaration may still say v0. (Quoted
    // historical mentions such as blackboard narratives live in
    // OUTER_LOOP_REVIEW.md and are out of scope for this check.)
    assert!(
        !agents.contains("protocol: olp/v0"),
        "AGENTS.md must not declare protocol: olp/v0"
    );
    assert!(
        !protocol.contains("`protocol: olp/v0`"),
        "OUTER_LOOP_PROTOCOL.md must not declare protocol: olp/v0"
    );
}

#[test]
fn olp_result_schema_fields_documented() {
    let protocol = read("docs/OUTER_LOOP_PROTOCOL.md");
    let appendix_marker = "## 附录 A";
    let appendix_start = protocol
        .find(appendix_marker)
        .expect("OUTER_LOOP_PROTOCOL.md must contain appendix A (result.md schema)");
    // Appendix A runs to the next `## ` heading (appendix B) or EOF.
    let appendix = &protocol[appendix_start..];
    let appendix_end = appendix[appendix_marker.len()..]
        .find("\n## ")
        .map(|i| i + appendix_marker.len())
        .unwrap_or(appendix.len());
    let appendix = &appendix[..appendix_end];

    // Field names are documented as `field` in the schema table rows.
    let mut fields = BTreeSet::new();
    for line in appendix.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let Some(first_cell) = line.split('|').nth(1) else {
            continue;
        };
        let cell = first_cell.trim();
        if let Some(name) = cell.strip_prefix('`').and_then(|c| c.strip_suffix('`')) {
            fields.insert(name.to_string());
        }
    }
    let expected: BTreeSet<String> = [
        "slug",
        "outcome",
        "updated_unix",
        "turn",
        "verified",
        "protocol",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        fields, expected,
        "result.md schema fields documented in appendix A must be exactly \
         {{slug, outcome, updated_unix, turn, verified, protocol}}"
    );
}
