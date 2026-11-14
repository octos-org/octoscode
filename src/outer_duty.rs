//! `octoscode outer-duty` (OUTER_LOOP_REVIEW #38 / #38-r1): the kernel lock
//! behind multi-outer primary-reviewer authority — a per-project,
//! session-lifetime OS-exclusive lock. Linux and macOS (single-machine
//! `flock`; NFS out of scope); other platforms route the subcommand to an
//! explicit unsupported error (see `src/cmd/mod.rs`); Windows LockFileEx
//! support is a separate follow-up entry. The module does not compile on
//! unsupported targets by design (honest shrink, per the countersign).
//!
//! Lifecycle binding (#38-r2 adjudicated: GUARDIAN death coupling) — the
//! invariant on every supported platform is "wrapper dead ⇒ agent dead,
//! and the lock is never VACANT while the agent is alive":
//!
//! - Linux: the wrapper is the sole fd holder (CLOEXEC stays set — the
//!   lock never leaks to descendants); the child gets setpgid +
//!   PR_SET_PDEATHSIG(SIGKILL), so the kernel kills the agent the moment
//!   the wrapper dies and the authority is released immediately.
//! - macOS: there is no PDEATHSIG. The wrapper forks a tiny GUARDIAN
//!   (reaper) process which inherits the lock fd and is the agent's
//!   parent: `wrapper → guardian → agent`. The guardian watches the
//!   wrapper (kqueue EVFILT_PROC + a getppid poll); when the wrapper dies
//!   it SIGKILLs the agent's process group and exits only after the group
//!   is gone, so the lock stays HELD until the agent is provably dead.
//!   The wrapper in turn watches the guardian: a guardian that dies
//!   abnormally makes the wrapper kill the agent group and fail. The agent
//!   itself never holds the fd (CLOEXEC). Boundary: this survives the
//!   death of either supervisor, not a simultaneous SIGKILL of both.
//!
//! Grandchildren hold no fd: an agent that exits while leaving a
//! grandchild running yields VACANT. These invariants are contract-pinned
//! with real processes on both platforms.
//!
//! Terminal (preflight 2026-11-14: an interactive agent under `hold` was
//! stopped by SIGTTIN): the agent's own process group is not the
//! terminal's foreground, so a TUI reading the terminal stops. When the
//! wrapper starts as the foreground of its terminal, the agent makes its
//! group the foreground before exec, and the wrapper takes the terminal
//! back once the agent is gone. A ^Z that stops the agent stops the
//! wrapper too (the shell sees an ordinary stopped job); on `fg` the
//! terminal goes back to the agent and the agent is continued. A wrapper
//! started in the background, or without a terminal, changes nothing.
//!
//! Lock naming is a STABLE protocol: SHA-256 over the domain-prefixed
//! canonical project path (DefaultHasher is not stable across Rust versions
//! — protocol-documented limitation). HOME missing ⇒ fail-closed error.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::io::Write as _;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, eyre};

/// One of exactly three machine-readable states `check` may print — the
/// stdout line is the state token ALONE (single line, machine-parseable);
/// diagnostics (holder signature, corruption notes) go to stderr as JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DutyState {
    /// No live fd holder.
    Vacant,
    /// At least one live fd holder (wrapper or the inherited child).
    Held,
    /// I/O or structural error — NEVER masqueraded as VACANT.
    Error,
}

impl DutyState {
    pub fn as_str(self) -> &'static str {
        match self {
            DutyState::Vacant => "VACANT",
            DutyState::Held => "HELD",
            DutyState::Error => "ERROR",
        }
    }
}

/// Lock-name domain prefix — separation of namespace from any other
/// consumer of the same directory.
const LOCK_DOMAIN: &str = "octoscode/outer-duty/v1";

/// How long acquire() retries past a contention that may be a transient
/// check-probe collision (probes unlock in microseconds).
const PROBE_COLLISION_RETRY_MS: u64 = 2_000;

