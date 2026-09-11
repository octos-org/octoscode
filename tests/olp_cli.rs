//! Exercise the installed CLI from unrelated directories, with isolated HOME.
use std::{path::PathBuf, process::Command};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_octoscode"))
}

fn bash() -> PathBuf {
    if let Some(value) = std::env::var_os("OCTOSCODE_BASH") {
        return value.into();
    }
    #[cfg(windows)]
    {
        let git_bash = PathBuf::from("C:/Program Files/Git/bin/bash.exe");
        if git_bash.is_file() {
            return git_bash;
        }
    }
    PathBuf::from("bash")
}

#[test]
fn olp_help_and_invalid_arguments_never_launch_bash() {
    for args in [
        vec!["olp", "--help"],
        vec!["olp", "init", "--help"],
        vec!["olp", "evo", "--help"],
        vec!["olp", "evo", "metrics", "--help"],
    ] {
        let out = cli()
            .args(args)
            .env("OCTOSCODE_BASH", "missing-bash-for-test")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    for args in [
        vec!["olp", "unknown"],
        vec!["olp", "init", ".", "unexpected"],
        vec!["olp", "watch", "board", "token", "--interval", "bad"],
    ] {
        let out = cli().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{out:?}");
    }
}

#[test]
fn olp_missing_bash_has_actionable_error_without_project_changes() {
    let root = tempfile::tempdir().unwrap();
    let out = cli()
        .args(["olp", "--bash", "missing-bash-for-test", "init", "."])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Could not start Bash"));
    assert!(!root.path().join(".octos").exists());
}

#[test]
fn olp_init_uses_target_directory_and_preserves_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("project with spaces 中文 $literal");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".octos")).unwrap();
    std::fs::write(project.join(".octos/loop.md"), "keep this\n").unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&project)
            .status()
            .unwrap()
            .success()
    );
    let invoke = || {
        cli()
            .args(["olp", "--bash"])
            .arg(bash())
            .arg("init")
            .arg(&project)
            .current_dir(root.path())
            .env("HOME", &home)
            .output()
            .unwrap()
    };
    let first = invoke();
    // The unchanged init script exits 2 after scaffolding when octoscode is
    // not installed on PATH. Preserve that status rather than masking it.
    assert!(matches!(first.status.code(), Some(0 | 2)), "{first:?}");
    assert_eq!(
        std::fs::read_to_string(project.join(".octos/loop.md")).unwrap(),
        "keep this\n"
    );
    let board = std::fs::read(project.join(".octos/OUTER_LOOP_REVIEW.md")).unwrap();
    let ignore = std::fs::read(project.join(".gitignore")).unwrap();
    let second = invoke();
    assert_eq!(second.status.code(), first.status.code(), "{second:?}");
    assert_eq!(
        std::fs::read(project.join(".octos/OUTER_LOOP_REVIEW.md")).unwrap(),
        board
    );
    assert_eq!(std::fs::read(project.join(".gitignore")).unwrap(), ignore);
    assert!(!root.path().join(".octos").exists());
    assert!(home.join(".octos/outer/watch-board.sh").is_file());
}

#[cfg(unix)]
#[test]
fn olp_board_append_inherits_stdin_and_propagates_script_failure() {
    use std::{io::Write, process::Stdio};
    let root = tempfile::tempdir().unwrap();
    let board = root.path().join("board with spaces.md");
    let mut child = cli()
        .args(["olp", "--bash"])
        .arg(bash())
        .arg("board-append")
        .arg(&board)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"literal $HOME; $(echo unsafe)\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        std::fs::read_to_string(board).unwrap(),
        "literal $HOME; $(echo unsafe)\n"
    );
    let out = cli()
        .args(["olp", "--bash"])
        .arg(bash())
        .arg("board-append")
        .arg(root.path().join("missing/board.md"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn olp_evo_uses_bundled_python_companions_and_returns_harvest_status() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("knowledge/context/evolution")).unwrap();
    let invoke = |args: &[&str]| {
        cli()
            .args(["olp", "--bash"])
            .arg(bash())
            .arg("evo")
            .args(args)
            .current_dir(root.path())
            .env("HOME", root.path())
            .output()
            .unwrap()
    };
    let index = invoke(&["index", "."]);
    assert!(index.status.success(), "{index:?}");
    assert!(
        std::fs::read_to_string(root.path().join("knowledge/context/evolution/INDEX.md"))
            .unwrap()
            .contains("FLAW")
    );
    let metrics = invoke(&["metrics", ".", "--json"]);
    assert!(metrics.status.success(), "{metrics:?}");
    serde_json::from_slice::<serde_json::Value>(&metrics.stdout).unwrap();
    let harvest = invoke(&["harvest", ".", "--dry-run"]);
    assert_eq!(harvest.status.code(), Some(2), "{harvest:?}");
    assert!(String::from_utf8_lossy(&harvest.stderr).contains("review board not found"));
    assert!(!root.path().join(".octos").exists());
}

#[test]
fn olp_watch_matches_new_literal_lines() {
    use std::{
        io::Write,
        process::Stdio,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().unwrap();
    let board = root.path().join("board with spaces.md");
    std::fs::write(&board, "old ACK(done\n").unwrap();
    let mut child = cli()
        .args(["olp", "--bash"])
        .arg(bash())
        .args(["watch"])
        .arg(&board)
        .args(["ACK(done", "--interval", "1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(15) {
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &child.id().to_string()])
                    .output();
            }
            let _ = child.kill();
            let _ = child.wait();
            panic!("watch did not signal within 15 seconds");
        }
        std::thread::sleep(Duration::from_millis(100));
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&board)
            .unwrap();
        writeln!(file, "new ACK(done").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("BOARD-SIGNAL: ACK(done"), "{text}");
    assert!(text.contains("new ACK(done"), "{text}");
    assert!(!text.contains("old ACK(done"), "{text}");
}
