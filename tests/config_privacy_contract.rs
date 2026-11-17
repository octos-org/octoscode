//! Config publication and diagnostic-output behavior through public entrypoints.
use std::fs;
use std::path::Path;
use std::process::Command;

use octoscode::cli::merge_into_config;
use serde_json::{Map, Value, json};

fn changes() -> Map<String, Value> {
    Map::from_iter([("theme".into(), json!("claude"))])
}

fn config_command(path: &Path, show_secrets: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_octoscode"));
    command.env("OCTOS_LANG", "en");
    command.args(["config", "show", "--config"]).arg(path);
    if show_secrets {
        command.arg("--show-secrets");
    }
    command.output().expect("run config show")
}

#[test]
fn config_show_masks_tokens_without_changing_saved_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let original = json!({
        "auth-token": "dummy-canonical-token",
        "auth_token": "dummy-legacy-token",
        "theme": "codex",
        "extra": [{"auth-token": "dummy-nested-token", "untouched": 7}]
    });
    fs::write(&path, original.to_string()).unwrap();

    let output = config_command(&path, false);
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    for token in [
        "dummy-canonical-token",
        "dummy-legacy-token",
        "dummy-nested-token",
    ] {
        assert!(!output.contains(token));
    }
    assert!(output.contains("[REDACTED]"));
    assert!(output.contains("codex"));
    assert!(output.contains("untouched"));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),
        original
    );
}

#[test]
fn config_show_requires_explicit_opt_in_for_original_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, r#"{"auth-token":"dummy-visible-token"}"#).unwrap();
    let output = config_command(&path, true);
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("dummy-visible-token")
    );
}

#[test]
fn config_show_does_not_fall_back_to_raw_malformed_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, r#"{"auth-token":"dummy-malformed-token", invalid}"#).unwrap();
    let output = config_command(&path, false);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("dummy-malformed-token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("dummy-malformed-token"));
}

#[test]
fn config_save_replaces_the_file_instead_of_truncating_the_old_inode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let previous = dir.path().join("previous-config.json");
    let original = r#"{"theme":"codex","auth-token":"dummy-saved-token","unknown":9}"#;
    fs::write(&path, original).unwrap();
    fs::hard_link(&path, &previous).unwrap();

    merge_into_config(&path, &changes()).unwrap();

    assert_eq!(fs::read_to_string(previous).unwrap(), original);
    let updated: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(updated["theme"], "claude");
    assert_eq!(updated["auth-token"], "dummy-saved-token");
    assert_eq!(updated["unknown"], 9);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no temporary file remains"
    );
}

#[test]
fn config_save_failure_keeps_read_only_original_and_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let original = r#"{"theme":"codex","auth-token":"dummy-preserved-token"}"#;
    fs::write(&path, original).unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut read_only = original_permissions.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&path, read_only).unwrap();

    let result = merge_into_config(&path, &changes());
    let unchanged = fs::read_to_string(&path).unwrap();
    let entries = fs::read_dir(dir.path()).unwrap().count();
    // Restore the original permission object rather than set_readonly(false),
    // which would grant everyone write access on Unix.
    fs::set_permissions(&path, original_permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(unchanged, original);
    assert_eq!(entries, 1);
}

#[cfg(windows)]
#[test]
fn config_publish_failure_preserves_original_and_cleans_temporary_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let original = r#"{"theme":"codex","auth-token":"dummy-original-token"}"#;
    fs::write(&path, original).unwrap();
    // Permit reads/writes but deny deletion/rename while the old file is
    // open. Reading and writing the temporary sibling succeeds; publication
    // must fail without truncating the active config.
    let blocker = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&path)
        .unwrap();
    assert!(merge_into_config(&path, &changes()).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    drop(blocker);
}

#[cfg(unix)]
#[test]
fn saved_config_is_private_for_new_and_existing_files() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for existing in [false, true] {
        let path = dir.path().join(format!("config-{existing}.json"));
        if existing {
            fs::write(&path, r#"{"auth_token":"dummy-legacy-token"}"#).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        merge_into_config(&path, &changes()).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn config_save_preserves_user_managed_symlink() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real-config.json");
    let path = dir.path().join("config.json");
    fs::write(&target, r#"{"theme":"codex"}"#).unwrap();
    symlink(&target, &path).unwrap();
    merge_into_config(&path, &changes()).unwrap();
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let updated: Value = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
    assert_eq!(updated["theme"], "claude");
}