/// Canonicalize `--project` and derive the stable lock path:
/// `~/.octos/outer/duty/<sha256(domain + "\0" + canonical)>.lock`.
pub fn lock_path(project: &Path) -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| {
        eyre!("outer-duty: HOME is not set — fail-closed (refusing to guess a lock root)")
    })?;
    if home.is_empty() {
        return Err(eyre!("outer-duty: HOME is empty — fail-closed"));
    }
    let canonical = std::fs::canonicalize(project)
        .wrap_err_with(|| format!("cannot canonicalize project path: {}", project.display()))?;
    let digest = lock_digest(canonical.to_string_lossy().as_bytes());
    Ok(Path::new(&home)
        .join(".octos")
        .join("outer")
        .join("duty")
        .join(format!("{digest}.lock")))
}

/// Pure, stable lock-name digest: SHA-256 over
/// `LOCK_DOMAIN ++ [0] ++ canonical_path_bytes` → 64 lowercase hex chars.
/// Golden-pinned by a fixed input in tests (NOT a mirror of this code).
pub fn lock_digest(canonical_path: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(LOCK_DOMAIN.as_bytes());
    hasher.update([0u8]);
    hasher.update(canonical_path);
    let bytes = hasher.finalize();
    let mut digest = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(digest, "{byte:02x}");
    }
    digest
}

/// Hold guard: the fd IS the lock.
pub struct DutyHold {
    pub file: std::fs::File,
    pub lock_path: PathBuf,
}

fn open_lock_file(path: &Path, mode: u32) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(mode)
        .open(path)
}

/// Tighten permissions on a possibly pre-existing (e.g. 0644) file/dir so a
/// permissive umask or an old artifact cannot leave wide-open state behind.
/// Fail-closed: a failed tightening is an ERROR, not silently swallowed.
/// #38-r8: the seam is PRIVATE — `tighten_with` takes the chmod operation
/// as a parameter (production passes the real chmod; unit tests under
/// cfg(test) pass a failing closure). No global/public hook remains.
fn tighten(path: &Path, mode: u32) -> Result<()> {
    tighten_with(path, mode, real_tighten)
}

/// The parameterized boundary: apply `op` unless a `cfg(test)` injection
/// replaces it (test-only thread-local, invisible in production builds).
fn tighten_with(path: &Path, mode: u32, op: fn(&Path, u32) -> Result<()>) -> Result<()> {
    #[cfg(test)]
    {
        if let Some(injected) = TIGHTEN_TEST_OVERRIDE.with(|slot| *slot.borrow()) {
            return injected(path, mode);
        }
    }
    op(path, mode)
}

// Test-only injection point (cfg(test): absent from production builds).
#[cfg(test)]
type TightenOp = fn(&Path, u32) -> Result<()>;

#[cfg(test)]
thread_local! {
    static TIGHTEN_TEST_OVERRIDE: std::cell::RefCell<Option<TightenOp>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_tighten_test_override(f: Option<TightenOp>) {
    TIGHTEN_TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = f);
}

/// The production chmod.
fn real_tighten(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .wrap_err_with(|| format!("failed to tighten {} to {:o}", path.display(), mode))
}

/// Acquire the duty lock NONBLOCKING, or fail structurally on contention.
pub fn acquire(project: &Path) -> Result<DutyHold> {
    let path = lock_path(project)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("failed to create duty lock dir: {}", parent.display()))?;
        tighten(parent, 0o700)?;
    }
    let file = open_lock_file(&path, 0o600)
        .wrap_err_with(|| format!("failed to open duty lockfile: {}", path.display()))?;
    tighten(&path, 0o600)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(DutyHold {
            file,
            lock_path: path,
        }),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            // A concurrently-running CHECK probe holds the lock for
            // microseconds (it unlocks immediately on its Vacant verdict).
            // Retry the acquire briefly so a probe collision — transient by
            // construction — is never misreported as a live-holder conflict.
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_millis(PROBE_COLLISION_RETRY_MS);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(20));
                match fs2::FileExt::try_lock_exclusive(&file) {
                    Ok(()) => {
                        return Ok(DutyHold {
                            file,
                            lock_path: path,
                        });
                    }
                    Err(retry)
                        if retry.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                            && std::time::Instant::now() < deadline =>
                    {
                        continue;
                    }
                    Err(_) => {
                        return Err(eyre!(
                            "outer-duty: HELD by another live holder for this project ({}) — \
                             no agent self-takeover: the operator terminates the old holder, \
                             then a fresh acquire",
                            path.display()
                        ));
                    }
                }
            }
        }
        Err(error) => Err(eyre::Report::new(error)
            .wrap_err(format!("failed to acquire duty lock: {}", path.display()))),
    }
}

