//! Contract tests for specs/task-startup-splash.spec.

use octoscode::splash::{
    SPLASH_EFFECTS, SplashGate, effect_args_for, pick_effect_args, should_play, splash_text,
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
    let gate = SplashGate {
        stdout_is_tty: false,
        ..open_gate()
    };
    assert!(!should_play(&gate));
}

#[test]
fn should_play_false_on_flag_or_env() {
    let gate = SplashGate {
        no_splash_flag: true,
        ..open_gate()
    };
    assert!(!should_play(&gate));
    let gate = SplashGate {
        env_disabled: true,
        ..open_gate()
    };
    assert!(!should_play(&gate));
}

#[test]
fn should_play_false_in_ci() {
    let gate = SplashGate {
        ci: true,
        ..open_gate()
    };
    assert!(!should_play(&gate));
}

#[test]
fn should_play_false_when_terminal_narrower_than_logo() {
    let gate = SplashGate {
        term_cols: 30,
        ..open_gate()
    };
    assert!(!should_play(&gate));
    let gate = SplashGate {
        term_rows: 5,
        ..open_gate()
    };
    assert!(!should_play(&gate));
}

#[test]
fn pick_effect_stays_in_curated_list() {
    for seed in 0..64u64 {
        let args = pick_effect_args(seed);
        assert!(
            SPLASH_EFFECTS.contains(&args),
            "seed {seed} picked {args:?}, not in SPLASH_EFFECTS"
        );
    }
}

#[test]
fn effect_pin_resolves_curated_names_only() {
    let matrix = effect_args_for("matrix").expect("matrix is curated");
    assert_eq!(matrix[0], "matrix");
    assert!(
        matrix.contains(&"--rain-time"),
        "pinned matrix must keep its tuned args"
    );
    assert!(
        effect_args_for("decrypt").is_none(),
        "non-curated name pins nothing"
    );
}

#[test]
fn splash_text_carries_logo_and_version() {
    let text = splash_text();
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    assert!(text.lines().count() >= 6, "logo should be multi-line");
}

use octoscode::splash::{SessionOpts, SplashSession};

fn test_opts() -> SessionOpts {
    SessionOpts {
        frame_rate: 0,
        virtual_clock: true,
        seed: 7,
    }
}

#[test]
fn curated_effects_produce_frames_on_virtual_clock() {
    for args in SPLASH_EFFECTS {
        let name = args[0];
        let mut session = SplashSession::new(args, &splash_text(), test_opts(), 80, String::new())
            .unwrap_or_else(|e| panic!("{name}: session builds: {e}"));
        let mut out: Vec<u8> = Vec::new();
        let stats = session
            .run(&mut out, || false)
            .unwrap_or_else(|e| panic!("{name}: run ok: {e}"));
        assert!(stats.frames >= 1, "{name}: produced no frames");
        assert!(
            !stats.truncated,
            "{name}: untruncated run reported truncated"
        );
        assert!(!out.is_empty(), "{name}: wrote no output");
    }
}

#[test]
fn truncated_run_ends_with_full_logo() {
    let text = splash_text();
    let mut session = SplashSession::new(&["decrypt"], &text, test_opts(), 80, String::new())
        .expect("session builds");
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
        assert!(
            tail.contains(line.trim_end()),
            "final paint missing: {line:?}"
        );
    }
    assert!(tail.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn play_swallows_engine_errors() {
    // Empty input makes the splash session build fail; the run path must
    // surface that as Err (never panic), which `play` then discards.
    let result = SplashSession::new(&["decrypt"], "", test_opts(), 80, String::new());
    assert!(result.is_err(), "empty input should fail session build");
}
