//! Startup splash: a ttfx-rendered OCTOS logo animation played on the main
//! screen before the event loop claims the terminal.
//! Contract: specs/task-startup-splash.spec.

use unicode_width::UnicodeWidthStr;

/// Block-letter OCTOS. 44 columns wide, 6 rows tall; all glyphs are
/// single-width so ttfx canvas geometry matches `lines()`/width math.
const LOGO: &str = "\
 ██████╗  ██████╗████████╗ ██████╗ ███████╗
██╔═══██╗██╔════╝╚══██╔══╝██╔═══██╗██╔════╝
██║   ██║██║        ██║   ██║   ██║███████╗
██║   ██║██║        ██║   ██║   ██║╚════██║
╚██████╔╝╚██████╗   ██║   ╚██████╔╝███████║
 ╚═════╝  ╚═════╝   ╚═╝    ╚═════╝ ╚══════╝";

/// Curated effects as ttfx CLI arg lists (`ttfx <name> [args…]` must parse).
/// Each runs to natural completion at startup, so members are limited to
/// ~1.7–4.5s natural duration on this logo at 60fps (e.g. decrypt = 12.4s was
/// dropped). Durations were measured with the virtual clock — EXCEPT effects
/// that read wall time (matrix), whose phase lengths are set explicitly in
/// args because the virtual measure does not predict their real duration.
pub const SPLASH_EFFECTS: [&[&str]; 9] = [
    &["beams"],     // 3.5s
    &["sweep"],     // 3.7s
    &["wipe"],      // 2.3s
    &["rain"],      // 2.8s
    &["slide"],     // 1.9s
    &["scattered"], // 1.8s
    &["middleout"], // 1.7s
    &["highlight"], // 2.1s
    // Matrix paces its phases on WALL time (not frames), so its duration is
    // tuned via args: ~3s in release builds, ~5s in debug (heavier frames).
    &[
        "matrix",
        "--rain-time",
        "1",
        "--rain-fall-delay-range",
        "1-4",
        "--rain-column-delay-range",
        "1-3",
        "--resolve-delay",
        "1",
    ],
];

/// The animated input: logo plus a version footer line.
pub fn splash_text() -> String {
    format!(
        "{LOGO}\n\n         octoscode v{}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Widest line / line count of the splash text, for the gate and printer.
fn text_dimensions(text: &str) -> (u16, u16) {
    let cols = text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0);
    let rows = text.lines().count();
    (cols as u16, rows as u16)
}

/// Every input to the play/skip decision, resolved by the caller so the
/// decision itself is a pure function (spec: 门控).
#[derive(Debug, Clone, Copy)]
pub struct SplashGate {
    pub no_splash_flag: bool,
    /// OCTOSCODE_NO_SPLASH is set (any value).
    pub env_disabled: bool,
    pub stdout_is_tty: bool,
    /// CI env var is set (any value).
    pub ci: bool,
    pub term_cols: u16,
    pub term_rows: u16,
}

/// All skip conditions are an unordered OR; any hit disables the splash.
pub fn should_play(gate: &SplashGate) -> bool {
    let (logo_cols, logo_rows) = text_dimensions(&splash_text());
    !gate.no_splash_flag
        && !gate.env_disabled
        && gate.stdout_is_tty
        && !gate.ci
        && gate.term_cols >= logo_cols
        // +2: one row below the canvas for the parked cursor, one of headroom.
        && gate.term_rows >= logo_rows + 2
}

/// Deterministic pick from SPLASH_EFFECTS (seeded ttfx Rng, unit-testable).
pub fn pick_effect_args(seed: u64) -> &'static [&'static str] {
    let mut rng = ttfx::utils::rng::Rng::seeded(seed);
    SPLASH_EFFECTS[rng.choice_index(SPLASH_EFFECTS.len())]
}

/// Curated entry for a specific effect name (`OCTOSCODE_SPLASH_EFFECT=matrix`
/// pins the pick). Curated-only on purpose: arbitrary ttfx effects would break
/// the duration guarantees the list encodes.
pub fn effect_args_for(name: &str) -> Option<&'static [&'static str]> {
    SPLASH_EFFECTS.iter().copied().find(|args| args[0] == name)
}

use std::io::Write;

use eyre::{Result, WrapErr, eyre};
use ttfx::engine::ctx::{Clock, EngineCtx};
use ttfx::engine::effect::Effect;
use ttfx::engine::terminal::TerminalConfig;
use ttfx::utils::rng::Rng;

