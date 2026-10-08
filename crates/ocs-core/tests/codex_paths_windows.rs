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
