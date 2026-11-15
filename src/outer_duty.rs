//! `octoscode outer-duty` (OUTER_LOOP_REVIEW #38 / #38-r1): the kernel lock
//! behind multi-outer primary-reviewer authority — a per-project,
//! session-lifetime OS-exclusive lock. Linux and macOS (single-machine
//! `flock`; NFS out of scope); other platforms route the subcommand to an
//! explicit unsupported error (see `src/cmd/mod.rs`); Windows LockFileEx
//! support is a separate follow-up entry. The module does not compile on
//! unsupported targets by design (honest shrink, per the countersign).
//!
//! Lifecycle binding (#38-r2 adjudicated: GUARDIAN death coupling) — the
//! invariant on every supported platform is "wrapper dead ⇒ agent and
//! everything it started dead, and the lock is never VACANT while any of
//! them is alive":
//!
//! - `wrapper → guardian → agent`. The guardian is a fork of the wrapper
//!   without exec, so it inherits the lock fd; the agent never holds it
//!   (CLOEXEC). The guardian watches the wrapper (macOS: kqueue
//!   EVFILT_PROC; Linux: pidfd; both: a getppid poll). When the wrapper
//!   dies it freezes (SIGSTOP) and SIGKILLs the agent's whole tree —
//!   whatever process groups or sessions its members moved to, such as
//!   the commands Codex runs — and exits, releasing the lock, only once
//!   they are all gone (capped at 30 s).
//! - Linux: guardian and wrapper are child subreapers, so an orphaned
//!   descendant is reparented to the guardian (or, once it is gone, to the
//!   wrapper) rather than to init and stays in reach; the agent also gets
//!   PR_SET_PDEATHSIG(SIGKILL) against the guardian.
//! - macOS: there is no subreaper; a descendant that had already left the
//!   tree (reparented to launchd because its parent exited) is not seen.
//! - The wrapper watches the guardian: a guardian that dies abnormally
//!   makes the wrapper, still holding the lock, kill the agent's tree and
//!   fail. Boundary: this survives the death of either supervisor, not a
//!   simultaneous SIGKILL of both.
//!
//! An agent that exits on its own leaves its lingering grandchildren
//! alone: they hold no fd and the lock is VACANT. These invariants are
//! contract-pinned with real processes on both platforms.
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
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return DutyState::Error;
        }
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
/// agent pid (what the diagnostic sidecar records); `wait()` blocks until
/// the agent is gone and yields its exit status.
pub struct HolderChild {
    guardian: libc::pid_t,
    agent: libc::pid_t,
    /// The terminal handed to the agent (None: wrapper not in the
    /// foreground of a terminal).
    terminal: Option<terminal::Foreground>,
}

impl HolderChild {
    /// The real agent pid.
    pub fn id(&self) -> u32 {
        self.agent as u32
    }

    /// Wait for the guardian, which relays the agent's exit code
    /// (128+signal for a signal death). A guardian that itself dies by a
    /// signal is a supervision failure: every descendant left is killed
    /// and an error is returned (the wrapper still holds the lock while
    /// that happens).
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = guardian::wait_guardian(self.guardian, self.agent);
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
}

