//! One-way copy into the Rust data directory. The source installation is never changed.
use crate::storage::{canonical_missing, Storage};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn copy_tree(source: &Path, dest: &Path) -> Result<u64> {
    let mut bytes = 0;
    for e in walkdir::WalkDir::new(source).follow_links(false) {
        let e = e?;
        if [
            "SingletonLock",
            "SingletonSocket",
            "SingletonCookie",
            "RunningChromeVersion",
        ]
        .contains(&e.file_name().to_string_lossy().as_ref())
        {
            continue;
        }
        let out = dest.join(e.path().strip_prefix(source)?);
        if e.file_type().is_symlink() {
            bail!("迁移遇到符号链接，未复制：{}", e.path().display())
        }
        if e.file_type().is_dir() {
            fs::create_dir_all(out)?;
        } else if e.file_type().is_file() {
            fs::create_dir_all(out.parent().unwrap())?;
            bytes += fs::copy(e.path(), out)?;
        }
    }
    Ok(bytes)
}
fn ensure_closed(profile: &Path) -> Result<()> {
    let lock = profile.join("SingletonLock");
    if let Ok(target) = fs::read_link(lock) {
        if let Some(pid) = target
            .to_string_lossy()
            .rsplit('-')
            .next()
            .and_then(|v| v.parse::<i32>().ok())
        {
            #[cfg(unix)]
            if unsafe { libc::kill(pid, 0) } == 0 {
                bail!("请先关闭旧版浏览器，避免复制正在写入的登录资料")
            }
        }
    }
    Ok(())
}
pub fn import_legacy(storage: &Storage, config_file: &Path) -> Result<Value> {
    let source_root = config_file
        .parent()
        .context("旧配置路径无效")?
        .canonicalize()?;
    if source_root == storage.root.canonicalize()? {
        bail!("不能从当前 Rust 配置目录导入自身")
    }
    let mut old: Value = serde_json::from_slice(&fs::read(config_file)?)?;
    if let Some(text) = old["render"].as_str() {
        old["render"] = serde_json::from_str(&storage.decrypt(text)?)?;
    }
    if !old["render"].is_object() {
        bail!("旧配置缺少 render 数据")
    }
    let temp = tempfile::tempdir_in(&storage.root)?;
    let final_root = storage
        .root
        .join(format!("migration-{}", uuid::Uuid::new_v4().simple()));
    let mut count = 0usize;
    let mut bytes = 0u64;
    fn visit(
        node: &mut Value,
        source_root: &Path,
        temp: &Path,
        final_root: &Path,
        count: &mut usize,
        bytes: &mut u64,
    ) -> Result<()> {
        if node["type"] == "browser" {
            let uid = node["uid"].as_str().context("浏览器标识缺失")?;
            if !uid
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                bail!("非法浏览器标识")
            }
            let relative = std::path::PathBuf::from("profiles").join(uid);
            let dest = temp.join(&relative);
            if let Some(p) = node["cachePath"]
                .as_str()
                .filter(|v| *v != "$CACHE_PATH" && !v.is_empty())
            {
                let source = canonical_missing(Path::new(p))?;
                if !source.starts_with(source_root) {
                    bail!(
                        "浏览器资料位于旧配置目录之外，请先通过旧版导出迁移：{}",
                        source.display()
                    )
                }
                if source.exists() {
                    ensure_closed(&source)?;
                    *bytes += copy_tree(&source, &dest)?;
                }
            }
            fs::create_dir_all(&dest)?;
            node["cachePath"] = json!(final_root.join(relative));
            *count += 1;
        }
        if let Some(children) = node["children"].as_object_mut() {
            for child in children.values_mut() {
                visit(child, source_root, temp, final_root, count, bytes)?;
            }
        }
        Ok(())
    }
    visit(
        &mut old["render"]["browser"]["root"],
        &source_root,
        temp.path(),
        &final_root,
        &mut count,
        &mut bytes,
    )?;
    let mut next = storage.snapshot();
    next["render"] = old["render"].clone();
    // Prefer the independently installed browser, so removing the Electron app
    // cannot break the migrated Rust installation.
    if let Some(browser) = crate::platform::valid_browsers(&storage.root, &storage.resources)
        .iter()
        .find(|b| {
            b["path"]
                .as_str()
                .is_some_and(|p| Path::new(p).starts_with(storage.root.join("browser")))
        })
    {
        next["render"]["setting"]["launchOptions"]["executablePath"] = browser["path"].clone();
        next["render"]["setting"]["launchOptions"]["custom"] = json!(true);
    }
    for key in ["app", "window"] {
        if old[key].is_object() {
            next[key] = old[key].clone();
        }
    }
    let extension = old["paths"]["extensionsFolder"].as_str().map(Path::new);
    if let Some(source) = extension.filter(|p| p.is_dir()) {
        let source = source.canonicalize()?;
        if !source.starts_with(&source_root) {
            bail!("拓展目录位于旧配置目录之外，未复制")
        }
        bytes += copy_tree(&source, &temp.path().join("extensions"))?;
        next["paths"]["extensionsFolder"] = json!(final_root.join("extensions"));
    }
    if let Some(scripts) = next["render"]["scripts"].as_array_mut() {
        for script in scripts {
            let name = script["info"]["name"].as_str().unwrap_or("");
            let url = script["url"].as_str().unwrap_or("");
            if name.contains("OCS") || url.contains("ocs-unlimited") {
                let p = storage.resources.join("assets/ocs-rust.user.js");
                script["url"] = json!(p);
                script["info"]["code_url"] = json!(p);
                script["info"]["url"] = json!(p);
                let version = include_str!("../../../assets/ocs-rust.user.js")
                    .lines()
                    .find_map(|line| line.strip_prefix("// @version").map(str::trim))
                    .filter(|version| !version.is_empty())
                    .context("内置用户脚本缺少版本号")?;
                script["info"]["version"] = json!(version);
                script["isLocalScript"] = json!(true);
                script["isInternetLinkScript"] = json!(false);
                script["lastInstalledVersion"] = Value::Null;
            }
        }
    }
    fs::rename(temp.path(), &final_root)?;
    if let Err(e) = storage.save(next) {
        let _ = fs::remove_dir_all(&final_root);
        return Err(e);
    }
    Ok(
        json!({"browsers":count,"copied_bytes":bytes,"source_unchanged":true,"destination":final_root}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copies_profiles_without_modifying_source() {
        let source = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("profiles")).unwrap();
        fs::create_dir(source.path().join("profiles/a")).unwrap();
        fs::write(
            source.path().join("profiles/a/Preferences"),
            b"login fixture",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "137.0.7151.55",
            source.path().join("profiles/a/RunningChromeVersion"),
        )
        .unwrap();
        let old = json!({"render":{"browser":{"root":{"children":{"a":{"type":"browser","uid":"a","cachePath":source.path().join("profiles/a")}}}},"scripts":[]},"paths":{}});
        let config = source.path().join("config.json");
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&config, &bytes).unwrap();
        let storage =
            Storage::with_key(dest.path().into(), dest.path().join("resources"), [2; 32]).unwrap();
        let report = import_legacy(&storage, &config).unwrap();
        assert_eq!(report["browsers"], 1);
        assert_eq!(fs::read(config).unwrap(), bytes);
        let snapshot = storage.snapshot();
        let path = snapshot["render"]["browser"]["root"]["children"]["a"]["cachePath"]
            .as_str()
            .unwrap();
        assert_eq!(
            fs::read(Path::new(path).join("Preferences")).unwrap(),
            b"login fixture"
        );
        assert!(!Path::new(path)
            .join("RunningChromeVersion")
            .symlink_metadata()
            .is_ok());
    }
}
