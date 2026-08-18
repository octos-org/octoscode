# Startup Splash (ttfx) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Play a ttfx-rendered OCTOS ASCII-logo animation on the main screen at startup (before `event_loop::run` claims the terminal), capped at 1.5s, skippable by any key, gated off for non-TTY/CI/`--no-splash`, ending in a full plain logo banner; all failures silent.

**Architecture:** New `src/splash.rs` module drives ttfx's public engine primitives (`Effect::build`/`next_frame` + `EngineCtx`) with a custom raw-mode-safe frame printer (ttfx frames join rows with bare `\n`, which staircases under raw mode — we reposition with `\r\n` + cursor-up ourselves). Effect configs are obtained the same way ttfx's own `--random-effect` does: `clap try_parse_from(["ttfx", name])`. Gating, effect picking, and the frame loop are pure/injectable (Vec writer + virtual clock + frame_rate 0) so tests never sleep or touch a TTY; only `play()` touches crossterm raw mode and is verified manually.

**Tech Stack:** Rust 2024, ttfx (git dep, engine primitives), crossterm 0.28 (raw mode + key/resize poll), clap 4, eyre.

**Spec:** `specs/task-startup-splash.spec` (committed, lint 100%). Test names below are bound by the spec — do not rename.

## Global Constraints

- Allowed changes ONLY: `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml.example`, `src/main.rs`, `src/lib.rs`, `src/splash.rs`, `src/cli.rs`, `tests/splash_contract.rs`, `specs/**`.
- No new crate dependencies other than `ttfx`.
- No `unsafe` in this crate (workspace `unsafe_code = "deny"`); do not call ttfx's signal-handler APIs (`install_sigint_handler` etc.).
- Splash must never panic the process or delay startup beyond its own deadline; `splash::play` returns `()` and swallows every error.
- Do not enter the alternate screen in splash; do not change `event_loop`'s terminal takeover.
- Commit after each task. Run `cargo fmt` before each commit.

---

### Task 1: ttfx git dependency

**Files:**
- Modify: `Cargo.toml` (dependencies section)
- Modify: `.cargo/config.toml.example`
- Modify: `Cargo.lock` (regenerated)

**Interfaces:**
- Produces: `ttfx` crate available to `src/splash.rs` (`ttfx::cli::Cli`, `ttfx::effects::EffectCommand`, `ttfx::engine::{ctx::EngineCtx, ctx::Clock, effect::Effect, terminal::TerminalConfig}`, `ttfx::utils::rng::Rng`).

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, after the `octos-core` dependency block, add:

```toml
# Startup splash animation engine (specs/task-startup-splash.spec). Git dep +
# optional local patch, same pattern as octos-core above. ttfx itself only
# depends on clap + terminal_size.
ttfx = { git = "https://github.com/omacom-io/ttfx", rev = "6e24dac78e3011d89bd7ff24d1ad91dd89e11d8a" }
```

- [ ] **Step 2: Add the local-dev patch example**

Read `.cargo/config.toml.example` first, then append (matching its existing comment style):

```toml
# Live-develop the splash engine against a local ttfx checkout:
# [patch."https://github.com/omacom-io/ttfx"]
# ttfx = { path = "../consult/ttfx" }
```

(Keep it commented out — it is an example file.)

- [ ] **Step 3: Verify it builds**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors; `cargo tree -p ttfx | head -3` shows ttfx 0.3.1.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock .cargo/config.toml.example
git commit -m "feat(splash): add ttfx git dependency for the startup animation"
```

---

### Task 2: `--no-splash` CLI flag

**Files:**
- Modify: `src/cli.rs` (struct `Cli` ~line 121, struct `CliArgs` ~line 189, `from_args` ~line 418, tests module at bottom)

**Interfaces:**
- Produces: `Cli.no_splash: bool` (consumed by Task 5's `play(&cli)`).

- [ ] **Step 1: Write the failing test**

In the existing `#[cfg(test)] mod tests` in `src/cli.rs`, next to `parses_theme_choice`:

