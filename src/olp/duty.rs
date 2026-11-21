//! The outer-duty lease check every writing command makes first (protocol
//! R7). `octoscode outer-duty hold` gives the outer agent, and what it runs,
//! `OLP_DUTY_LEASE`; a write carrying one goes through only while the
//! project's duty lock is held with that lease.

use std::path::Path;

use crate::outer_duty::{LeaseVerdict, verify_lease};

/// Exit code of a write refused for a dead or replaced outer session.
pub const EXIT_NO_LEASE: i32 = 6;

/// `Err(reason)` when the caller is an outer session whose lease is no
/// longer the current one. Without a lease in the environment (inner
/// loops, units, a person), or from another herdr pane than the outer's,
/// there is nothing to check.
pub fn check() -> Result<(), String> {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let lease = var("OLP_DUTY_LEASE");
    if lease.is_empty() || var("HERDR_PANE_ID") != var("OLP_DUTY_PANE") {
        return Ok(());
    }
    let project = var("OLP_DUTY_PROJECT");
    if project.is_empty() {
        return Err("环境里有 OLP_DUTY_LEASE,却没有 OLP_DUTY_PROJECT".to_string());
    }
    match verify_lease(Path::new(&project), &lease) {
        LeaseVerdict::Valid => Ok(()),
        LeaseVerdict::Vacant => Err("主审权锁空着:发出这份租约的外环会话已结束".to_string()),
        LeaseVerdict::Superseded => Err("主审权已由后来的外环会话持有".to_string()),
        LeaseVerdict::Error(why) => Err(why),
    }
}

/// [`check`], reported the way every tool reports it. `Err` is the exit code.
pub fn gate(prog: &str) -> Result<(), i32> {
    check().map_err(|reason| {
        eprintln!("{prog}: 主审权租约无效,拒绝写入({reason})");
        EXIT_NO_LEASE
    })
}