/// Probe the lock WITHOUT disturbing a live holder: open the same file and
/// `try_lock_exclusive` — contention = Held, success = Vacant (probe fd
/// drops immediately). No `path.exists()` TOCTOU branch (#38-r1 C): a
/// missing parent is Error; a missing lock FILE is simply Vacant via the
/// same create-open path (open errors are Error, never VACANT).
pub fn check(project: &Path) -> DutyState {
    let path = match lock_path(project) {
        Ok(path) => path,
        Err(_) => return DutyState::Error,
    };
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return DutyState::Error;
    }
    let file = match open_lock_file(&path, 0o600) {
        Ok(file) => file,
        Err(_) => return DutyState::Error,
    };
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            DutyState::Vacant
        }
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            DutyState::Held
        }
        Err(_) => DutyState::Error,
    }
}

/// Sanitize a metadata field for the stderr diagnostic line: strip control
/// characters (incl. newlines/ANSI) and cap length.
fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace() || *c == ' ')
        .take(120)
        .collect()
}

/// Write the diagnostic metadata sidecar atomically: unique 0600 tempfile +
/// file fsync + rename (unique names so concurrent diagnostic writes cannot
/// collide; corruption never affects adjudication).
pub fn write_metadata(
    lock: &Path,
    signature: &str,
    duties: &str,
    child_pid: Option<u32>,
) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let sidecar = lock.with_extension("meta");
    let payload = format!(
        "{{\"signature\":{:?},\"duties\":{:?},\"written_at_unix\":{},\"wrapper_pid\":{},\"wrapper_starttime\":{}}}\n",
        sanitize_field(signature),
        sanitize_field(duties),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        std::process::id(),
        process_starttime(std::process::id()),
    )
    .trim_end_matches('\n')
    .to_string()
    .replace(
        "}",
        &format!(
            ",\"child_pid\":{},\"child_starttime\":{}}}",
            child_pid.map(|p| p.to_string()).unwrap_or("null".into()),
            child_pid.map(process_starttime).unwrap_or(0),
        ),
    )
    + "\n";
    if let Some(parent) = sidecar.parent() {
        std::fs::create_dir_all(parent)?;
        tighten(parent, 0o700)?;
    }
    let tmp = sidecar.with_extension(format!("meta.tmp.{}", std::process::id()));
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(payload.as_bytes())?;
        file.sync_all()?;
    }
    tighten(&tmp, 0o600)?;
    std::fs::rename(&tmp, &sidecar)
        .wrap_err_with(|| format!("failed to rename metadata sidecar: {}", sidecar.display()))
}

/// Parse the sidecar for diagnostics; `None` = unreadable/corrupt
/// (METADATA_CORRUPT — adjudication unaffected).
pub fn read_metadata(lock: &Path) -> Option<serde_json::Value> {
    let sidecar = lock.with_extension("meta");
    let text = std::fs::read_to_string(sidecar).ok()?;
    serde_json::from_str(&text).ok()
}

/// The sanitized one-line stderr diagnostic for a HELD probe.
pub fn held_diagnostics(lock: &Path) -> String {
    match read_metadata(lock) {
        Some(meta) => format!(
            "{{\"holder\":{:?},\"duties\":{:?}}}",
            sanitize_field(
                meta.get("signature")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            ),
            sanitize_field(meta.get("duties").and_then(|v| v.as_str()).unwrap_or("?")),
        ),
        None => "{\"diagnostics\":\"METADATA_CORRUPT\"}".into(),
    }
}

/// The duty-wrapped agent as the wrapper sees it. `id()` is the REAL
/// agent pid on every platform (what the diagnostic sidecar records);
/// `wait()` blocks until the agent is gone and yields its exit status.
pub struct HolderChild {
    #[cfg(target_os = "linux")]
    child: std::process::Child,
    #[cfg(target_os = "macos")]
    guardian: libc::pid_t,
    #[cfg(target_os = "macos")]
    agent: libc::pid_t,
    /// The terminal handed to the agent (None: wrapper not in the
    /// foreground of a terminal).
    terminal: Option<terminal::Foreground>,
}

impl HolderChild {
    /// The real agent pid.
    pub fn id(&self) -> u32 {
        #[cfg(target_os = "linux")]
        {
            self.child.id()
        }
        #[cfg(target_os = "macos")]
        {
            self.agent as u32
        }
    }