```rust
/// specs/task-startup-splash.spec: --no-splash disables the startup animation.
#[test]
fn cli_parses_no_splash_flag() {
    let cli = Cli::try_parse_from(["octoscode", "--no-splash"]).expect("cli parses");
    assert!(cli.no_splash);
    let cli = Cli::try_parse_from(["octoscode"]).expect("cli parses");
    assert!(!cli.no_splash);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cli_parses_no_splash_flag 2>&1 | tail -5`
Expected: FAIL — `no field no_splash`.

- [ ] **Step 3: Implement**

In `pub struct Cli` (after `steer_mid_turn`):

```rust
    /// Skip the ttfx startup animation (also skipped for non-TTY/CI, or via
    /// OCTOSCODE_NO_SPLASH).
    pub no_splash: bool,
```

In `struct CliArgs` (after its `steer_mid_turn` field):

```rust
    /// Skip the startup logo animation.
    #[arg(long = "no-splash")]
    pub no_splash: bool,
```

In `from_args`, in the `Cli { ... }` literal after `steer_mid_turn: ...`:

```rust
            no_splash: args.no_splash,
```

(No config-file key: the spec gates via flag + env var only, and `CliFileConfig` is `deny_unknown_fields`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib cli_parses_no_splash_flag 2>&1 | tail -5`
Expected: PASS. Also run `cargo test --lib 2>&1 | tail -3` — no other cli test may break.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "feat(splash): add --no-splash flag"
```

---

### Task 3: splash module — logo, gating, effect picking

**Files:**
- Create: `src/splash.rs`
- Modify: `src/lib.rs` (add `pub mod splash;` to the module list, alphabetical: after `pub mod sanitize;`)
- Create: `tests/splash_contract.rs`

**Interfaces:**
- Produces:
  - `splash::SPLASH_EFFECTS: [&str; 6]`
  - `splash::splash_text() -> String` (logo + version line)
  - `splash::SplashGate { no_splash_flag, env_disabled, stdout_is_tty, ci, term_cols, term_rows }` + `splash::should_play(&SplashGate) -> bool`
  - `splash::pick_effect_name(seed: u64) -> &'static str`
  - Task 4 consumes `splash_text()` and `pick_effect_name`.

- [ ] **Step 1: Write the failing tests**

Create `tests/splash_contract.rs`:

```rust
//! Contract tests for specs/task-startup-splash.spec.

use octoscode::splash::{
    pick_effect_name, should_play, splash_text, SplashGate, SPLASH_EFFECTS,
};

/// A gate whose every condition allows playback; tests flip one field each.
fn open_gate() -> SplashGate {
    SplashGate {
        no_splash_flag: false,
        env_disabled: false,
        stdout_is_tty: true,
        ci: false,
        term_cols: 120,
        term_rows: 40,
    }
}

#[test]
fn should_play_true_when_interactive_and_wide_enough() {
    assert!(should_play(&open_gate()));
}

#[test]
fn should_play_false_when_stdout_not_tty() {
    let gate = SplashGate { stdout_is_tty: false, ..open_gate() };
    assert!(!should_play(&gate));
}

#[test]
fn should_play_false_on_flag_or_env() {
    let gate = SplashGate { no_splash_flag: true, ..open_gate() };
    assert!(!should_play(&gate));
    let gate = SplashGate { env_disabled: true, ..open_gate() };
    assert!(!should_play(&gate));
}

#[test]
fn should_play_false_in_ci() {
    let gate = SplashGate { ci: true, ..open_gate() };
    assert!(!should_play(&gate));
}

#[test]
fn should_play_false_when_terminal_narrower_than_logo() {
    let gate = SplashGate { term_cols: 30, ..open_gate() };
    assert!(!should_play(&gate));
    let gate = SplashGate { term_rows: 5, ..open_gate() };
    assert!(!should_play(&gate));
}

#[test]
fn pick_effect_stays_in_curated_list() {
    for seed in 0..64u64 {
        let name = pick_effect_name(seed);
        assert!(
            SPLASH_EFFECTS.contains(&name),
            "seed {seed} picked {name}, not in SPLASH_EFFECTS"
        );
    }
}

#[test]
fn splash_text_carries_logo_and_version() {
    let text = splash_text();
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    assert!(text.lines().count() >= 6, "logo should be multi-line");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test splash_contract 2>&1 | tail -5`
Expected: FAIL — `could not find splash in octoscode`.

- [ ] **Step 3: Implement `src/splash.rs` (gating half)**

```rust
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

/// Curated effects, chosen for reading well when truncated at ~1.5s.
/// Members may be tuned after visual testing; keep them valid ttfx
/// subcommand names (`ttfx <name>` must parse).
pub const SPLASH_EFFECTS: [&str; 6] = ["decrypt", "beams", "sweep", "wipe", "slice", "expand"];

/// The animated input: logo plus a version footer line.
pub fn splash_text() -> String {
    format!("{LOGO}\n\n         octoscode v{}", env!("CARGO_PKG_VERSION"))
}

/// Widest line / line count of the splash text, for the gate and printer.
pub(crate) fn text_dimensions(text: &str) -> (u16, u16) {
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
pub fn pick_effect_name(seed: u64) -> &'static str {
    let mut rng = ttfx::utils::rng::Rng::seeded(seed);
    SPLASH_EFFECTS[rng.choice_index(SPLASH_EFFECTS.len())]
}
```

In `src/lib.rs`, add to the module list (after `pub mod sanitize;`):

```rust
pub mod splash;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test splash_contract 2>&1 | tail -5`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/splash.rs src/lib.rs tests/splash_contract.rs
git commit -m "feat(splash): logo, gating, and curated effect picking"
```

---

### Task 4: SplashSession — engine wiring + raw-mode-safe frame loop

**Files:**
- Modify: `src/splash.rs`
- Modify: `tests/splash_contract.rs`

**Interfaces:**
- Consumes: `splash_text()`, `text_dimensions()`, `pick_effect_name` (Task 3).
- Produces:
  - `splash::SplashSession::new(effect_name: &str, text: &str, opts: SessionOpts) -> eyre::Result<SplashSession>`
  - `splash::SessionOpts { frame_rate: i64, virtual_clock: bool, seed: u64 }`
  - `SplashSession::run(&mut self, out: &mut impl std::io::Write, should_stop: impl FnMut() -> bool) -> eyre::Result<RunStats>`
  - `splash::RunStats { frames: usize, truncated: bool }`
  - Task 5 consumes `SplashSession` + `SessionOpts` from `play()`.

**Design notes (why not ttfx's own loop):** ttfx frames join rows with bare `\n` and `prep_canvas`/`print_frame` assume cooked mode; under raw mode that staircases. `run()` therefore reserves rows itself and repositions with `\r` + `ESC[{n}A`, which renders identically in cooked and raw mode. `TerminalConfig.ignore_terminal_dimensions = true` pins the canvas to the input text dimensions so every frame has exactly `rows` lines. `frame_rate: 0` disables `enforce_framerate`'s sleep (used by tests); the run loop always ends by painting the plain input text into the canvas area, so a truncated run finishes on the complete logo.

- [ ] **Step 1: Write the failing tests**

Append to `tests/splash_contract.rs`:

```rust
use octoscode::splash::{SessionOpts, SplashSession};

fn test_opts() -> SessionOpts {
    SessionOpts { frame_rate: 0, virtual_clock: true, seed: 7 }
}

#[test]
fn curated_effects_produce_frames_on_virtual_clock() {
    for name in SPLASH_EFFECTS {
        let mut session = SplashSession::new(name, &splash_text(), test_opts())
            .unwrap_or_else(|e| panic!("{name}: session builds: {e}"));
        let mut out: Vec<u8> = Vec::new();
        let stats = session
            .run(&mut out, || false)
            .unwrap_or_else(|e| panic!("{name}: run ok: {e}"));
        assert!(stats.frames >= 1, "{name}: produced no frames");
        assert!(!stats.truncated, "{name}: untruncated run reported truncated");
        assert!(!out.is_empty(), "{name}: wrote no output");
    }
}

#[test]
fn truncated_run_ends_with_full_logo() {
    let text = splash_text();
    let mut session = SplashSession::new("decrypt", &text, test_opts()).expect("session builds");
    let mut out: Vec<u8> = Vec::new();
    let mut calls = 0;
    let stats = session
        .run(&mut out, || {
            calls += 1;
            calls > 3
        })
        .expect("run ok");
    assert!(stats.truncated);
    let rendered = String::from_utf8_lossy(&out);
    // The final paint must include every logo line and the version footer,
    // after the last animation frame (i.e. in the trailing portion).
    let tail = &rendered[rendered.len().saturating_sub(text.len() * 3)..];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        assert!(tail.contains(line.trim_end()), "final paint missing: {line:?}");
    }
    assert!(tail.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn play_swallows_engine_errors() {
    // Empty input makes ttfx's Terminal::new fail; the run path must surface
    // that as Err (never panic), which `play` then discards.
    let result = SplashSession::new("decrypt", "", test_opts());
    assert!(result.is_err(), "empty input should fail session build");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test splash_contract 2>&1 | tail -5`
Expected: FAIL — `SplashSession` not found.

- [ ] **Step 3: Implement in `src/splash.rs`**

Add:

```rust
use std::io::Write;

use eyre::{eyre, Result, WrapErr};
use ttfx::engine::ctx::{Clock, EngineCtx};
use ttfx::engine::effect::Effect;
use ttfx::engine::terminal::TerminalConfig;
use ttfx::utils::rng::Rng;

/// How long the animation may run before it is truncated to the final logo.
pub const SPLASH_DEADLINE: std::time::Duration = std::time::Duration::from_millis(1500);

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
    pub fn new(effect_name: &str, text: &str, opts: SessionOpts) -> Result<Self> {
        if text.trim().is_empty() {
            return Err(eyre!("splash input text is empty"));
        }
        // Default effect config via ttfx's own CLI, exactly like its
        // --random-effect path does.
        let parsed = <ttfx::cli::Cli as clap::Parser>::try_parse_from(["ttfx", effect_name])
            .map_err(|e| eyre!("unknown ttfx effect {effect_name}: {e}"))?;
        let effect = parsed
            .effect
            .ok_or_else(|| eyre!("ttfx parsed no effect for {effect_name}"))?
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
        Ok(SplashSession { effect, ctx, text: text.to_string(), rows })
    }

    /// Drive the effect to completion, `should_stop`, or error. Always ends by
    /// painting the plain input text into the canvas area (deterministic final
    /// state) and parking the cursor on the line below it.
    ///
    /// Raw-mode safe: rows are repositioned with `\r` + cursor-up instead of
    /// relying on cooked-mode `\n` (ttfx frames join rows with bare `\n`).
    pub fn run(&mut self, out: &mut impl Write, mut should_stop: impl FnMut() -> bool) -> Result<RunStats> {
        self.effect
            .build(&mut self.ctx)
            .map_err(|e| eyre!("ttfx effect build failed: {e:?}"))?;

        // Reserve the canvas: rows-1 newlines put the cursor at column 0 of
        // the bottom canvas row (invariant between frames).
        out.write_all("\r\n".repeat(self.rows.saturating_sub(1) as usize).as_bytes())
            .wrap_err("splash: reserve canvas")?;

        let mut stats = RunStats { frames: 0, truncated: false };
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
                self.ctx.terminal.recycle_output_string(frame);
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
```

Note: `clap` is already a direct dependency; `use clap::Parser` via the fully-qualified `<ttfx::cli::Cli as clap::Parser>` avoids an unused import when only used once.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test splash_contract 2>&1 | tail -5`
Expected: PASS (10 tests). If `curated_effects_produce_frames_on_virtual_clock` hangs, a curated effect loops forever on the virtual clock — drop that effect from `SPLASH_EFFECTS` (update the spec's decision note) rather than adding loop caps.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/splash.rs tests/splash_contract.rs
git commit -m "feat(splash): ttfx engine session with raw-mode-safe frame loop"
```

---

### Task 5: production `play()` + main.rs wiring + verification

**Files:**
- Modify: `src/splash.rs`
- Modify: `src/main.rs` (insert one call before `event_loop::run(cli)`)

**Interfaces:**
- Consumes: `SplashGate`/`should_play`, `pick_effect_name`, `SplashSession` (Tasks 3–4), `Cli.no_splash` (Task 2).
- Produces: `splash::play(cli: &octoscode::cli::Cli)` — the only symbol `main.rs` uses.

- [ ] **Step 1: Implement `play()` in `src/splash.rs`**

```rust
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
    let mut session = SplashSession::new(
        pick_effect_name(seed),
        &splash_text(),
        SessionOpts { frame_rate: 60, virtual_clock: false, seed },
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
    session.run(&mut stdout, || {
        std::time::Instant::now() >= deadline || key_or_resize_pending()
    })?;
    Ok(())
}

/// Drain pending terminal events; any key press or resize stops the splash.
/// Errors read as "stop" — never let event plumbing wedge the animation.
fn key_or_resize_pending() -> bool {
    use crossterm::event::{poll, read, Event, KeyEventKind};
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
```

- [ ] **Step 2: Wire into `src/main.rs`**

In `main()`, between `backend_ensure::ensure_octos_backend(&mut cli)?;` and `event_loop::run(cli)`:

```rust
    // Startup splash: ttfx-rendered logo on the main screen, before the event
    // loop claims the terminal. Gated (non-TTY/CI/--no-splash) and best-effort;
    // see specs/task-startup-splash.spec.
    octoscode::splash::play(&cli);
```

Update the import line to include `splash`... (it uses the crate path directly, so no import change is needed).

- [ ] **Step 3: Full check**

Run: `cargo fmt && cargo clippy --all-targets 2>&1 | tail -5 && cargo test 2>&1 | tail -5`
Expected: clippy clean, all tests pass (including the pre-existing suite).

- [ ] **Step 4: Manual verification (real terminal)**

Run: `cargo run -- --mode mock` in an interactive terminal.
Expected: logo animates below the prompt for ≤1.5s, then the full OCTOS logo + `octoscode v…` line stands still, then the TUI starts. Verify:
- pressing a key mid-animation jumps straight to the full logo + TUI;
- `cargo run -- --mode mock --no-splash` shows no animation;
- `CI=1 cargo run -- --mode mock` shows no animation;
- `cargo run -- --mode mock </dev/null | cat` (non-TTY) shows no animation;
- after quitting the TUI, the scrollback shows the intact logo banner and an intact shell prompt (no staircasing, cursor visible).

Repeat launches a few times to sample different random effects; if one reads badly when truncated, remove it from `SPLASH_EFFECTS` and note the change in the spec's decision line.

- [ ] **Step 5: Commit**

```bash
git add src/splash.rs src/main.rs
git commit -m "feat(splash): play ttfx startup animation before the event loop (specs/task-startup-splash.spec)"
```

---

## Self-Review Notes

- Spec coverage: 门控 5 scenarios → Task 3 tests; pick_effect → Task 3; CLI flag → Task 2; virtual-clock smoke + truncation + error-swallow → Task 4; deadline/key-skip/resize/raw-guard + main.rs mount → Task 5 (manual, per spec lint-ack).
- All spec test names match: `should_play_*` ×5, `pick_effect_stays_in_curated_list`, `curated_effects_produce_frames_on_virtual_clock`, `truncated_run_ends_with_full_logo`, `play_swallows_engine_errors`, `cli_parses_no_splash_flag`.
- Types consistent across tasks: `SplashGate`/`SessionOpts`/`RunStats`/`SplashSession` defined once, consumed by name.