/// Hang safety net only: the curated effects finish naturally in ≤3.7s
/// (matrix: ~3s release / ~5s debug), so this cap fires only if an effect
/// misbehaves — never in the normal path.
pub const SPLASH_DEADLINE: std::time::Duration = std::time::Duration::from_millis(8000);

/// Beat between the settled logo and the TUI taking over, so the ending
/// doesn't jump-cut. Cut short by any key press.
pub const SPLASH_HOLD: std::time::Duration = std::time::Duration::from_millis(450);

#[derive(Debug, Clone, Copy)]
pub struct SessionOpts {
    /// 0 disables frame pacing (tests); production uses 60.
    pub frame_rate: i64,
    /// Virtual clock: effects that read time advance per frame, no sleeping.
    pub virtual_clock: bool,
    pub seed: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct RunStats {
    pub frames: usize,
    pub truncated: bool,
}

/// One prepared splash: a built ttfx effect plus its engine context.
pub struct SplashSession {
    effect: Box<dyn Effect>,
    ctx: EngineCtx,
    text: String,
    rows: u16,
    /// Left padding that block-centers the FINAL TEXT to the terminal width.
    /// Computed once here from `text_dimensions(text)` — never from animation
    /// frames, whose SGR color sequences (`\x1b[38;5;196m` etc.) would inflate
    /// `UnicodeWidthStr::width` and collapse the pad to 0 (Bug 1).
    block_pad: usize,
    /// SGR color sequence for the final paint, matching the launch banner's
    /// accent color (theme-aware). Empty string means no color (default).
    final_color: String,
}

impl SplashSession {
    pub fn new(
        effect_args: &[&str],
        text: &str,
        opts: SessionOpts,
        term_cols: u16,
        final_color: String,
    ) -> Result<Self> {
        if text.trim().is_empty() {
            return Err(eyre!("splash input text is empty"));
        }
        // Effect config via ttfx's own CLI (like its --random-effect path);
        // extra args tune effect phases, e.g. matrix --rain-time.
        let argv = std::iter::once("ttfx").chain(effect_args.iter().copied());
        let parsed = <ttfx::cli::Cli as clap::Parser>::try_parse_from(argv)
            .map_err(|e| eyre!("bad ttfx effect args {effect_args:?}: {e}"))?;
        let effect = parsed
            .effect
            .ok_or_else(|| eyre!("ttfx parsed no effect for {effect_args:?}"))?
            .build_effect();

        let config = TerminalConfig {
            frame_rate: opts.frame_rate,
            // Pin the canvas to the input text so every frame has exactly
            // `rows` lines regardless of (or absent) real terminal dimensions.
            ignore_terminal_dimensions: true,
            no_color: std::env::var_os("NO_COLOR").is_some(),
            ..TerminalConfig::default()
        };
        let clock = if opts.virtual_clock {
            Clock::virtual_with_frame_rate(config.frame_rate.max(1))
        } else {
            Clock::real()
        };
        let ctx = EngineCtx::new(text, config, Rng::seeded(opts.seed), clock)
            .map_err(|e| eyre!("ttfx engine init failed: {e:?}"))?;
        let (cols, rows) = text_dimensions(text);
        // ANSI-immune: measured from the plain final text, so the pad is
        // stable across frames and matches the final paint exactly.
        let block_pad = (term_cols as usize).saturating_sub(cols as usize) / 2;
        Ok(SplashSession {
            effect,
            ctx,
            text: text.to_string(),
            rows,
            block_pad,
            final_color,
        })
    }