    /// Wait for the agent. Linux: the direct child. macOS: the guardian,
    /// which relays the agent's exit code (128+signal for a signal death);
    /// a guardian that itself dies by a signal is a supervision failure —
    /// the agent group is killed and an error is returned (the wrapper
    /// still holds the lock while that happens).
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        #[cfg(target_os = "linux")]
        let status = match self.terminal {
            // Interactive: also see the agent stop, to relay ^Z.
            Some(term) => terminal::wait_relaying_stops(self.child.id() as libc::pid_t, term),
            None => self.child.wait(),
        };
        #[cfg(target_os = "macos")]
        let status = macos::wait_guardian(self.guardian, self.agent);
        if let Some(term) = self.terminal {
            terminal::reclaim(term);
        }
        status
    }
}

/// Foreground hand-over between the wrapper and an interactive agent.
#[allow(unsafe_code)]
mod terminal {
    /// A terminal and the wrapper's process group, which was its
    /// foreground when `hold` started.
    #[derive(Clone, Copy)]
    pub(super) struct Foreground {
        pub(super) fd: libc::c_int,
        pub(super) pgrp: libc::pid_t,
    }

    /// The first of stdin/stdout/stderr that is a terminal whose
    /// foreground is the wrapper's own process group.
    pub(super) fn capture() -> Option<Foreground> {
        unsafe {
            let pgrp = libc::getpgrp();
            (0..3)
                .find(|&fd| libc::isatty(fd) == 1 && libc::tcgetpgrp(fd) == pgrp)
                .map(|fd| Foreground { fd, pgrp })
        }
    }

    /// Make `pgrp` the terminal's foreground. Async-signal-safe (it runs
    /// between fork and exec): SIGTTOU is ignored around the call, since
    /// a caller outside the foreground would otherwise be stopped by it.
    pub(super) unsafe fn give(fd: libc::c_int, pgrp: libc::pid_t) {
        unsafe {
            let mut ignore: libc::sigaction = std::mem::zeroed();
            ignore.sa_sigaction = libc::SIG_IGN;
            let mut previous: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTTOU, &ignore, &mut previous);
            libc::tcsetpgrp(fd, pgrp);
            libc::sigaction(libc::SIGTTOU, &previous, std::ptr::null_mut());
        }
    }

    /// The agent is gone: give the terminal back to the wrapper's group.
    pub(super) fn reclaim(term: Foreground) {
        unsafe {
            if libc::tcgetpgrp(term.fd) != term.pgrp {
                give(term.fd, term.pgrp);
            }
        }
    }

    /// Relay a stopped agent to the wrapper's own job: take the terminal
    /// back, stop the wrapper's group like the shell expects of a ^Z,
    /// and once continued (`fg` gives the terminal to the wrapper's
    /// group) hand the terminal to the agent again and continue it.
    pub(super) unsafe fn relay_stop(term: Foreground, agent: libc::pid_t, stop: impl FnOnce()) {
        unsafe {
            give(term.fd, term.pgrp);
            stop();
            if libc::tcgetpgrp(term.fd) == term.pgrp {
                give(term.fd, agent);
            }
            libc::killpg(agent, libc::SIGCONT);
        }
    }

    /// Linux: the wrapper is the agent's parent. Wait for it to exit,
    /// relaying every stop.
    #[cfg(target_os = "linux")]
    pub(super) fn wait_relaying_stops(
        agent: libc::pid_t,
        term: Foreground,
    ) -> std::io::Result<std::process::ExitStatus> {
        use std::os::unix::process::ExitStatusExt as _;
        loop {
            let mut status = 0;
            if unsafe { libc::waitpid(agent, &mut status, libc::WUNTRACED) } == -1 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if !libc::WIFSTOPPED(status) {
                return Ok(std::process::ExitStatus::from_raw(status));
            }
            // SIGSTOP to ourselves returns only once we are continued.
            unsafe {
                relay_stop(term, agent, || {
                    libc::kill(libc::getpid(), libc::SIGSTOP);
                });
            }
        }
    }
}

