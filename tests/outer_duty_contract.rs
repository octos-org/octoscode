//! Contract tests for `octoscode outer-duty` (#38 / #38-r1) — the
//! per-project session-lifetime OS-exclusive duty lock. Real subprocess
//! (CARGO_BIN_EXE), temp HOME + project (locks never touch real state).
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn bin() -> PathBuf {
    // CARGO_BIN_EXE may point at a bin-unittest harness under some cargo
    // versions; prefer the real bin adjacent to the test deps directory,
    // falling back to the env var and the plain target path.
    let candidates = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("../../octoscode")))
            .map(|p| p.canonicalize().unwrap_or(p)),
        std::env::var("CARGO_BIN_EXE_octoscode")
            .ok()
            .map(PathBuf::from),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/octoscode")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_BIN_EXE_octoscode"))
}

struct TempHome(PathBuf);
impl TempHome {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("outer-duty-r1-{tag}-{}-{seq}", std::process::id(),));
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::create_dir_all(dir.join("project")).unwrap();
        Self(dir)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn project(&self) -> PathBuf {
        self.0.join("project")
    }
    fn sentinel(&self) -> PathBuf {
        self.0.join("sentinel")
    }
    /// Recompute the stable SHA-256 lock name (domain-prefixed) the same
    /// way the implementation does — used to locate files for assertions.
    fn lock_path(&self) -> PathBuf {
        use sha2::Digest as _;
        let canonical = std::fs::canonicalize(self.project()).unwrap();
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"octoscode/outer-duty/v1");
        hasher.update([0u8]);
        hasher.update(canonical.to_string_lossy().as_bytes());
        let digest_bytes = hasher.finalize();
        let mut digest = String::new();
        for byte in digest_bytes {
            digest.push_str(&format!("{byte:02x}"));
        }
        self.home()
            .join(".octos")
            .join("outer")
            .join("duty")
            .join(format!("{digest}.lock"))
    }
}
impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Warm the binary/page cache once per test run (first-spawn cold jitter
/// was the intermittence amplifier for the acquire polls).
fn warmup() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = Command::new(bin())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

fn duty(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(bin());
    cmd.arg("outer-duty").env("HOME", home);
    cmd
}

/// Spawn `hold` whose child loops while a per-test sentinel file exists.
/// Returns (wrapper, sentinel): remove the sentinel to end the REAL child.
#[allow(clippy::zombie_processes)]
fn spawn_holder(
    home: &std::path::Path,
    project: &std::path::Path,
    sentinel: &std::path::Path,
) -> std::process::Child {
    std::fs::write(sentinel, b"run").unwrap();
    let started = sentinel.with_extension("started");
    let loop_cmd = format!(
        "while test -e {sent}; do touch {started}; sleep 0.05; done",
        sent = sentinel.display(),
        started = started.display(),
    );
    let mut cmd = duty(home);
    cmd.args([
        "hold",
        "--project",
        project.to_str().unwrap(),
        "--signature",
        "test-holder",
        "--duties",
        "primary-review",
        "--",
        "/bin/sh",
        "-c",
        &loop_cmd,
    ])
    .stdout(Stdio::null())
    .stderr(std::process::Stdio::from(
        std::fs::File::create(sentinel.with_extension("wrapper-err")).unwrap(),
    ));
    warmup();
    let child = cmd.spawn().expect("spawn holder");
    for _ in 0..1200 {
        let out = duty(home)
            .args(["check", "--project", project.to_str().unwrap()])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&out.stdout).trim() == "HELD" {
            return child;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let wrapper_err =
        std::fs::read_to_string(sentinel.with_extension("wrapper-err")).unwrap_or_default();
    panic!("holder never acquired the lock; wrapper_err={wrapper_err:?}");
}

/// Wait until the holder's real child is provably past execve (started
/// marker from the sentinel loop).
fn wait_started(env: &TempHome) {
    let started = env.sentinel().with_extension("started");
    for _ in 0..600 {
        if started.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("real child never started");
}

fn check_state(home: &std::path::Path, project: &std::path::Path) -> String {
    let out = duty(home)
        .args(["check", "--project", project.to_str().unwrap()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 契约 1(#38-r3/r4:真 barrier): two contenders released from a SHARED
/// gate at the same instant — exactly one exits 0, the other contention.
#[test]
fn duty_two_contenders_exactly_one_wins() {
    let env = TempHome::new("two");
    let project = env.project();
    let gate = env.0.join("start-gate");
    let stop = env.0.join("stop");
    std::fs::write(&gate, b"wait").unwrap();
    std::fs::write(&stop, b"run").unwrap();
    let spin = |sig: &str| {
        format!(
            "while test -e {g}; do :; done; exec {bin} outer-duty hold --project {p} --signature {sig} --duties review -- /bin/sh -c 'while test -e {s}; do sleep 0.05; done'",
            g = gate.display(),
            bin = bin().display(),
            p = project.display(),
            s = stop.display(),
        )
    };
    warmup();
    // Capture both contenders' stderr to files (never /dev/null): the
    // loser's diagnostics must be the CONTENTION message itself — some
    // other failure (crash, usage error) must never masquerade as it.
    let err_a = env.0.join("stderr-c1");
    let err_b = env.0.join("stderr-c2");
    let spin_with_err = |sig: &str, err: &std::path::Path| {
        format!("{spin} 2>{err}", spin = spin(sig), err = err.display(),)
    };
    let mut a = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(spin_with_err("c1", &err_a))
        .env("HOME", env.home())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut b = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(spin_with_err("c2", &err_b))
        .env("HOME", env.home())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let _ = std::fs::remove_file(&gate);
    let mut hel = false;
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "HELD" {
            hel = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(hel, "a holder must be observed HELD");
    let mut loser: Option<char> = None;
    for _ in 0..1200 {
        match (a.try_wait().unwrap(), b.try_wait().unwrap()) {
            (Some(_), None) => {
                loser = Some('a');
                break;
            }
            (None, Some(_)) => {
                loser = Some('b');
                break;
            }
            (Some(_), Some(_)) => break,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        loser.is_some(),
        "the contention loser must settle while the winner holds"
    );
    let _ = std::fs::remove_file(&stop);
    let a_out = a.wait().unwrap();
    let b_out = b.wait().unwrap();
    let codes = [a_out.code(), b_out.code()];
    let winners = codes.iter().filter(|c| **c == Some(0)).count();
    assert_eq!(winners, 1, "exactly one contender exits 0 (got {codes:?})");
    // The LOSER's stderr must be the lock-contention diagnostic itself:
    // non-zero exit + "HELD"/"contention" wording, never some other fault.
    if let Some(who) = loser {
        let (loser_code, loser_err) = if who == 'a' {
            (
                a_out.code(),
                std::fs::read_to_string(&err_a).unwrap_or_default(),
            )
        } else {
            (
                b_out.code(),
                std::fs::read_to_string(&err_b).unwrap_or_default(),
            )
        };
        assert_ne!(loser_code, Some(0), "loser exited non-zero");
        assert!(
            loser_err.contains("HELD by another live holder") || loser_err.contains("contention"),
            "loser stderr must be the contention diagnostic, got: {loser_err}"
        );
    }
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "VACANT" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        check_state(env.home().as_path(), project.as_path()),
        "VACANT"
    );
}

/// 契约 2a(#38-r3/r4:真实 child 观测): wrapper SIGKILL → the REAL agent
/// (sidecar-recorded PID + /proc starttime, reuse-proof) must DIE, then
/// the lock goes VACANT — not the lock state alone.
#[test]
fn duty_wrapper_death_kills_agent_and_releases() {
    let env = TempHome::new("guardian");
    let project = env.project();
    let mut wrapper = spawn_holder(
        env.home().as_path(),
        project.as_path(),
        env.sentinel().as_path(),
    );
    wait_started(&env);
    let (pid, start) = read_sidecar_child(&env);
    assert!(pid > 0, "sidecar must record the real child pid");
    assert!(start > 0, "sidecar must record a real child starttime (>0)");
    assert!(proc_alive_with_start(pid, start), "agent alive before kill");
    wrapper.kill().unwrap();
    let _ = wrapper.wait();
    let mut died = false;
    for _ in 0..600 {
        if !proc_alive_with_start(pid, start) {
            died = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        died,
        "real agent (pid {pid}) must die when the wrapper is SIGKILLed"
    );
    let mut vacant = false;
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "VACANT" {
            vacant = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(vacant, "wrapper death must release the authority");
}

/// child_pid/child_starttime from the diagnostic sidecar JSON.
fn read_sidecar_child(env: &TempHome) -> (u32, u64) {
    for _ in 0..600 {
        if let Ok(text) = std::fs::read_to_string(env.lock_path().with_extension("meta")) {
            let pid = text
                .split("\"child_pid\":")
                .nth(1)
                .and_then(|r| r.split(',').next())
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let start = text
                .split("\"child_starttime\":")
                .nth(1)
                .and_then(|r| r.split('}').next())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(0);
            if pid > 0 {
                return (pid, start);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("sidecar never recorded child_pid");
}

/// /proc/<pid>/stat field 22 must equal the recorded starttime (reuse-proof).
fn proc_alive_with_start(pid: u32, starttime: u64) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(rest) = stat.rsplit(')').next() else {
        return false;
    };
    let start: u64 = rest
        .split_whitespace()
        .nth(19)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0);
    start != 0 && start == starttime
}

/// 契约 2b(#38-r3/r4:grandchild 活性观测): the agent exits while a
/// GRANDCHILD keeps running — observed ALIVE via its own pid + 10Hz
/// heartbeat mtime — and holds no fd: VACANT while provably alive.
#[test]
fn duty_grandchild_lingering_yields_vacant() {
    let env = TempHome::new("grandchild");
    let project = env.project();
    let gctl = env.0.join("grand-ctl");
    let ginfo = env.0.join("grand-info");
    std::fs::write(&gctl, b"run").unwrap();
    let loop_cmd = format!(
        "/bin/sh -c 'echo $$ > {info}; while test -e {ctl}; do touch {info}; sleep 0.1; done' & exit 0",
        info = ginfo.display(),
        ctl = gctl.display(),
    );
    let mut cmd = duty(env.home().as_path());
    cmd.args([
        "hold",
        "--project",
        project.to_str().unwrap(),
        "--signature",
        "agent",
        "--duties",
        "review",
        "--",
        "/bin/sh",
        "-c",
        &loop_cmd,
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let wrapper = cmd.spawn().unwrap();
    let mut gpid = 0u32;
    for _ in 0..600 {
        if let Ok(text) = std::fs::read_to_string(&ginfo) {
            if let Ok(p) = text.trim().parse::<u32>() {
                gpid = p;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(gpid > 0, "grandchild must register its pid");
    let mut vacant = false;
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "VACANT" {
            vacant = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        vacant,
        "a lingering grandchild must never keep the duty lock"
    );
    assert!(
        heartbeat_fresh(&ginfo),
        "grandchild (pid {gpid}) provably alive at VACANT"
    );
    assert!(
        std::path::Path::new(&format!("/proc/{gpid}")).exists(),
        "grandchild proc exists"
    );
    let _ = std::fs::remove_file(&gctl);
    let _ = wrapper;
}

fn heartbeat_fresh(info: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(info) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime.elapsed().map(|e| e.as_secs() < 2).unwrap_or(false)
}

/// 契约 3: repeated checks never disturb the live holder.
#[test]
fn duty_check_does_not_disturb_holder() {
    let env = TempHome::new("probe");
    let project = env.project();
    let mut holder = spawn_holder(
        env.home().as_path(),
        project.as_path(),
        env.sentinel().as_path(),
    );
    for _ in 0..10 {
        assert_eq!(check_state(env.home().as_path(), project.as_path()), "HELD");
    }
    holder.kill().unwrap();
    let _ = holder.wait();
    release_via_sentinel(&env);
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "VACANT" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        check_state(env.home().as_path(), project.as_path()),
        "VACANT"
    );
}

/// 契约 4: corrupt sidecar → HELD verdict unchanged, METADATA_CORRUPT on
/// stderr; after the holders die → VACANT (adjudication never reads sidecar).
#[test]
fn duty_corrupt_metadata_keeps_ownership() {
    let env = TempHome::new("meta");
    let project = env.project();
    let mut holder = spawn_holder(
        env.home().as_path(),
        project.as_path(),
        env.sentinel().as_path(),
    );
    // #38-r5: wait for the wrapper's healthy sidecar to LAND before
    // corrupting it — otherwise the corruption races the wrapper's own
    // atomic rename and can be overwritten by the fresh healthy JSON
    // (holder field present = the write completed).
    let sidecar = env.lock_path().with_extension("meta");
    let mut settled = false;
    for _ in 0..600 {
        if let Ok(text) = std::fs::read_to_string(&sidecar) {
            if text.contains("holder") || text.contains("signature") {
                settled = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(settled, "wrapper never wrote a healthy sidecar to corrupt");
    std::fs::write(&sidecar, "{not json").unwrap();
    let out = duty(env.home().as_path())
        .args(["check", "--project", project.to_str().unwrap()])
        .output()
        .unwrap();
    // stdout: single state line; diagnostics on stderr.
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "HELD");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("METADATA_CORRUPT"), "{stderr}");
    holder.kill().unwrap();
    let _ = holder.wait();
    release_via_sentinel(&env);
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "VACANT" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        check_state(env.home().as_path(), project.as_path()),
        "VACANT"
    );
}

/// 契约 5: lockfile + sidecar + duty dir are 0600/0700 even when
/// pre-created 0644/0755 (tighten-on-open) under a permissive umask.
#[test]
fn duty_files_tighten_preexisting_permissive() {
    let env = TempHome::new("perm");
    let project = env.project();
    // Permissive umask EXPLICITLY set for the whole test (countersign D:
    // umask 显式设置) — restored on exit.
    struct UmaskGuard(libc::mode_t);
    impl Drop for UmaskGuard {
        fn drop(&mut self) {
            #[allow(unsafe_code)]
            unsafe {
                libc::umask(self.0)
            };
        }
    }
    #[allow(unsafe_code)]
    let _umask_guard = UmaskGuard(unsafe { libc::umask(0o000) });
    // Pre-create the lock dir + lockfile WORLD-READABLE.
    let lock = env.lock_path();
    let dir = lock.parent().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&lock, b"").unwrap();
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut holder = spawn_holder(
        env.home().as_path(),
        project.as_path(),
        env.sentinel().as_path(),
    );
    let lock_mode = std::fs::metadata(&lock).unwrap().permissions().mode() & 0o777;
    let dir_mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        lock_mode, 0o600,
        "lockfile tightened from 0644: {lock_mode:o}"
    );
    assert_eq!(dir_mode, 0o700, "dir tightened from 0755: {dir_mode:o}");
    let meta = lock.with_extension("meta");
    for _ in 0..600 {
        if meta.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let meta_mode = std::fs::metadata(&meta).unwrap().permissions().mode() & 0o777;
    assert_eq!(meta_mode, 0o600);
    holder.kill().unwrap();
    let _ = holder.wait();
    release_via_sentinel(&env);
}

/// 契约 6(#38-r1 B/D): ERROR — never VACANT — for a missing HOME
/// (fail-closed) and a nonexistent project path.
#[test]
fn duty_error_never_vacant_for_bad_inputs() {
    let env = TempHome::new("err");
    // Missing HOME: fail-closed.
    let out = Command::new(bin())
        .arg("outer-duty")
        .args(["check", "--project", env.project().to_str().unwrap()])
        .env_remove("HOME")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ERROR");
    assert!(!out.status.success());
    // Nonexistent project: canonicalize fails → ERROR.
    let out = duty(env.home().as_path())
        .args(["check", "--project", "/nonexistent/project/xyz"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "ERROR",
        "canonicalize failure must be ERROR, not VACANT"
    );
}

/// 契约 7(#38-r1 C): stdout is EXACTLY one state line even when the
/// signature/duties metadata is hostile (newlines/ANSI/oversized).
#[test]
fn duty_stdout_single_line_with_hostile_metadata() {
    let env = TempHome::new("single");
    let project = env.project();
    let mut cmd = duty(env.home().as_path());
    cmd.args([
        "hold",
        "--project",
        project.to_str().unwrap(),
        "--signature",
        "evil\nANSI\x1b[31m",
        "--duties",
        "line1\nline2\r\nline3",
        "--",
        "sleep",
        "30",
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let mut holder = cmd.spawn().unwrap();
    for _ in 0..200 {
        let out = duty(env.home().as_path())
            .args(["check", "--project", project.to_str().unwrap()])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let trimmed = stdout.trim();
        if trimmed == "HELD" {
            assert_eq!(stdout.lines().count(), 1, "stdout stays one line");
            break;
        }
        assert!(
            ["VACANT", "ERROR"].contains(&trimmed) || trimmed == "HELD",
            "single machine state per line: {stdout:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    holder.kill().unwrap();
    let _ = holder.wait();
    release_via_sentinel(&env);
}

/// 契约 8(#38-r2 B3): stable digest — a FIXED input's 64-hex golden,
/// computed independently (not a mirror of the implementation), plus
/// symlink/relative convergence on the same canonical lock.
#[test]
fn duty_lock_digest_golden_and_convergence() {
    // Golden: sha256("octoscode/outer-duty/v1\0" ++ "/tmp/duty-golden-proj")
    // — computed out-of-band; the pure fn must reproduce it exactly.
    // FIXED out-of-band constant (printf ... | sha256sum, precomputed):
    // sha256("octoscode/outer-duty/v1\0/tmp/duty-golden-proj")
    let golden = "d258b689203cfb1b3c95d56e0bbef32a436cb0952817f581c6bb3aed82461bbb";
    assert_eq!(
        octoscode::outer_duty::lock_digest(b"/tmp/duty-golden-proj"),
        golden,
        "stable lock-name digest must match the independent golden"
    );
    assert_eq!(golden.len(), 64);

    let env = TempHome::new("symlink");
    let project = env.project();
    let link = env.0.join("project-link");
    std::os::unix::fs::symlink(&project, &link).unwrap();
    let mut holder = spawn_holder(
        env.home().as_path(),
        project.as_path(),
        env.sentinel().as_path(),
    );
    assert_eq!(
        check_state(env.home().as_path(), link.as_path()),
        "HELD",
        "symlink path converges on the same canonical lock"
    );
    let lock = env.lock_path();
    assert!(lock.exists(), "stable-named lock at {}", lock.display());
    holder.kill().unwrap();
    let _ = holder.wait();
    for _ in 0..600 {
        if check_state(env.home().as_path(), project.as_path()) == "VACANT" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// End the real child by removing its sentinel and wait for VACANT.
fn release_via_sentinel(env: &TempHome) {
    let _ = std::fs::remove_file(env.sentinel());
    for _ in 0..600 {
        if check_state(env.home().as_path(), env.project().as_path()) == "VACANT" {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 契约 10(#38-r4: tighten 失败路径显式测试): inside a user namespace
/// mapped to a foreign uid (via unshare -r on a nobody-owned file), the
/// pre-existing lock file CANNOT be tightened — chmod must fail with
/// EPERM even against a 0555-dir sibling file — so acquire surfaces an
/// ERROR and never a held lock. Runs the whole scenario under
/// 契约 10(#38-r8:create_dir 失败,原 HOME 0555 场景): create_dir failure —
/// an unwritable HOME (0555) means the duty dir cannot even be created;
/// acquire surfaces ERROR before any lock exists (single-state stdout).
#[test]
fn duty_create_dir_failure_is_error() {
    let env = TempHome::new("create-dir-fail");
    let project = env.project();
    let ro_home = env.0.join("ro-home");
    std::fs::create_dir_all(&ro_home).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&ro_home, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = duty(&ro_home)
        .args([
            "hold",
            "--project",
            project.to_str().unwrap(),
            "--signature",
            "s",
            "--duties",
            "d",
            "--",
            "true",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "acquire under an unwritable HOME must fail (got success)"
    );
    assert!(
        stdout.is_empty() || stdout == "ERROR",
        "stdout stays single-state, got {stdout:?}"
    );
    assert!(
        stderr.contains("duty lock dir") || stderr.contains("Permission denied"),
        "the create-dir failure is the reported cause: {stderr}"
    );
    std::fs::set_permissions(&ro_home, std::fs::Permissions::from_mode(0o755)).unwrap();
}