    /// Drive the effect to completion, `should_stop`, or error. Always ends by
    /// painting the plain input text into the canvas area (deterministic final
    /// state) and returning the cursor to the canvas TOP row, so the TUI's
    /// inline viewport starts exactly over the splash's final frame.
    ///
    /// Raw-mode safe: rows are repositioned with `\r` + cursor-up instead of
    /// relying on cooked-mode `\n` (ttfx frames join rows with bare `\n`).
    pub fn run(
        &mut self,
        out: &mut impl Write,
        mut should_stop: impl FnMut() -> bool,
    ) -> Result<RunStats> {
        self.effect
            .build(&mut self.ctx)
            .map_err(|e| eyre!("ttfx effect build failed: {e:?}"))?;

        // Reserve the canvas: rows-1 newlines put the cursor at column 0 of
        // the bottom canvas row (invariant between frames).
        out.write_all(
            "\r\n"
                .repeat(self.rows.saturating_sub(1) as usize)
                .as_bytes(),
        )
        .wrap_err("splash: reserve canvas")?;

        let mut stats = RunStats {
            frames: 0,
            truncated: false,
        };
        let result: Result<()> = (|| {
            loop {
                if should_stop() {
                    stats.truncated = true;
                    return Ok(());
                }
                let Some(frame) = self.effect.next_frame(&mut self.ctx) else {
                    return Ok(());
                };
                self.paint(out, &frame)?;
                stats.frames += 1;
                self.ctx.terminal.enforce_framerate();
            }
        })();

        // Final paint runs on success AND truncation; a paint error skips it.
        // The final frame is painted in the launch banner's accent color
        // (theme-aware) so the splash → banner handoff is color-smooth.
        if result.is_ok() {
            let text = self.text.clone();
            let colored = format!("{}{}\x1b[0m", self.final_color, text);
            self.paint(out, &colored)?;
        }
        // Move the cursor back UP to the canvas top row (paint left it at the
        // bottom row) instead of parking below the canvas. octoscode's TUI is
        // an INLINE VIEWPORT that starts at the current cursor row — parking
        // below would push the launch banner `rows` lines down and leave
        // splash residue above it (Bug 2). With the cursor on the canvas top
        // row, the banner's first frame overwrites the splash's final frame
        // in place (smooth handoff).
        let up = self.rows.saturating_sub(1);
        if up > 0 {
            write!(out, "\x1b[{up}A").wrap_err("splash: cursor to canvas top")?;
        }
        out.write_all(b"\r").wrap_err("splash: home")?;
        out.flush().ok();
        result.map(|_| stats)
    }

