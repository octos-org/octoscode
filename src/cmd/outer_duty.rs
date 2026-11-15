#![cfg(any(target_os = "linux", target_os = "macos"))]

//! `octoscode outer-duty` (OUTER_LOOP_REVIEW #38/#38-r1): hold/check
//! entrypoints over [`crate::outer_duty`]. Linux and macOS; other
//! platforms strip the kernel-adjudication module.

use super::OuterDutyArgs;
use crate::outer_duty::{self, DutyState};

/// Run the subcommand; returns the process exit code.
pub fn run(args: OuterDutyArgs) -> i32 {
    let project = std::path::PathBuf::from(&args.project);
    match args.action.as_str() {
        "check" => run_check(&project),
        "hold" => run_hold(&args, &project),
        "lease" => run_lease(&project),
        other => {
            eprintln!("outer-duty: unknown action {other:?} (expected hold|check|lease)");
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
    if let Some(refusal) = shared_server_refusal(&args.command) {
        eprintln!("outer-duty hold: {refusal}");
        return 2;
    }
    let hold = match outer_duty::acquire(project) {
        Ok(hold) => hold,
        Err(err) => {
            eprintln!("outer-duty hold: {err:#}");
            return 1;
        }
    };
    // Fencing lease: written before the agent starts, handed to it (and to
    // every command it runs) through the environment, with where to check
    // it. The process is still single-threaded here.
    let lease = match outer_duty::new_lease()
        .and_then(|lease| outer_duty::write_lease(&hold.lock_path, &lease).map(|()| lease))
    {
        Ok(lease) => lease,
        Err(err) => {
            eprintln!("outer-duty hold: {err:#}");
            return 1;
        }
    };
    let canonical = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("OLP_DUTY_LEASE", &lease);
        std::env::set_var("OLP_DUTY_PROJECT", &canonical);
        // The herdr pane the outer runs in ("" outside herdr): a writer in
        // another pane is not this outer, whatever it inherited.
        std::env::set_var(
            "OLP_DUTY_PANE",
            std::env::var("HERDR_PANE_ID").unwrap_or_default(),
        );
        if let Ok(exe) = std::env::current_exe() {
            std::env::set_var("OLP_DUTY_BIN", exe);
        }
    }
    // #38-r2: spawn first (the sidecar wants the child pid), guardian
    // death-coupled (Linux: setpgid + PDEATHSIG, wrapper sole fd holder;
    // macOS: a guardian process between wrapper and agent) — the wrapper
    // waits; child exit ends the wrapper and releases the lock.
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
    // The wrapper keeps its fd while waiting — every fd holder must die
    // before the lock is released.
    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("outer-duty hold: child wait failed: {err}");
            return 1;
        }
    };
    code_of(status)
}

/// `lease`: may a write carrying `OLP_DUTY_LEASE` proceed? stdout is one
/// token: VALID (exit 0) or REFUSED (exit 3, reason on stderr); no lease in
/// the environment is a usage error (exit 2).
fn run_lease(project: &std::path::Path) -> i32 {
    let Some(lease) = std::env::var("OLP_DUTY_LEASE")
        .ok()
        .filter(|l| !l.is_empty())
    else {
        eprintln!(
            "outer-duty lease: OLP_DUTY_LEASE is not set (only an agent started by `hold` has one)"
        );
        return 2;
    };
    let reason = match outer_duty::verify_lease(project, &lease) {
        outer_duty::LeaseVerdict::Valid => {
            println!("VALID");
            return 0;
        }
        outer_duty::LeaseVerdict::Vacant => {
            "the duty lock is VACANT: the session that issued this lease is gone".to_string()
        }
        outer_duty::LeaseVerdict::Superseded => {
            "the duty lock is HELD by a later session: this lease is superseded".to_string()
        }
        outer_duty::LeaseVerdict::Error(error) => error,
    };
    println!("REFUSED");
    eprintln!("outer-duty lease: {reason}");
    3
}

/// Codex keeps an interactive session in its shared background server
/// (`codex app-server daemon`) unless started with `--no-daemon`: the
/// terminal program is only a client, and when it goes away "any running
/// work continues" in the server, which is not a descendant of this
/// wrapper. The lock would then be released while that work still writes
/// (preflight 2026-11-14). Such a command is refused before the lock is
/// taken. `exec` and `review` run in the process itself.
fn shared_server_refusal(command: &[String]) -> Option<String> {
    let program = std::path::Path::new(&command[0]).file_name()?.to_str()?;
    if program != "codex" {
        return None;
    }
    let args = &command[1..];
    let in_process = |a: &String| {
        matches!(
            a.as_str(),
            "--no-daemon" | "exec" | "e" | "review" | "-h" | "--help" | "-V" | "--version"
        )
    };
    if args.iter().any(in_process) {
        return None;
    }
    Some(format!(
        "codex without --no-daemon runs the session in its shared background server, \
         which keeps working after this wrapper is gone, so the duty lock cannot bind it; \
         start it as `{} --no-daemon{}`",
        command[0],
        args.iter().map(|a| format!(" {a}")).collect::<String>()
    ))
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
