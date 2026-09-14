//! Unified validation-environment regression tests.
//!
//! Real-subprocess checks for `scripts/verify.sh`:
//! - GNU coreutils capability (an already-GNU PATH wins; brew fallback is
//!   only consulted when the capability is missing);
//! - a NESTED python interpreter (verify.sh -> python -> subprocess python)
//!   inherits PYTHONDONTWRITEBYTECODE=1 and writes no __pycache__;
//! - a custom `--` command's non-zero exit propagates untouched;
//! - `--` with no command and unknown arguments print usage and exit 64;
//! - a controlled gnubin-absent probe (inside the script, PATH stripped of
//!   GNU) fails loudly with guidance — not via a shebang-level PATH failure.
//!
//! Only std is used so the file compiles standalone with
//! `rustc --test tests/verify_environment.rs` while the shared cargo target
//! is owned by another lane. The repo root resolves via CARGO_MANIFEST_DIR
//! under cargo, or VERIFY_REPO_ROOT set explicitly for standalone builds.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // Runtime-first resolution: VERIFY_REPO_ROOT (set for standalone
    // `rustc --test` builds), then CARGO_MANIFEST_DIR under `cargo test`.
    // A compile-time env! would break standalone compilation entirely.
    if let Ok(dir) = std::env::var("VERIFY_REPO_ROOT") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(dir);
    }
    // Last resort for in-repo `rustc --test` invocations without either
    // variable: resolve relative to the current directory.
    let cwd = std::env::current_dir().expect("cwd");
    if cwd.join("scripts/verify.sh").exists() {
        return cwd;
    }
    cwd.parent().map(|p| p.to_path_buf()).unwrap_or(cwd)
}

fn verify_sh() -> PathBuf {
    repo_root().join("scripts/verify.sh")
}

fn run_verify(args: &[&str], extra_env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(verify_sh());
    c.args(args);
    // The wrapper must set this itself, even when CI exports it.
    c.env_remove("PYTHONDONTWRITEBYTECODE");
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let out = c.output().expect("spawn verify.sh");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn verify_script_exists_and_is_executable() {
    let p = verify_sh();
    assert!(
        p.exists(),
        "scripts/verify.sh must exist at {}",
        p.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).expect("stat").permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "verify.sh must be executable (mode {mode:o})"
        );
    }
}

#[test]
fn gnu_tools_resolved_or_clear_failure() {
    // `stat -c '%s' <file>` is a GNU-only flag shape: on Linux (native GNU)
    // it must succeed under verify.sh; on Darwin the capability check plus
    // brew/stock-prefix fallback must make it succeed; only a genuinely
    // GNU-less Mac exits 127 with the coreutils/brew guidance.
    let probe = repo_root().join("Cargo.toml");
    let (code, _out, err) = run_verify(
        &["--", "stat", "-c", "%s", &probe.display().to_string()],
        &[],
    );
    if cfg!(target_os = "linux") {
        assert_eq!(code, 0, "Linux must run GNU stat natively via verify.sh");
    } else if code != 0 {
        assert_eq!(
            code, 127,
            "Darwin without GNU capability must exit 127 with guidance, got {code}"
        );
        assert!(
            err.contains("coreutils") && err.contains("brew"),
            "failure text must name the fix (coreutils via brew), got: {err}"
        );
    }
}