/// Spawn `wrapper → guardian → agent` (see the module docs). The guardian
/// is a fork WITHOUT exec, so it inherits the lock fd (same open file
/// description ⇒ same flock) and keeps the authority HELD until every
/// descendant of the agent is gone. Must be called before any thread is
/// started (the `outer-duty` subcommand dispatches first thing in `main`).
pub fn spawn_holder_child(command: &[String]) -> Result<HolderChild> {
    let terminal = terminal::capture();
    let (guardian, agent) = guardian::spawn_guarded(command, terminal)?;
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

/// A snapshot of the process table: every process with its parent and
/// state. Fixed-size buffers, so it can run in the guardian after
/// `fork()`.
#[allow(unsafe_code)]
mod proctab {
    /// More processes than this are not seen (far above a workstation's).
    pub(super) const CAP: usize = 16_384;

    #[derive(Clone, Copy, Default)]
    pub(super) struct Entry {
        pub(super) pid: libc::pid_t,
        pub(super) ppid: libc::pid_t,
        /// 'R' running, 'T' stopped, 'Z' zombie (normalised across platforms).
        pub(super) state: u8,
    }

    pub(super) struct Table {
        pub(super) entries: [Entry; CAP],
        pub(super) len: usize,
    }

    impl Table {
        pub(super) fn new() -> Self {
            Self {
                entries: [Entry::default(); CAP],
                len: 0,
            }
        }

        fn push(&mut self, entry: Entry) {
            if self.len < CAP {
                self.entries[self.len] = entry;
                self.len += 1;
            }
        }

        pub(super) fn iter(&self) -> impl Iterator<Item = &Entry> {
            self.entries[..self.len].iter()
        }
    }

    /// One process's parent and state, None when it does not exist.
    #[cfg(target_os = "macos")]
    pub(super) fn read(pid: libc::pid_t) -> Option<Entry> {
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            if libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            ) != size
            {
                return None;
            }
            let state = match info.pbi_status {
                libc::SSTOP => b'T',
                libc::SZOMB => b'Z',
                _ => b'R',
            };
            Some(Entry {
                pid,
                ppid: info.pbi_ppid as libc::pid_t,
                state,
            })
        }
    }

    /// One process's parent and state from `/proc/<pid>/stat`.
    #[cfg(target_os = "linux")]
    pub(super) fn read(pid: libc::pid_t) -> Option<Entry> {
        let mut path = *b"/proc/0000000000/stat\0";
        let mut digits = [0u8; 10];
        let mut n = pid as u32;
        let mut count = 0;
        loop {
            digits[count] = b'0' + (n % 10) as u8;
            count += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let mut at = 6;
        for i in (0..count).rev() {
            path[at] = digits[i];
            at += 1;
        }
        path[at..at + 6].copy_from_slice(b"/stat\0");
        let mut buf = [0u8; 512];
        let read = unsafe {
            let fd = libc::open(path.as_ptr().cast(), libc::O_RDONLY | libc::O_CLOEXEC);
            if fd == -1 {
                return None;
            }
            let read = libc::read(fd, buf.as_mut_ptr().cast(), buf.len());
            libc::close(fd);
            read
        };
        if read <= 0 {
            return None;
        }
        let text = &buf[..read as usize];
        // "<pid> (<comm>) <state> <ppid> …"; comm may hold ')' or spaces.
        let close = text.iter().rposition(|&b| b == b')')?;
        let rest = text.get(close + 2..)?;
        let state = match *rest.first()? {
            b'T' | b't' => b'T',
            b'Z' | b'X' => b'Z',
            _ => b'R',
        };
        let mut ppid: libc::pid_t = 0;
        for &b in rest.get(2..)? {
            if !b.is_ascii_digit() {
                break;
            }
            ppid = ppid * 10 + (b - b'0') as libc::pid_t;
        }
        Some(Entry { pid, ppid, state })
    }

    /// Every process now.
    #[cfg(target_os = "macos")]
    pub(super) fn snapshot(table: &mut Table) {
        table.len = 0;
        let mut pids = [0 as libc::pid_t; CAP];
        let bytes = (CAP * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        for &pid in pids.iter().take(count.max(0) as usize) {
            if pid > 0
                && let Some(entry) = read(pid)
            {
                table.push(entry);
            }
        }
    }

    /// Every process now (`/proc`).
    #[cfg(target_os = "linux")]
    pub(super) fn snapshot(table: &mut Table) {
        table.len = 0;
        unsafe {
            let dir = libc::opendir(c"/proc".as_ptr());
            if dir.is_null() {
                return;
            }
            loop {
                let ent = libc::readdir(dir);
                if ent.is_null() {
                    break;
                }
                let name = std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()).to_bytes();
                if name.is_empty() || !name.iter().all(u8::is_ascii_digit) {
                    continue;
                }
                let mut pid: libc::pid_t = 0;
                for &b in name {
                    pid = pid * 10 + (b - b'0') as libc::pid_t;
                }
                if let Some(entry) = read(pid) {
                    table.push(entry);
                }
            }
            libc::closedir(dir);
        }
    }

    /// Gone: no such process, or a zombie (it has exited).
    pub(super) fn gone(pid: libc::pid_t) -> bool {
        read(pid).is_none_or(|entry| entry.state == b'Z')
    }

    /// Stopped (false when it cannot be read).
    pub(super) fn stopped(pid: libc::pid_t) -> bool {
        read(pid).is_some_and(|entry| entry.state == b'T')
    }
}

