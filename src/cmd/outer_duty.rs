#![cfg(target_os = "linux")]

//! `octoscode outer-duty` (OUTER_LOOP_REVIEW #38/#38-r1): hold/check
//! entrypoints over [`crate::outer_duty`]. Linux-only (#38-r1 adjudication; macOS builds strip the kernel-adjudication module).

use super::OuterDutyArgs;
use crate::outer_duty::{self, DutyState};

/// Run the subcommand; returns the process exit code.
pub fn run(args: OuterDutyArgs) -> i32 {
    let project = std::path::PathBuf::from(&args.project);
    match args.action.as_str() {
        "check" => run_check(&project),
        "hold" => run_hold(&args, &project),
        other => {
            eprintln!("outer-duty: unknown action {other:?} (expected hold|check)");
            2
        }
    }
}

/// stdout carries the state token ALONE — exactly one line, machine
/// parseable; diagnostics go to stderr as one JSON line (#38-r1 C).
fn run_check(project: &std::path::Path) -> i32 {
    match outer_duty::check(project) {
        DutyState::Vacant => {
            println!("VACANT");
            0
        }
        DutyState::Held => {
            let diag = outer_duty::lock_path(project)
                .map(|p| outer_duty::held_diagnostics(&p))
                .unwrap_or_else(|_| "{\"diagnostics\":\"unavailable\"}".into());
            eprintln!("{diag}");
            println!("HELD");
            0
        }
        DutyState::Error => {
            println!("ERROR");
            1
        }
    }
}

fn run_hold(args: &OuterDutyArgs, project: &std::path::Path) -> i32 {
    if args.command.is_empty() {
        eprintln!("outer-duty hold: a child command after `--` is required");
        return 2;
    }
    let hold = match outer_duty::acquire(project) {
        Ok(hold) => hold,
        Err(err) => {
            eprintln!("outer-duty hold: {err:#}");
            return 1;
        }
    };
    // #38-r2: spawn first (the sidecar wants the child pid), guardian
    // death-coupled (setpgid + PDEATHSIG) — the wrapper is the sole fd
    // holder and waits; child exit ends the wrapper and releases the lock.
    let mut child = match outer_duty::spawn_holder_child(&args.command) {
        Ok(child) => child,
        Err(err) => {
            eprintln!("outer-duty hold: {err:#}");
            return 1;
        }
    };
    // Diagnostic sidecar (best-effort; corruption never affects the lock);
    // wrapper+child PID & starttime give operators a reuse-proof locator.
    let _ = outer_duty::write_metadata(
        &hold.lock_path,
        &args.signature,
        &args.duties,
        Some(child.id()),
    );
    // The wrapper keeps its fd while waiting — both holders must die before
    // the lock is released.
    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("outer-duty hold: child wait failed: {err}");
            return 1;
        }
    };
    code_of(status)
}

#[cfg(unix)]
fn code_of(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

#[cfg(not(unix))]
fn code_of(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