    /// Draw `rows` lines over the reserved canvas area, CENTERED to
    /// `term_cols` as a BLOCK. Expects the cursor at column 0 of the bottom
    /// canvas row; restores that invariant on return.
    ///
    /// The pad is the precomputed `self.block_pad` (from the FINAL TEXT width
    /// in `new()`), NOT measured from `frame`: ttfx frames carry SGR color
    /// sequences (`\x1b[38;5;196m` etc.) whose printable characters inflate
    /// `UnicodeWidthStr::width` into the hundreds, collapsing the pad to 0
    /// (Bug 1: left-aligned animation, then a sudden jump to centered on the
    /// plain final frame). Measuring the final text once is ANSI-immune,
    /// stable across frames, and agrees with the final paint by construction.
    /// Rows narrower than the block stay left-aligned within it, matching the
    /// launch banner's figlet centering (`{art:<fig_w$}` then `centered()`),
    /// so the final paint lands where the banner renders (smooth handoff).
    fn paint(&self, out: &mut impl Write, frame: &str) -> Result<()> {
        let up = self.rows.saturating_sub(1);
        if up > 0 {
            write!(out, "\x1b[{up}A").wrap_err("splash: cursor up")?;
        }
        let block_pad = self.block_pad;
        let mut first = true;
        for line in frame.split('\n') {
            if !first {
                out.write_all(b"\r\n").wrap_err("splash: row break")?;
            }
            first = false;
            // Clear the row before drawing so a shorter final paint fully
            // covers the last animation frame.
            out.write_all(b"\r\x1b[2K").wrap_err("splash: clear row")?;
            // Block centering: pad left by the block offset, then draw the row.
            if block_pad > 0 {
                out.write_all(" ".repeat(block_pad).as_bytes())
                    .wrap_err("splash: block pad")?;
            }
            out.write_all(line.as_bytes()).wrap_err("splash: row")?;
        }
        out.write_all(b"\r").wrap_err("splash: home")?;
        out.flush().wrap_err("splash: flush")
    }
}

/// Play the startup splash if gating allows. Every failure is swallowed:
/// the splash is decoration and must never block or delay startup beyond
/// its own deadline (specs/task-startup-splash.spec: 失败静默).
pub fn play(cli: &crate::cli::Cli) {
    use std::io::IsTerminal;

    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((0, 0));
    let gate = SplashGate {
        no_splash_flag: cli.no_splash,
        env_disabled: std::env::var_os("OCTOSCODE_NO_SPLASH").is_some(),
        stdout_is_tty: std::io::stdout().is_terminal(),
        ci: std::env::var_os("CI").is_some(),
        term_cols,
        term_rows,
    };
    if !should_play(&gate) {
        return;
    }
    let _ = play_inner(&cli.theme);
}

fn play_inner(theme: &crate::cli::ThemeName) -> Result<()> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) ^ d.as_secs())
        .unwrap_or(0);
    // OCTOSCODE_SPLASH_EFFECT pins a curated effect by name (e.g. `matrix`);
    // unset or unknown names fall back to the seeded random pick.
    let effect_args = std::env::var("OCTOSCODE_SPLASH_EFFECT")
        .ok()
        .and_then(|name| effect_args_for(name.trim()))
        .unwrap_or_else(|| pick_effect_args(seed));
    let (term_cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
    // Theme-aware final color: match the launch banner's accent color.
    // Honor NO_COLOR: if set, the final paint is plain (no color).
    let palette = crate::theme::Palette::for_theme(*theme);
    let final_color = if std::env::var_os("NO_COLOR").is_some() {
        String::new()
    } else {
        color_to_sgr(palette.accent)
    };
    let mut session = SplashSession::new(
        effect_args,
        &splash_text(),
        SessionOpts {
            frame_rate: 60,
            virtual_clock: false,
            seed,
        },
        term_cols,
        final_color,
    )?;

    // Raw mode for echo-free any-key skip; the guard restores it on every
    // exit path (the panic hook in main.rs also disables raw mode, so a
    // double-disable is harmless).
    struct RawGuard;
    impl Drop for RawGuard {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);
        }
    }
    crossterm::terminal::enable_raw_mode().wrap_err("splash: raw mode")?;
    let _guard = RawGuard;
    let mut stdout = std::io::stdout().lock();
    crossterm::execute!(stdout, crossterm::cursor::Hide).ok();

    // NO cursor::MoveTo(0, 0): octoscode's TUI is an inline viewport that
    // starts at the CURRENT cursor row — jumping to screen row 0 would paint
    // the splash over shell scrollback and then leave the banner starting
    // `rows` lines below (Bug 2). The splash plays from the cursor row, and
    // `run()` returns the cursor to the canvas top so the banner's first
    // frame overwrites the splash's final frame in place.

    let deadline = std::time::Instant::now() + SPLASH_DEADLINE;
    let stats = session.run(&mut stdout, || {
        std::time::Instant::now() >= deadline || key_or_resize_pending()
    })?;

    // Hold the settled logo for a beat before the TUI takes over — a skipped
    // run means the user is in a hurry, so no hold there.
    if !stats.truncated {
        let hold_until = std::time::Instant::now() + SPLASH_HOLD;
        while std::time::Instant::now() < hold_until && !key_or_resize_pending() {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
    Ok(())
}

/// Drain pending terminal events; any key press or resize stops the splash.
/// Errors read as "stop" — never let event plumbing wedge the animation.
fn key_or_resize_pending() -> bool {
    use crossterm::event::{Event, KeyEventKind, poll, read};
    loop {
        match poll(std::time::Duration::ZERO) {
            Ok(true) => match read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => return true,
                Ok(Event::Resize(..)) => return true,
                Ok(_) => continue,
                Err(_) => return true,
            },
            Ok(false) => return false,
            Err(_) => return true,
        }
    }
}

