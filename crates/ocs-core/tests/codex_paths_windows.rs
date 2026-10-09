#![cfg(windows)]

#[path = "../../../src-tauri/src/codex_paths.rs"]
mod codex_paths;

use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn binary(root: &Path, name: &str, age: u64) -> std::path::PathBuf {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = fs::File::create(&path).unwrap();
    file.set_times(
        fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(age)),
    )
    .unwrap();
    path
}

#[test]
fn desktop_update_uses_latest_versioned_binary_not_the_stale_root_copy() {
    let dir = tempfile::tempdir().unwrap();
    binary(dir.path(), "codex.exe", 100);
    let old = binary(dir.path(), "1111111111111111/codex.exe", 200);
    let new = binary(dir.path(), "2222222222222222/codex.exe", 300);
    binary(dir.path(), "unrelated/codex.exe", 400);
    assert_eq!(codex_paths::windows_app_executable(dir.path()), Some(new));
    fs::remove_file(old).unwrap();
    assert!(codex_paths::windows_app_executable(dir.path())
        .unwrap()
        .is_file());
}

#[test]
fn unversioned_install_is_used_only_when_no_versioned_binary_exists() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(codex_paths::windows_app_executable(dir.path()), None);
    fs::create_dir_all(dir.path().join("1111111111111111")).unwrap();
    let root = binary(dir.path(), "codex.exe", 100);
    assert_eq!(codex_paths::windows_app_executable(dir.path()), Some(root));
}

#[test]
fn explicit_existing_program_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let custom = binary(dir.path(), "custom/codex.exe", 100);
    let detected = codex_paths::detect(custom.to_str().unwrap(), dir.path().to_str().unwrap());
    assert_eq!(
        fs::canonicalize(detected["codex_bin"].as_str().unwrap()).unwrap(),
        fs::canonicalize(custom).unwrap()
    );
}

#[test]
fn persisted_npm_fallback_does_not_hide_the_desktop_installation() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("desktop/bin");
    let npm = binary(dir.path(), "roaming/npm/codex.cmd", 100);
    let desktop = binary(&bin, "1111111111111111/codex.exe", 200);
    assert_eq!(
        codex_paths::windows_preferred_executable(Some(npm.clone()), &bin, Some(&npm)),
        Some(desktop)
    );
}

#[test]
fn unversioned_desktop_fallback_moves_to_the_current_installation() {
    let dir = tempfile::tempdir().unwrap();
    let root = binary(dir.path(), "codex.exe", 100);
    let desktop = binary(dir.path(), "1111111111111111/codex.exe", 200);
    assert_eq!(
        codex_paths::windows_preferred_executable(Some(root), dir.path(), None),
        Some(desktop)
    );
}

#[test]
fn custom_command_is_preserved_even_when_desktop_codex_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let npm = binary(dir.path(), "roaming/npm/codex.cmd", 100);
    let custom = binary(dir.path(), "custom/npm/codex.cmd", 100);
    binary(dir.path(), "1111111111111111/codex.exe", 200);
    assert_eq!(
        codex_paths::windows_preferred_executable(Some(custom.clone()), dir.path(), Some(&npm)),
        Some(custom)
    );
}

#[test]
fn npm_fallback_is_retained_when_no_desktop_installation_exists() {
    let dir = tempfile::tempdir().unwrap();
    let npm = binary(dir.path(), "npm/codex.cmd", 100);
    assert_eq!(
        codex_paths::windows_preferred_executable(Some(npm.clone()), dir.path(), Some(&npm)),
        Some(npm)
    );
}

#[test]
fn stored_npm_path_with_different_case_and_separators_is_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let npm = binary(dir.path(), "roaming/npm/codex.cmd", 100);
    let stored = std::path::PathBuf::from(npm.to_string_lossy().replace('\\', "/").to_uppercase());
    let desktop = binary(dir.path(), "1111111111111111/codex.exe", 200);
    assert_eq!(
        codex_paths::windows_preferred_executable(Some(stored), dir.path(), Some(&npm)),
        Some(desktop)
    );
}

#[test]
fn startup_detection_recovers_an_existing_npm_path() {
    const EXPECTED: &str = "OCS_CODEX_PATH_TEST_EXPECTED";
    const CONFIGURED: &str = "OCS_CODEX_PATH_TEST_CONFIGURED";
    if let Ok(expected) = std::env::var(EXPECTED) {
        let configured = std::env::var(CONFIGURED).unwrap();
        let detected = codex_paths::detect(&configured, "");
        assert_eq!(
            fs::canonicalize(detected["codex_bin"].as_str().unwrap()).unwrap(),
            fs::canonicalize(expected).unwrap()
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let roaming = dir.path().join("Roaming");
    let local = dir.path().join("Local");
    let npm = binary(&roaming, "npm/codex.cmd", 100);
    let desktop = binary(&local, "OpenAI/Codex/bin/1111111111111111/codex.exe", 200);
    // Use a child harness so changing Windows paths cannot race other tests.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "startup_detection_recovers_an_existing_npm_path"])
        .env("APPDATA", roaming)
        .env("LOCALAPPDATA", local)
        .env(CONFIGURED, npm)
        .env(EXPECTED, desktop)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
