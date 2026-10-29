//! `octoscode outer-duty` (OUTER_LOOP_REVIEW #38 / #38-r1): the kernel lock
//! behind multi-outer primary-reviewer authority — a per-project,
//! session-lifetime OS-exclusive lock. Linux-only by adjudication
//! (#38-r3/r4): single-machine `flock` + PR_SET_PDEATHSIG + /proc
//! semantics; NFS out of scope; non-Linux builds route the subcommand to
//! an explicit unsupported error (see `src/cmd/mod.rs`); Windows
//! LockFileEx support is a separate follow-up entry. The module does not
//! compile on non-Linux targets by design (honest shrink, per the
//! countersign).
//!
//! Lifecycle binding (#38-r2 adjudicated: GUARDIAN death coupling): the
//! wrapper is the sole fd holder (CLOEXEC stays set — the lock never leaks
//! to descendants); the child gets setpgid + PR_SET_PDEATHSIG(SIGKILL), so
//! wrapper death kills the agent and releases the authority immediately —
//! no split brain, no lingering authority. Grandchildren hold no fd: an
//! agent that exits while leaving a grandchild running yields VACANT. Both
//! invariants are contract-pinned with real processes.
//!
//! Lock naming is a STABLE protocol: SHA-256 over the domain-prefixed
//! canonical project path (DefaultHasher is not stable across Rust versions
//! — protocol-documented limitation). HOME missing ⇒ fail-closed error.

#![cfg(target_os = "linux")]

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
#[cfg(target_os = "linux")]
fn tighten(path: &Path, mode: u32) -> Result<()> {
    tighten_with(path, mode, real_tighten)
}

/// The parameterized boundary: apply `op` unless a `cfg(test)` injection
/// replaces it (test-only thread-local, invisible in production builds).
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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

/// Spawn the duty-wrapped child with GUARDIAN death coupling (#38-r2
/// adjudicated design): the wrapper is the ONLY fd holder (CLOEXEC stays
/// set — the lock never leaks to descendants); the child runs in its own
/// process group with PR_SET_PDEATHSIG(SIGKILL) — when the wrapper dies,
/// the agent dies with it, releasing the authority immediately (no
/// split brain). The wrapper then waits on the child: when the agent
/// exits (normally or not), the wrapper exits and the lock releases.
///
/// PDEATHSIG semantics boundary: the signal is delivered when the child's
/// *parent thread* dies; it is Linux-specific (hence the linux-gnu target
/// confinement) and does NOT chain to grandchildren — a grandchild that
/// outlives its parent holds no fd and no authority (pinned by contract).
pub fn spawn_holder_child(command: &[String]) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt as _;
    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
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
    cmd.spawn()
        .wrap_err_with(|| format!("failed to spawn duty child: {}", command[0]))
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
