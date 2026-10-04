use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    }
}
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("OCS-Rust/0.1")
        .build()
        .unwrap()
}
pub fn web_url(s: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(s)?;
    if !["http", "https"].contains(&u.scheme()) {
        bail!("仅允许 HTTP 或 HTTPS 地址")
    }
    Ok(u)
}
pub async fn fetch(url: &str, options: &Value, status: bool) -> Result<Value> {
    let mut req = client().get(web_url(url)?);
    if let Some(headers) = options["headers"].as_object() {
        for (k, v) in headers {
            if let Some(s) = v.as_str() {
                req = req.header(k, s);
            }
        }
    }
    if let Some(params) = options["params"].as_object() {
        req = req.query(params);
    }
    let response = req.send().await?;
    let code = response.status().as_u16();
    if !status && !response.status().is_success() {
        bail!("HTTP {code}")
    }
    let bytes = response.bytes().await?;
    let data =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    Ok(if status {
        json!({"status":code,"data":data})
    } else {
        data
    })
}
pub async fn download(url: &str, dest: &Path, progress: impl Fn(f64, u64, u64)) -> Result<()> {
    let response = client()
        .get(web_url(url)?)
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let parent = dest.parent().context("下载路径无父目录")?;
    fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut out = tokio::fs::File::from_std(temp.reopen()?);
    let mut stream = response.bytes_stream();
    let mut done = 0;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        out.write_all(&bytes).await?;
        done += bytes.len() as u64;
        progress(
            if total > 0 {
                done as f64 / total as f64
            } else {
                0.0
            },
            total,
            done,
        );
    }
    out.sync_all().await?;
    drop(out);
    temp.persist(dest).map_err(|e| e.error)?;
    progress(1.0, total, done);
    Ok(())
}
pub fn unzip(input: &Path, output: &Path) -> Result<()> {
    let parent = output.parent().context("解压路径无父目录")?;
    fs::create_dir_all(parent)?;
    let temp = tempfile::tempdir_in(parent)?;
    let temp_root = temp.path().canonicalize()?;
    let mut archive = zip::ZipArchive::new(fs::File::open(input)?)?;
    let mut links = Vec::new();
    let mut total = 0u64;
    if archive.len() > 100000 {
        bail!("压缩包包含过多文件")
    }
    for i in 0..archive.len() {
        let mut f = archive.by_index(i)?;
        total = total.checked_add(f.size()).context("压缩包大小溢出")?;
        if total > 8 * 1024 * 1024 * 1024 {
            bail!("压缩包解压尺寸超出限制")
        }
        let relative = f.enclosed_name().context("压缩包包含目录穿越路径")?;
        if f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            use std::io::Read;
            if f.size() > 4096 {
                bail!("符号链接目标过长")
            }
            let mut target = String::new();
            f.read_to_string(&mut target)?;
            if Path::new(&target).is_absolute() {
                bail!("压缩包符号链接不能指向绝对路径")
            }
            links.push((relative, target));
            continue;
        }
        let dest = temp.path().join(relative);
        if f.is_dir() {
            fs::create_dir_all(dest)?;
        } else {
            fs::create_dir_all(dest.parent().unwrap())?;
            let mut out = fs::File::create(&dest)?;
            std::io::copy(&mut f, &mut out)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    dest,
                    fs::Permissions::from_mode(f.unix_mode().unwrap_or(0o644) & 0o777),
                )?;
            }
        }
    }
    for (relative, target) in &links {
        let dest = temp.path().join(relative);
        fs::create_dir_all(dest.parent().unwrap())?;
        let resolved = crate::storage::canonical_missing(&dest.parent().unwrap().join(target))?;
        if !resolved.starts_with(&temp_root) {
            bail!("压缩包符号链接越过目标目录")
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &dest)?;
        #[cfg(windows)]
        {
            if resolved.is_dir() {
                std::os::windows::fs::symlink_dir(target, &dest)?
            } else {
                std::os::windows::fs::symlink_file(target, &dest)?
            }
        }
    }
    for (relative, _) in &links {
        if !temp
            .path()
            .join(relative)
            .canonicalize()?
            .starts_with(&temp_root)
        {
            bail!("压缩包符号链接越过目标目录")
        }
    }
    if output.exists() {
        bail!("目标目录已存在，未覆盖原有数据")
    }
    fs::rename(temp.path(), output)?;
    Ok(())
}
pub fn folder_size(path: &Path) -> Result<u64> {
    let mut size = 0;
    for f in walkdir::WalkDir::new(path).follow_links(false) {
        let f = f?;
        if f.file_type().is_file() {
            size += f.metadata()?.len();
        }
    }
    Ok(size)
}
pub fn valid_browsers(root: &Path, resources: &Path) -> Vec<Value> {
    let mut candidates: Vec<PathBuf> = vec![];
    for dir in [
        root.join("browser"),
        resources.join("browser"),
        PathBuf::from("/Applications/OCS Desktop.app/Contents/Resources/bin"),
    ] {
        if dir.is_dir() {
            for entry in walkdir::WalkDir::new(dir)
                .max_depth(9)
                .into_iter()
                .filter_map(Result::ok)
            {
                if ["Google Chrome for Testing", "chrome.exe", "chrome"]
                    .contains(&entry.file_name().to_string_lossy().as_ref())
                    && entry.file_type().is_file()
                {
                    candidates.push(entry.into_path());
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        candidates.extend(
            [
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ]
            .map(PathBuf::from),
        );
    }
    if cfg!(windows) {
        for env in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(env) {
                for tail in [
                    "Google/Chrome/Application/chrome.exe",
                    "Microsoft/Edge/Application/msedge.exe",
                ] {
                    candidates.push(PathBuf::from(&base).join(tail));
                }
            }
        }
    }
    if cfg!(target_os = "linux") {
        candidates.extend(
            [
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
                "/usr/bin/google-chrome",
            ]
            .map(PathBuf::from),
        );
    }
    candidates
        .into_iter()
        .filter(|p| p.is_file())
        .map(|p| json!({"name":p.file_name().unwrap_or_default().to_string_lossy(),"path":p}))
        .collect()
}
pub fn browser_version(path: &Path) -> Result<Option<u64>> {
    if !path.is_file() {
        return Ok(None);
    }
    if cfg!(target_os = "macos") {
        let p = path
            .parent()
            .context("无效浏览器路径")?
            .join("../Info.plist");
        if p.exists() {
            let info = plist::Value::from_file(p)?;
            if let Some(v) = info
                .as_dictionary()
                .and_then(|v| v.get("CFBundleShortVersionString"))
                .and_then(plist::Value::as_string)
            {
                return Ok(v.split('.').next().and_then(|v| v.parse().ok()));
            }
        }
    }
    for f in fs::read_dir(path.parent().context("无效浏览器路径")?)? {
        let f = f?;
        let n = f.file_name().to_string_lossy().to_string();
        if n.contains('.') && n.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if let Ok(v) = n.split('.').next().unwrap().parse() {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}
pub fn export_excel(sheets: &Value, path: &Path) -> Result<()> {
    let mut book = rust_xlsxwriter::Workbook::new();
    for data in sheets.as_array().context("工作表列表格式无效")? {
        let rows = data["list"].as_array().context("数据行格式无效")?;
        let sheet = book.add_worksheet();
        sheet.set_name(data["sheetName"].as_str().unwrap_or("Sheet1"))?;
        let mut keys = Vec::<String>::new();
        for row in rows {
            if let Some(o) = row.as_object() {
                for key in o.keys() {
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
            }
        }
        for (c, k) in keys.iter().enumerate() {
            sheet.write_string(0, c as u16, k)?;
            for (r, row) in rows.iter().enumerate() {
                let v = &row[k];
                if let Some(n) = v.as_f64() {
                    sheet.write_number((r + 1) as u32, c as u16, n)?;
                } else {
                    sheet.write_string(
                        (r + 1) as u32,
                        c as u16,
                        v.as_str().map(str::to_string).unwrap_or_else(|| {
                            if v.is_null() {
                                String::new()
                            } else {
                                v.to_string()
                            }
                        }),
                    )?;
                }
            }
        }
    }
    crate::storage::atomic_write(path, &book.save_to_buffer()?)
}