#[test]
fn already_gnu_path_is_used_without_brew_requirement() {
    // Controlled GNU-capable PATH built from SHIMS (both platforms): stat
    // and realpath delegates live first, system dirs after. verify.sh must
    // accept the capability as-is — no brew lookup required.
    let dir = unique_temp_dir("verify-env-gnu-path");
    write_shim(&dir, "stat", "stat");
    write_shim(&dir, "realpath", "realpath");
    std::fs::write(dir.join("uname"), "#!/bin/sh\necho Darwin\n").unwrap();
    std::fs::write(
        dir.join("brew"),
        "#!/bin/sh\n: > \"$VERIFY_TEST_BREW_MARKER\"\nexit 99\n",
    )
    .unwrap();
    make_executable(&dir.join("uname"));
    make_executable(&dir.join("brew"));
    let path_env = format!("{}:/usr/bin:/bin", dir.display());
    let probe = repo_root().join("Cargo.toml");
    let (code, out, err) = run_verify(
        &["--", "stat", "-c", "%s", &probe.display().to_string()],
        &[
            ("PATH", &path_env),
            (
                "VERIFY_TEST_BREW_MARKER",
                dir.join("brew-called").to_str().unwrap(),
            ),
        ],
    );
    assert_eq!(
        code, 0,
        "an already-GNU PATH must be used directly (no brew lookup); got {code}: {err}"
    );
    assert!(
        out.trim().chars().all(|c| c.is_ascii_digit()),
        "GNU stat -c '%s' prints the byte size, got: {out}"
    );
    assert!(
        !dir.join("brew-called").exists(),
        "already-GNU PATH must not query brew"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mixed_path_with_bsd_realpath_still_resolves_gnu() {
    // Exercise Darwin resolution on both Linux and macOS. The fallback
    // prefix belongs to this fixture, so no host Homebrew installation or
    // platform-specific PATH accident can make this test pass.
    let dir = unique_temp_dir("verify-env-mixed");
    let prefix = dir.join("coreutils prefix");
    let gnubin = prefix.join("libexec/gnubin");
    std::fs::create_dir_all(&gnubin).unwrap();
    write_shim(&dir, "stat", "stat");
    write_shim(&gnubin, "stat", "stat");
    write_shim(&gnubin, "realpath", "realpath");
    std::fs::write(dir.join("realpath"), "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::write(dir.join("uname"), "#!/bin/sh\necho Darwin\n").unwrap();
    std::fs::write(
        dir.join("brew"),
        "#!/bin/sh\nprintf '%s\\n' \"$VERIFY_TEST_COREUTILS_PREFIX\"\n",
    )
    .unwrap();
    for name in ["realpath", "uname", "brew"] {
        make_executable(&dir.join(name));
    }
    let path_env = format!("{}:/usr/bin:/bin", dir.display());
    let (code, out, err) = run_verify(
        &["--", "realpath", "-m", "/etc/../etc/hosts"],
        &[
            ("PATH", &path_env),
            ("VERIFY_TEST_COREUTILS_PREFIX", prefix.to_str().unwrap()),
        ],
    );
    assert_eq!(
        code, 0,
        "mixed PATH must fall through so realpath -m works; got {code}: {err}"
    );
    assert!(
        out.trim().ends_with("/etc/hosts"),
        "realpath -m normalized the path, got: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_python_inherits_no_bytecode() {
    // TRUE nesting: verify.sh -> python3 (parent) -> subprocess python3
    // (child) imports a module. PYTHONDONTWRITEBYTECODE=1 exported by
    // verify.sh must reach the GRANDCHILD; the import still works and no
    // __pycache__ may appear. A unique temp dir is created and only that
    // directory is removed afterwards (no PID-reuse collisions).
    let dir = std::env::temp_dir().join(format!(
        "verify-env-pyc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let module = dir.join("m.py");
    std::fs::write(&module, "VALUE = 7\n").expect("write module");
    // Parent python spawns the child itself; both run under verify.sh's env.
    let parent_code = format!(
        "import subprocess, sys\n\
         out = subprocess.run([sys.executable, '-c', 'import sys; sys.path.insert(0, {d:?}); import m; print(m.VALUE)'], capture_output=True, text=True)\n\
         sys.stdout.write(out.stdout)\n\
         sys.exit(out.returncode)\n",
        d = dir
    );
    let (code, out, err) = run_verify(&["--", "python3", "-c", &parent_code], &[]);
    assert_eq!(code, 0, "nested python must run; stderr: {err}");
    assert!(
        out.trim().contains("7"),
        "grandchild imported module, got: {out}"
    );
    assert!(
        !dir.join("__pycache__").exists(),
        "no __pycache__ may be created by the nested interpreter tree"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn custom_failure_exit_code_propagates() {
    let (code, _out, _err) = run_verify(&["--", "sh", "-c", "exit 7"], &[]);
    assert_eq!(
        code, 7,
        "a custom command's exit code must propagate untouched"
    );
}

#[test]
fn unknown_argument_and_empty_dash_print_usage() {
    let (code, _out, err) = run_verify(&["--bad"], &[]);
    assert_eq!(code, 64, "unknown argument must exit 64 (usage)");
    assert!(err.contains("usage:"), "usage text on stderr, got: {err}");

    let (code2, _out2, err2) = run_verify(&["--"], &[]);
    assert_eq!(code2, 64, "'--' without a command must exit 64 (usage)");
    assert!(err2.contains("usage:"), "usage text on stderr, got: {err2}");
}

#[test]
fn controlled_missing_gnu_fails_loudly_inside_script() {
    // Deterministic missing-GNU negative on BOTH platforms (no host-prefix
    // dependence, no silent skips): the PATH is built entirely from shims —
    //   uname     -> always reports Darwin (forces the resolution branch);
    //   stat      -> rejects GNU flags;
    //   realpath  -> rejects GNU flags;
    //   brew      -> --prefix coreutils resolves to a FAKE prefix whose
    //                libexec/gnubin EXISTS but holds the same rejecting
    //                stubs — so the fallback prepend succeeds, the re-probe
    //                still fails, and the script reaches its own guidance:
    //                exit 127 with the coreutils/brew remedy text.
    // /bin:/usr/bin stay reachable so env/bash resolve; the stock prefixes
    // are irrelevant because the brew shim's answer wins first.
    let dir = unique_temp_dir("verify-env-nognu");
    let fake_prefix = dir.join("prefix");
    let fake_gnubin = fake_prefix.join("libexec/gnubin");
    std::fs::create_dir_all(&fake_gnubin).expect("mkdir fake gnubin");

    std::fs::write(dir.join("uname"), "#!/bin/sh\necho Darwin\n").expect("uname shim");
    std::fs::write(dir.join("stat"), "#!/bin/sh\nexit 1\n").expect("stat shim");
    std::fs::write(dir.join("realpath"), "#!/bin/sh\nexit 1\n").expect("realpath shim");
    std::fs::write(fake_gnubin.join("stat"), "#!/bin/sh\nexit 1\n").expect("gnubin stat stub");
    std::fs::write(fake_gnubin.join("realpath"), "#!/bin/sh\nexit 1\n")
        .expect("gnubin realpath stub");
    std::fs::write(
        dir.join("brew"),
        format!(
            "#!/bin/sh\n[ \"$1\" = --prefix ] && [ \"$2\" = coreutils ] && printf '%s\\n' {} && exit 0\nexit 1\n",
            fake_prefix.display()
        ),
    )
    .expect("brew shim");
    for name in ["uname", "stat", "realpath", "brew"] {
        make_executable(&dir.join(name));
    }
    make_executable(&fake_gnubin.join("stat"));
    make_executable(&fake_gnubin.join("realpath"));

    let path_env = format!("{}:/usr/bin:/bin", dir.display());
    let probe = repo_root().join("Cargo.toml");
    let (code, _out, err) = run_verify(
        &["--", "stat", "-c", "%s", &probe.display().to_string()],
        &[("PATH", &path_env)],
    );
    assert_eq!(
        code, 127,
        "the deterministic no-GNU fixture must reach the script guidance exit, got {code}: {err}"
    );
    assert!(
        err.contains("coreutils") && err.contains("brew"),
        "script-emitted 127 must carry coreutils/brew guidance, got: {err}"
    );
    for args in [["--bad"], ["--"]] {
        let (code, _, err) = run_verify(&args, &[("PATH", &path_env)]);
        assert_eq!(code, 64, "usage must precede GNU checks: {err}");
        assert!(err.contains("usage:"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unique temp dir helper (pid + nanos — no PID-reuse collisions; the caller
/// removes only this directory).
fn unique_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Write an executable delegating shim for `name` into `dir`.
fn write_shim(dir: &Path, name: &str, delegate_to: &str) {
    let resolved = Command::new(verify_sh())
        .args(["--", "sh", "-c"])
        .arg(format!("command -v {delegate_to}"))
        .output()
        .expect("probe delegate");
    assert!(
        resolved.status.success(),
        "GNU delegate lookup: {}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    let target = String::from_utf8_lossy(&resolved.stdout).trim().to_string();
    assert!(
        !target.is_empty(),
        "delegate {delegate_to} must resolve on PATH"
    );
    let shim = dir.join(name);
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            target.replace('\'', "'\"'\"'")
        ),
    )
    .expect("write shim");
    make_executable(&shim);
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod shim");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