/// Spawn the duty-wrapped child with GUARDIAN death coupling (#38-r2
/// adjudicated design): the wrapper is the ONLY fd holder (CLOEXEC stays
/// set — the lock never leaks to descendants); the child runs in its own
/// process group with PR_SET_PDEATHSIG(SIGKILL) — when the wrapper dies,
/// the agent dies with it, releasing the authority immediately (no
/// split brain). The wrapper then waits on the child: when the agent
/// exits (normally or not), the wrapper exits and the lock releases.
///
/// PDEATHSIG semantics boundary: the signal is delivered when the child's
/// *parent thread* dies; it is Linux-specific and does NOT chain to
/// grandchildren — a grandchild that outlives its parent holds no fd and
/// no authority (pinned by contract).
#[cfg(target_os = "linux")]
pub fn spawn_holder_child(command: &[String]) -> Result<HolderChild> {
    use std::os::unix::process::CommandExt as _;
    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    let terminal = terminal::capture();
    #[allow(unsafe_code)]
    unsafe {
        // The child's parent must be THIS wrapper: capture our own pid,
        // not our parent's (the fork child's getppid() == wrapper pid).
        let expected_parent = std::process::id();
        cmd.pre_exec(move || {
            // Own process group: signals to the wrapper's group (e.g. an
            // interactive ^C) do not reach the duty child directly.
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Interactive: the agent's group becomes the foreground, so a
            // TUI reading the terminal is not stopped by SIGTTIN.
            if let Some(term) = terminal {
                terminal::give(term.fd, libc::getpgrp());
            }
            // Death coupling: wrapper dies ⇒ agent gets SIGKILL.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Classic PDEATHSIG install race: if the wrapper died between
            // fork and prctl, we were reparented and the signal will never
            // fire. Verify the parent is still the wrapper — else fail the
            // exec (the wrapper's exit already released the lock; an
            // orphaned agent must not outlive its authority).
            if libc::getppid() != expected_parent as libc::pid_t {
                return Err(std::io::Error::other(
                    "duty child's parent changed before PDEATHSIG install",
                ));
            }
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .wrap_err_with(|| format!("failed to spawn duty child: {}", command[0]))?;
    Ok(HolderChild { child, terminal })
}

/// macOS spawn: `wrapper → guardian → agent` (see the module docs). The
/// guardian is a fork WITHOUT exec, so it inherits the lock fd (same open
/// file description ⇒ same flock) and keeps the authority HELD until the
/// agent's process group is gone. Must be called before any thread is
/// started (the `outer-duty` subcommand dispatches first thing in `main`).
#[cfg(target_os = "macos")]
pub fn spawn_holder_child(command: &[String]) -> Result<HolderChild> {
    let terminal = terminal::capture();
    let (guardian, agent) = macos::spawn_guarded(command, terminal)?;
    Ok(HolderChild {
        guardian,
        agent,
        terminal,
    })
}

/// /proc/<pid>/stat field 22 (starttime in clock ticks since boot) — a
/// PID-reuse-proof locator for operators; 0 when unreadable.
#[cfg(target_os = "linux")]
fn process_starttime(pid: u32) -> u64 {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(_) => return 0,
    };
    // field 22; comm may contain spaces/parens — parse after the last ')'.
    let rest = stat.rsplit(')').next().unwrap_or("");
    rest.split_whitespace()
        .nth(19)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0)
}

/// Process start time in microseconds since the Unix epoch
/// (`proc_pidinfo(PROC_PIDTBSDINFO)`) — the macOS PID-reuse-proof locator
/// for operators; 0 when unreadable.
#[cfg(target_os = "macos")]
pub fn process_starttime(pid: u32) -> u64 {
    #[allow(unsafe_code)]
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let read = libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        );
        if read != size {
            return 0;
        }
        info.pbi_start_tvsec
            .saturating_mul(1_000_000)
            .saturating_add(info.pbi_start_tvusec)
    }
}

