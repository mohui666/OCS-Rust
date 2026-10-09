use serde_json::{json, Value};
use std::{env, fs, path::Path, path::PathBuf};

fn expanded_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = if value == "~" {
        dirs::home_dir()?
    } else if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        dirs::home_dir()?.join(rest)
    } else {
        PathBuf::from(value)
    };
    std::path::absolute(path).ok()
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ["exe", "com", "cmd", "bat"]
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            })
    }
}

fn from_search_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let mut names = vec![name.to_owned()];
    if cfg!(windows) && Path::new(name).extension().is_none() {
        names.extend(["exe", "com", "cmd", "bat"].map(|ext| format!("{name}.{ext}")));
    }
    env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| is_executable(path))
}

#[cfg(windows)]
pub(crate) fn windows_app_executable(bin: &Path) -> Option<PathBuf> {
    let mut versions: Vec<_> = fs::read_dir(bin)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            (8..=64).contains(&name.len()) && name.bytes().all(|c| c.is_ascii_hexdigit())
        })
        .map(|entry| entry.path().join("codex.exe"))
        .filter(|path| is_executable(path))
        .collect();
    versions.sort_by_cached_key(|path| {
        std::cmp::Reverse(fs::metadata(path).and_then(|m| m.modified()).ok())
    });
    versions.into_iter().next().or_else(|| {
        let path = bin.join("codex.exe");
        is_executable(&path).then_some(path)
    })
}

#[cfg(windows)]
pub(crate) fn windows_preferred_executable(
    configured: Option<PathBuf>,
    bin: &Path,
    npm: Option<&Path>,
) -> Option<PathBuf> {
    let legacy = configured.as_ref().is_some_and(|current| {
        let current = fs::canonicalize(current).ok();
        npm.into_iter()
            .chain(std::iter::once(bin.join("codex.exe").as_path()))
            .any(|path| current.is_some() && current == fs::canonicalize(path).ok())
    });
    // Previous auto-detection persisted these fallbacks as if they were explicit choices.
    if configured.is_some() && !legacy {
        return configured;
    }
    windows_app_executable(bin).or(configured)
}

fn executable(current: &str) -> Option<PathBuf> {
    let current = current.trim();
    let configured = if current.contains('/') || current.contains('\\') {
        expanded_path(current).filter(|path| is_executable(path))
    } else if !current.is_empty() {
        from_search_path(current)
    } else {
        None
    };
    #[cfg(windows)]
    let configured = if let Some(local) = env::var_os("LOCALAPPDATA") {
        let npm = env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("npm/codex.cmd"));
        windows_preferred_executable(
            configured,
            &PathBuf::from(local).join("OpenAI/Codex/bin"),
            npm.as_deref(),
        )
    } else {
        configured
    };
    if configured.is_some() {
        return configured;
    }

    let mut candidates = Vec::new();
    if cfg!(target_os = "macos") {
        let mut app_dirs = vec![PathBuf::from("/Applications")];
        if let Some(home) = dirs::home_dir() {
            app_dirs.push(home.join("Applications"));
        }
        for dir in app_dirs {
            for binary in [
                "ChatGPT.app/Contents/Resources/codex-cli/bin/codex",
                "Codex.app/Contents/Resources/codex",
                "Codex.app/Contents/Resources/codex-cli/bin/codex",
            ] {
                candidates.push(dir.join(binary));
            }
        }
    }
    if let Some(path) = from_search_path("codex") {
        candidates.push(path);
    }
    if cfg!(unix) {
        candidates.extend(
            [
                "/opt/homebrew/bin/codex",
                "/usr/local/bin/codex",
                "/usr/bin/codex",
            ]
            .map(PathBuf::from),
        );
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/bin/codex"));
        }
    }
    if cfg!(windows) {
        if let Some(app_data) = env::var_os("APPDATA") {
            candidates.push(PathBuf::from(app_data).join("npm/codex.cmd"));
        }
    }
    candidates.into_iter().find(|path| is_executable(path))
}

pub fn detect(current_bin: &str, current_home: &str) -> Value {
    // Inspect only paths and known filenames; never read login credentials or run Codex.
    let mut homes: Vec<_> = expanded_path(current_home).into_iter().collect();
    if let Ok(value) = env::var("CODEX_HOME") {
        homes.extend(expanded_path(&value));
    }
    if let Some(home) = dirs::home_dir() {
        homes.push(home.join(".codex"));
    }
    let home = homes.into_iter().find(|path| {
        path.is_dir()
            && ["auth.json", "config.toml", "models_cache.json"]
                .iter()
                .any(|name| path.join(name).is_file())
    });
    json!({ "codex_bin": executable(current_bin), "codex_home": home })
}