/// The guardian (reaper) between the wrapper and the agent. Everything
/// that runs after `fork()` uses libc calls and fixed-size buffers only;
/// the process is single-threaded when it forks.
#[allow(unsafe_code)]
mod guardian {
    use super::proctab::{self, Table};
    use super::terminal::{self, Foreground};
    use eyre::{Result, WrapErr, eyre};
    use std::ffi::CString;

    /// How often the guardian re-checks its parent when no event arrives.
    const POLL_MS: libc::c_int = 100;
    /// Upper bound on waiting for killed processes to disappear before
    /// the lock is released anyway (never a permanent stuck lock).
    const GONE_CAP_MS: u32 = 30_000;
    /// Signals the guardian ignores so a terminal ^C / hangup aimed at the
    /// wrapper cannot take both supervisors down at once.
    const GUARDED_SIGNALS: [libc::c_int; 4] =
        [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];
    /// Upper bound on waiting for a SIGSTOP sent to the wrapper to land.
    const STOP_LAND_CAP_MS: u32 = 1_000;
    /// Most processes one kill handles (an agent's tree is far smaller).
    const TREE_CAP: usize = 4_096;

    fn errno() -> libc::c_int {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

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
            // Linux: if the guardian dies, its orphans come to the wrapper
            // (which still holds the lock) instead of to init.
            #[cfg(target_os = "linux")]
            libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
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
            // still hold the lock — kill everything it left before failing,
            // so the authority is never released over live work. Linux:
            // the orphans were handed to us (subreaper). macOS: the agent
            // went to launchd with its tree intact.
            #[cfg(target_os = "linux")]
            kill_tree(libc::getpid(), false);
            #[cfg(target_os = "macos")]
            kill_tree(agent, true);
            #[cfg(target_os = "linux")]
            let _ = agent;
        }
        Err(std::io::Error::other(
            "the duty guardian died abnormally; the agent and its descendants were killed",
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
            // Linux: orphaned descendants of the agent come to us, so a
            // command the agent ran in its own process group stays in
            // reach after the agent is gone.
            #[cfg(target_os = "linux")]
            libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);

            let mut events = Events::new();
            events.watch(wrapper);
            // The wrapper may already be gone (fork race): no agent yet,
            // nothing to guard — the lock is released by our exit.
            if libc::getppid() != wrapper {
                libc::_exit(1);
            }

            let me = libc::getpid();
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
                // Linux, belt and braces: the guardian dies ⇒ the kernel
                // kills the agent at once (the wrapper kills the rest).
                #[cfg(target_os = "linux")]
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 || libc::getppid() != me
                {
                    libc::_exit(127);
                }
                #[cfg(target_os = "macos")]
                let _ = me;
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
            events.watch(agent);

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
                        terminal::relay_stop(term, agent, || stop_wrapper(&events, wrapper));
                    }
                    continue;
                }
                if reaped == agent {
                    // Agent finished on its own: relay its code. Lingering
                    // grandchildren hold no fd and are left alone.
                    libc::_exit(exit_code(status));
                }
                // Linux: reap orphans handed to us; the agent itself may
                // be among them if it exited just now.
                #[cfg(target_os = "linux")]
                loop {
                    let mut other = 0;
                    let pid = libc::waitpid(-1, &mut other, libc::WNOHANG);
                    if pid == agent {
                        libc::_exit(exit_code(other));
                    }
                    if pid <= 0 {
                        break;
                    }
                }
                if libc::getppid() != wrapper {
                    // Wrapper died: kill the agent and every descendant,
                    // and release the lock (by exiting) only once they are
                    // all gone.
                    kill_tree(me, false);
                    libc::_exit(128 + libc::SIGKILL);
                }
                events.wait();
            }
        }
    }

    /// Kill `root`'s whole tree (with `root` itself when `with_root`),
    /// whatever process groups or sessions its members moved to, and
    /// return once all are gone (capped). The tree is frozen first
    /// (SIGSTOP, repeated until no new member appears) so nothing forks
    /// its way out between finding and killing.
    unsafe fn kill_tree(root: libc::pid_t, with_root: bool) {
        unsafe {
            let mut table = Table::new();
            let mut tree = [0 as libc::pid_t; TREE_CAP];
            let mut len = 0usize;
            if with_root {
                tree[0] = root;
                len = 1;
            }
            for _ in 0..50 {
                proctab::snapshot(&mut table);
                let before = len;
                // Grow the set until closed under "child of a member".
                let mut grew = true;
                while grew {
                    grew = false;
                    for entry in table.iter() {
                        let parent_in = entry.ppid == root || tree[..len].contains(&entry.ppid);
                        if parent_in
                            && entry.pid != root
                            && !tree[..len].contains(&entry.pid)
                            && len < TREE_CAP
                        {
                            tree[len] = entry.pid;
                            len += 1;
                            grew = true;
                        }
                    }
                }
                for &pid in &tree[..len] {
                    libc::kill(pid, libc::SIGSTOP);
                }
                if len == before && before > 0 {
                    break;
                }
                if len == 0 {
                    break;
                }
            }
            for &pid in &tree[..len] {
                libc::kill(pid, libc::SIGKILL);
            }
            let mut waited_ms = 0u32;
            while waited_ms < GONE_CAP_MS {
                // Reap whatever of it is ours (direct or adopted children).
                loop {
                    let mut status = 0;
                    if libc::waitpid(-1, &mut status, libc::WNOHANG) <= 0 {
                        break;
                    }
                }
                if tree[..len].iter().all(|&pid| proctab::gone(pid)) {
                    return;
                }
                for &pid in &tree[..len] {
                    libc::kill(pid, libc::SIGKILL);
                }
                libc::usleep(10_000);
                waited_ms += 10;
            }
        }
    }

    /// Stop the wrapper (the shell sees its job stopped) and return once
    /// it runs again or is gone. The guardian is not the wrapper's
    /// parent, so it watches the wrapper's state.
    unsafe fn stop_wrapper(events: &Events, wrapper: libc::pid_t) {
        unsafe {
            libc::kill(wrapper, libc::SIGSTOP);
            // First see the stop land (bounded), then wait for SIGCONT.
            let mut waited_ms = 0u32;
            while !proctab::stopped(wrapper) && waited_ms < STOP_LAND_CAP_MS {
                libc::usleep(10_000);
                waited_ms += 10;
            }
            while libc::getppid() == wrapper && proctab::stopped(wrapper) {
                events.wait();
            }
        }
    }

    /// Wake-ups when a watched process exits (best effort: the guardian
    /// also polls every POLL_MS). macOS: kqueue NOTE_EXIT. Linux: pidfds.
    struct Events {
        #[cfg(target_os = "macos")]
        kq: libc::c_int,
        #[cfg(target_os = "linux")]
        fds: [libc::pollfd; 2],
    }

    impl Events {
        fn new() -> Self {
            #[cfg(target_os = "macos")]
            {
                Self {
                    kq: unsafe { libc::kqueue() },
                }
            }
            #[cfg(target_os = "linux")]
            {
                let empty = libc::pollfd {
                    fd: -1,
                    events: libc::POLLIN,
                    revents: 0,
                };
                Self {
                    fds: [empty, empty],
                }
            }
        }

        unsafe fn watch(&mut self, pid: libc::pid_t) {
            #[cfg(target_os = "macos")]
            unsafe {
                if self.kq == -1 {
                    return;
                }
                let mut change: libc::kevent = std::mem::zeroed();
                change.ident = pid as libc::uintptr_t;
                change.filter = libc::EVFILT_PROC;
                change.flags = libc::EV_ADD | libc::EV_ONESHOT;
                change.fflags = libc::NOTE_EXIT;
                libc::kevent(
                    self.kq,
                    &change,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                );
            }
            #[cfg(target_os = "linux")]
            unsafe {
                if let Some(slot) = self.fds.iter_mut().find(|slot| slot.fd == -1) {
                    let fd = libc::syscall(libc::SYS_pidfd_open, pid, 0);
                    slot.fd = if fd < 0 { -1 } else { fd as libc::c_int };
                }
            }
        }

        /// Sleep until a watched process exits or the poll interval elapses.
        fn wait(&self) {
            #[cfg(target_os = "macos")]
            unsafe {
                if self.kq == -1 {
                    libc::usleep((POLL_MS as libc::useconds_t) * 1000);
                    return;
                }
                let timeout = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: POLL_MS as libc::c_long * 1_000_000,
                };
                let mut event: libc::kevent = std::mem::zeroed();
                libc::kevent(self.kq, std::ptr::null(), 0, &mut event, 1, &timeout);
            }
            #[cfg(target_os = "linux")]
            unsafe {
                // Negative fds are ignored: with no pidfd this is a sleep.
                let mut fds = self.fds;
                libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, POLL_MS);
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
                if errno() != libc::EINTR {
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
                } else if read == -1 && errno() == libc::EINTR {
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