/// macOS guardian (reaper) implementation. Everything that runs after
/// `fork()` uses raw libc calls only — no allocation, no std I/O.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use super::terminal::{self, Foreground};
    use eyre::{Result, WrapErr, eyre};
    use std::ffi::CString;

    /// How often the guardian re-checks its parent when no kqueue event
    /// arrives (belt and braces next to EVFILT_PROC).
    const POLL_MS: libc::c_long = 100;
    /// Upper bound on waiting for a SIGKILLed agent group to disappear
    /// before the lock is released anyway (never a permanent stuck lock).
    const GROUP_GONE_CAP_MS: u32 = 30_000;
    /// Signals the guardian ignores so a terminal ^C / hangup aimed at the
    /// wrapper cannot take both supervisors down at once.
    const GUARDED_SIGNALS: [libc::c_int; 4] =
        [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];
    /// Upper bound on waiting for a SIGSTOP sent to the wrapper to land.
    const STOP_LAND_CAP_MS: u32 = 1_000;

    /// Fork the guardian (which forks + execs the agent). Returns
    /// `(guardian_pid, agent_pid)`.
    pub(super) fn spawn_guarded(
        command: &[String],
        terminal: Option<Foreground>,
    ) -> Result<(libc::pid_t, libc::pid_t)> {
        // Everything the post-fork code needs is prepared up front.
        let args: Vec<CString> = command
            .iter()
            .map(|arg| CString::new(arg.as_bytes()))
            .collect::<std::result::Result<_, _>>()
            .wrap_err("outer-duty: child command contains a NUL byte")?;
        let mut argv: Vec<*const libc::c_char> = args.iter().map(|arg| arg.as_ptr()).collect();
        argv.push(std::ptr::null());

        let mut fds = [0 as libc::c_int; 2];
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) == -1 {
                return Err(std::io::Error::last_os_error())
                    .wrap_err("outer-duty: failed to create the guardian pipe");
            }
            // The agent must not inherit either end.
            libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);

            let wrapper = libc::getpid();
            let guardian = libc::fork();
            if guardian == -1 {
                let error = std::io::Error::last_os_error();
                libc::close(fds[0]);
                libc::close(fds[1]);
                return Err(error).wrap_err("outer-duty: failed to fork the guardian");
            }
            if guardian == 0 {
                libc::close(fds[0]);
                guardian_main(wrapper, fds[1], argv.as_ptr(), terminal);
            }
            libc::close(fds[1]);
            let agent = read_pid(fds[0]);
            libc::close(fds[0]);
            match agent {
                Some(agent) => Ok((guardian, agent)),
                None => {
                    let mut status = 0;
                    waitpid_retry(guardian, &mut status);
                    Err(eyre!(
                        "failed to spawn duty child: {} (guardian exited before starting it)",
                        command[0]
                    ))
                }
            }
        }
    }

    /// Wrapper side: wait for the guardian and classify its end.
    pub(super) fn wait_guardian(
        guardian: libc::pid_t,
        agent: libc::pid_t,
    ) -> std::io::Result<std::process::ExitStatus> {
        use std::os::unix::process::ExitStatusExt as _;
        let mut status = 0;
        unsafe {
            if !waitpid_retry(guardian, &mut status) {
                return Err(std::io::Error::last_os_error());
            }
            if libc::WIFEXITED(status) {
                return Ok(std::process::ExitStatus::from_raw(status));
            }
            // The guardian was killed: the agent lost its supervisor. We
            // still hold the lock — kill the agent group before failing so
            // the authority is never released over a live agent.
            libc::killpg(agent, libc::SIGKILL);
            wait_group_gone(agent);
        }
        Err(std::io::Error::other(
            "the duty guardian died abnormally; the agent group was killed",
        ))
    }

    /// The guardian process body. Never returns.
    unsafe fn guardian_main(
        wrapper: libc::pid_t,
        report_fd: libc::c_int,
        argv: *const *const libc::c_char,
        terminal: Option<Foreground>,
    ) -> ! {
        unsafe {
            // Own process group: signals aimed at the wrapper's group do
            // not reach the guardian.
            libc::setpgid(0, 0);
            let mut previous: [libc::sigaction; 4] = std::mem::zeroed();
            let mut ignore: libc::sigaction = std::mem::zeroed();
            ignore.sa_sigaction = libc::SIG_IGN;
            for (index, signal) in GUARDED_SIGNALS.iter().enumerate() {
                libc::sigaction(*signal, &ignore, &mut previous[index]);
            }

            let kq = libc::kqueue();
            watch_exit(kq, wrapper);
            // The wrapper may already be gone (fork race): no agent yet,
            // nothing to guard — the lock is released by our exit.
            if libc::getppid() != wrapper {
                libc::_exit(1);
            }

            let agent = libc::fork();
            if agent == -1 {
                libc::_exit(1);
            }
            if agent == 0 {
                // Own process group (its id is the agent pid), original
                // signal dispositions, then exec. CLOEXEC drops the lock
                // fd and the report pipe.
                libc::setpgid(0, 0);
                // Interactive: the agent's group becomes the foreground,
                // so a TUI reading the terminal is not stopped by SIGTTIN.
                if let Some(term) = terminal {
                    terminal::give(term.fd, libc::getpgrp());
                }
                for (index, signal) in GUARDED_SIGNALS.iter().enumerate() {
                    libc::sigaction(*signal, &previous[index], std::ptr::null_mut());
                }
                libc::execvp(*argv, argv);
                let message = b"outer-duty hold: failed to exec the duty child\n";
                libc::write(2, message.as_ptr().cast(), message.len());
                libc::_exit(127);
            }
            // Set the group from this side too, so it exists before the
            // first kill regardless of scheduling.
            libc::setpgid(agent, agent);
            let bytes = agent.to_ne_bytes();
            libc::write(report_fd, bytes.as_ptr().cast(), bytes.len());
            libc::close(report_fd);
            watch_exit(kq, agent);

            // Interactive: also see the agent stop, to relay ^Z.
            let flags = match terminal {
                Some(_) => libc::WNOHANG | libc::WUNTRACED,
                None => libc::WNOHANG,
            };
            loop {
                let mut status = 0;
                let reaped = libc::waitpid(agent, &mut status, flags);
                if reaped == agent && libc::WIFSTOPPED(status) {
                    if let Some(term) = terminal {
                        terminal::relay_stop(term, agent, || stop_wrapper(kq, wrapper));
                    }
                    continue;
                }
                if reaped == agent {
                    // Agent finished on its own: relay its code. Lingering
                    // grandchildren hold no fd and are left alone.
                    libc::_exit(exit_code(status));
                }
                if libc::getppid() != wrapper {
                    // Wrapper died: kill the agent group, and release the
                    // lock (by exiting) only once it is gone.
                    libc::killpg(agent, libc::SIGKILL);
                    waitpid_retry(agent, &mut status);
                    wait_group_gone(agent);
                    libc::_exit(128 + libc::SIGKILL);
                }
                wait_event(kq);
            }
        }
    }

    /// Stop the wrapper (the shell sees its job stopped) and return once
    /// it runs again or is gone. The guardian is not the wrapper's
    /// parent, so it watches the wrapper's state.
    unsafe fn stop_wrapper(kq: libc::c_int, wrapper: libc::pid_t) {
        unsafe {
            libc::kill(wrapper, libc::SIGSTOP);
            // First see the stop land (bounded), then wait for SIGCONT.
            let mut waited_ms = 0u32;
            while !is_stopped(wrapper) && waited_ms < STOP_LAND_CAP_MS {
                libc::usleep(10_000);
                waited_ms += 10;
            }
            while libc::getppid() == wrapper && is_stopped(wrapper) {
                wait_event(kq);
            }
        }
    }

    /// Whether `pid` is stopped (false when it cannot be read).
    unsafe fn is_stopped(pid: libc::pid_t) -> bool {
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            ) == size
                && info.pbi_status == libc::SSTOP
        }
    }

    /// Register a one-shot NOTE_EXIT watch (best effort: the poll below
    /// covers a failed registration).
    unsafe fn watch_exit(kq: libc::c_int, pid: libc::pid_t) {
        if kq == -1 {
            return;
        }
        unsafe {
            let mut change: libc::kevent = std::mem::zeroed();
            change.ident = pid as libc::uintptr_t;
            change.filter = libc::EVFILT_PROC;
            change.flags = libc::EV_ADD | libc::EV_ONESHOT;
            change.fflags = libc::NOTE_EXIT;
            libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null());
        }
    }

    /// Sleep until a watched process exits or the poll interval elapses.
    unsafe fn wait_event(kq: libc::c_int) {
        unsafe {
            if kq == -1 {
                libc::usleep((POLL_MS as libc::useconds_t) * 1000);
                return;
            }
            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: POLL_MS * 1_000_000,
            };
            let mut event: libc::kevent = std::mem::zeroed();
            libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, &timeout);
        }
    }

    /// Block until the process group has no live member (re-sending
    /// SIGKILL), capped so the lock can never stay stuck forever.
    unsafe fn wait_group_gone(group: libc::pid_t) {
        unsafe {
            let mut waited_ms = 0u32;
            while waited_ms < GROUP_GONE_CAP_MS {
                if libc::killpg(group, libc::SIGKILL) == -1 {
                    return; // ESRCH: nobody left (EPERM cannot be ours).
                }
                libc::usleep(10_000);
                waited_ms += 10;
            }
        }
    }

    /// `waitpid` retried across EINTR; false on any other failure.
    unsafe fn waitpid_retry(pid: libc::pid_t, status: &mut libc::c_int) -> bool {
        unsafe {
            loop {
                if libc::waitpid(pid, status, 0) == pid {
                    return true;
                }
                if *libc::__error() != libc::EINTR {
                    return false;
                }
            }
        }
    }

    /// Shell-convention exit code for a wait status.
    fn exit_code(status: libc::c_int) -> libc::c_int {
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            128 + libc::WTERMSIG(status)
        }
    }

    /// Read one native-endian pid from the report pipe (None on EOF or a
    /// short read).
    unsafe fn read_pid(fd: libc::c_int) -> Option<libc::pid_t> {
        let mut bytes = [0u8; std::mem::size_of::<libc::pid_t>()];
        let mut filled = 0usize;
        unsafe {
            while filled < bytes.len() {
                let read = libc::read(
                    fd,
                    bytes.as_mut_ptr().add(filled).cast(),
                    bytes.len() - filled,
                );
                if read > 0 {
                    filled += read as usize;
                } else if read == -1 && *libc::__error() == libc::EINTR {
                    continue;
                } else {
                    return None;
                }
            }
        }
        Some(libc::pid_t::from_ne_bytes(bytes))
    }
}