/// Convert a `Color` to an SGR foreground color sequence (e.g.
/// `Color::Rgb(99, 151, 255)` → `\x1b[38;2;99;151;255m`). Returns an empty
/// string for `Color::Reset` (no color).
fn color_to_sgr(color: ratatui::style::Color) -> String {
    use ratatui::style::Color;
    match color {
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
        Color::Cyan => "\x1b[36m".to_string(),
        Color::Blue => "\x1b[34m".to_string(),
        Color::Magenta => "\x1b[35m".to_string(),
        Color::Yellow => "\x1b[33m".to_string(),
        Color::Green => "\x1b[32m".to_string(),
        Color::Red => "\x1b[31m".to_string(),
        Color::White => "\x1b[37m".to_string(),
        Color::Black => "\x1b[30m".to_string(),
        Color::Reset => String::new(),
        // Fallback for other colors: use white
        _ => "\x1b[37m".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_opts() -> SessionOpts {
        SessionOpts {
            frame_rate: 0,
            virtual_clock: true,
            seed: 42,
        }
    }

    #[test]
    fn paint_centers_block_to_terminal() {
        // Capture `paint()`'s output and verify every row has the same left
        // padding (block centered to term_cols).
        let text = "AB\nCD"; // 2 rows, 2 cols each
        let term_cols = 10u16;
        let session = SplashSession::new(&["beams"], text, test_opts(), term_cols, String::new())
            .expect("session builds");

        let mut out = Vec::new();
        session.paint(&mut out, text).expect("paint succeeds");
        let output = String::from_utf8_lossy(&out);

        // Block width = 2, term_cols = 10 → block_pad = 4. Every row starts
        // with 4 spaces after the row-clear escape.
        let rows: Vec<&str> = output.split("\r\n").collect();
        assert!(rows.len() >= 2, "at least 2 rows, got {}", rows.len());
        for (i, row) in rows.iter().take(2).enumerate() {
            assert!(
                row.contains("\x1b[2K    "),
                "row {i} has 4-space pad after clear, got: {row:?}"
            );
        }
    }

    #[test]
    fn paint_block_pad_is_ansi_immune() {
        // Bug 1 regression: ttfx frames carry SGR color sequences
        // (`\x1b[38;5;196m` etc.). `UnicodeWidthStr::width` counts the
        // printable characters of those sequences as width, so measuring the
        // FRAME would inflate block_width past term_cols and collapse
        // block_pad to 0 (left-aligned animation, then a sudden jump to
        // centered on the plain final frame). The pad is computed once in
        // `new()` from the final TEXT width and must be identical for
        // colored frames.
        let text = "AB\nCD"; // final text: 2 cols wide
        let term_cols = 10u16;
        let session = SplashSession::new(&["beams"], text, test_opts(), term_cols, String::new())
            .expect("session builds");

        // A frame wrapped in SGR sequences, like ttfx's colored output.
        // Naive width math: "\x1b[38;5;196mAB\x1b[0m" has ~17 printable
        // bytes → block_width ≥ 17 > term_cols → block_pad = 0. The correct
        // pad stays at (10 - 2) / 2 = 4.
        let colored_frame = "\x1b[38;5;196mAB\x1b[0m\n\x1b[38;5;46mCD\x1b[0m";

        let mut out = Vec::new();
        session
            .paint(&mut out, colored_frame)
            .expect("paint succeeds");
        let output = String::from_utf8_lossy(&out);

        let rows: Vec<&str> = output.split("\r\n").collect();
        assert!(rows.len() >= 2, "at least 2 rows, got {}", rows.len());
        for (i, row) in rows.iter().take(2).enumerate() {
            assert!(
                row.contains("\x1b[2K    "),
                "colored frame row {i} keeps the 4-space pad, got: {row:?}"
            );
        }

        // Sanity: the plain final text gets the SAME pad — animation frames
        // and the final paint agree, so there is no jump at handoff.
        let mut out_plain = Vec::new();
        session.paint(&mut out_plain, text).expect("paint succeeds");
        let plain = String::from_utf8_lossy(&out_plain);
        for (i, row) in plain.split("\r\n").take(2).enumerate() {
            assert!(
                row.contains("\x1b[2K    "),
                "plain frame row {i} keeps the 4-space pad, got: {row:?}"
            );
        }
    }

    #[test]
    fn color_to_sgr_maps_theme_accent_colors() {
        use ratatui::style::Color;
        // Codex theme blue
        assert_eq!(
            color_to_sgr(Color::Rgb(99, 151, 255)),
            "\x1b[38;2;99;151;255m"
        );
        // Terminal theme cyan
        assert_eq!(color_to_sgr(Color::Cyan), "\x1b[36m");
        // Reset (no color)
        assert_eq!(color_to_sgr(Color::Reset), "");
    }

    #[test]
    fn run_leaves_cursor_on_canvas_top_row() {
        // Bug 2 regression: the TUI's inline viewport starts at the current
        // cursor row, so after the splash the cursor must be back on the
        // canvas TOP row (not parked below the canvas). Verify run()'s tail:
        // after the final paint's trailing `\r`, the last escape is
        // `\x1b[{rows-1}A\r` (cursor up to the canvas top, then home) — and
        // NOT `\r\n` (park below).
        let text = "AB\nCD"; // rows = 2 → cursor-up count = 1
        let mut session = SplashSession::new(&["beams"], text, test_opts(), 10, String::new())
            .expect("session builds");

        let mut out = Vec::new();
        session.run(&mut out, || true).expect("run succeeds"); // skip on first poll
        let output = String::from_utf8_lossy(&out);

        assert!(
            output.ends_with("\x1b[1A\r"),
            "cursor returns to the canvas top row, got tail: {:?}",
            &output[output.len().saturating_sub(32)..]
        );
        assert!(
            !output.ends_with("\r\n"),
            "cursor must NOT be parked below the canvas"
        );
    }
}
