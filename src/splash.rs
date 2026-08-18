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
}

impl SplashSession {
    pub fn new(effect_args: &[&str], text: &str, opts: SessionOpts) -> Result<Self> {
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
        let (_, rows) = text_dimensions(text);
        Ok(SplashSession {
            effect,
            ctx,
            text: text.to_string(),
            rows,
        })
    }

    /// Drive the effect to completion, `should_stop`, or error. Always ends by
    /// painting the plain input text into the canvas area (deterministic final
    /// state) and parking the cursor on the line below it.
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
        if result.is_ok() {
            let text = self.text.clone();
            self.paint(out, &text)?;
        }
        // Park below the canvas either way.
        out.write_all(b"\r\n").wrap_err("splash: park cursor")?;
        out.flush().ok();
        result.map(|_| stats)
    }

    /// Draw `rows` lines over the reserved canvas area. Expects the cursor at
    /// column 0 of the bottom canvas row; restores that invariant on return.
    fn paint(&self, out: &mut impl Write, frame: &str) -> Result<()> {
        let up = self.rows.saturating_sub(1);
        if up > 0 {
            write!(out, "\x1b[{up}A").wrap_err("splash: cursor up")?;
        }
        let mut first = true;
        for line in frame.split('\n') {
            if !first {
                out.write_all(b"\r\n").wrap_err("splash: row break")?;
            }
            first = false;
            // Clear the row before drawing so a shorter final paint fully
            // covers the last animation frame.
            out.write_all(b"\r\x1b[2K").wrap_err("splash: clear row")?;
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
    let _ = play_inner();
}

fn play_inner() -> Result<()> {
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
    let mut session = SplashSession::new(
        effect_args,
        &splash_text(),
        SessionOpts {
            frame_rate: 60,
            virtual_clock: false,
            seed,
        },
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