#[cfg(test)]
mod tighten_boundary_tests {
    use super::*;

    /// #38-r8 ①: the boundary unit test lives WITH the seam (cfg(test)):
    /// inject an always-EPERM closure and prove (a) execution
    /// reachability, (b) error context carries the EXACT path and mode,
    /// (c) the production default is the real chmod.
    #[test]
    fn tighten_with_injected_eperm_propagates_exact_path() {
        use std::cell::Cell;
        thread_local! {
            static CALLS: Cell<u32> = const { Cell::new(0) };
        }
        fn injected(path: &Path, mode: u32) -> Result<()> {
            CALLS.with(|c| c.set(c.get() + 1));
            Err(eyre::eyre!(
                "failed to tighten {} to {:o} (injected EPERM)",
                path.display(),
                mode
            ))
        }
        set_tighten_test_override(Some(injected));
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                set_tighten_test_override(None);
            }
        }
        let _g = Guard;

        let tmp = std::env::temp_dir().join(format!("tighten-b-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // ② precision: the dir tightening pins EXACTLY 0700...
        let err = match tighten_with(&tmp, 0o700, real_tighten) {
            Err(report) => format!("{report:#}"),
            Ok(()) => panic!("injected EPERM must fail"),
        };
        let calls = CALLS.with(|c| c.get());
        assert!(calls > 0, "execution reachability: seam called {calls}x");
        assert!(err.contains("failed to tighten"), "context: {err}");
        assert!(
            err.contains("to 700"),
            "dir tightening pins exactly 0700: {err}"
        );
        assert!(
            err.contains(tmp.to_str().unwrap()),
            "error carries the exact path: {err}"
        );

        // ...and the lockfile tightening pins EXACTLY 0600.
        let lock = tmp.join("probe.lock");
        std::fs::write(&lock, b"x").unwrap();
        let err = match tighten_with(&lock, 0o600, real_tighten) {
            Err(report) => format!("{report:#}"),
            Ok(()) => panic!("injected EPERM must fail"),
        };
        assert!(
            err.contains("to 600"),
            "lockfile tightening pins exactly 0600: {err}"
        );
        assert!(
            err.contains(lock.to_str().unwrap()),
            "error carries the exact lock path: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The production default path (no injection) performs the real chmod.
    #[test]
    fn tighten_default_is_real_chmod() {
        let tmp = std::env::temp_dir().join(format!("tighten-d-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(tighten_with(&tmp, 0o700, real_tighten).is_ok());
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&tmp).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "real chmod ran (0700 exactly)");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
