use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("路径缺少父目录")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn encrypt(text: &str, key: &[u8; 32]) -> Result<String> {
    let mut iv = [0; 12];
    OsRng.fill_bytes(&mut iv);
    let mut sealed = Aes256Gcm::new_from_slice(key)
        .unwrap()
        .encrypt(Nonce::from_slice(&iv), text.as_bytes())
        .map_err(|_| anyhow::anyhow!("配置加密失败"))?;
    let tag = sealed.split_off(sealed.len() - 16);
    Ok(format!(
        "{}:{}:{}",
        STANDARD.encode(iv),
        STANDARD.encode(tag),
        STANDARD.encode(sealed)
    ))
}
pub fn decrypt(text: &str, key: &[u8; 32]) -> Result<String> {
    let parts: Vec<_> = text.split(':').collect();
    if parts.len() != 3 {
        bail!("加密配置格式错误")
    }
    let iv = STANDARD.decode(parts[0])?;
    if iv.len() != 12 {
        bail!("加密配置 IV 格式错误")
    }
    let tag = STANDARD.decode(parts[1])?;
    if tag.len() != 16 {
        bail!("加密配置标签格式错误")
    }
    let mut data = STANDARD.decode(parts[2])?;
    data.extend(tag);
    let text = Aes256Gcm::new_from_slice(key)
        .unwrap()
        .decrypt(Nonce::from_slice(&iv), data.as_slice())
        .map_err(|_| anyhow::anyhow!("无法解密配置：密钥不匹配或数据已损坏"))?;
    Ok(String::from_utf8(text)?)
}
pub fn encrypt_export(text: &str, password: &str) -> Result<String> {
    if password.chars().count() < 8 {
        bail!("导出密码至少需要 8 个字符")
    }
    let mut salt = [0; 16];
    OsRng.fill_bytes(&mut salt);
    let mut key = [0; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, 600_000, &mut key);
    Ok(format!(
        "export1:{}:{}",
        STANDARD.encode(salt),
        encrypt(text, &key)?
    ))
}
pub fn decrypt_export(text: &str, password: &str) -> Result<String> {
    let payload = text.strip_prefix("export1:").context("未知的导出版本")?;
    let (salt, sealed) = payload.split_once(':').context("导出格式错误")?;
    let salt = STANDARD.decode(salt)?;
    if salt.len() != 16 {
        bail!("导出盐值格式错误")
    }
    let mut key = [0; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, 600_000, &mut key);
    decrypt(sealed, &key).context("导出密码错误或文件已损坏")
}
fn key(service: &str, create: bool) -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(service, "aes-key")?;
    let value = match entry.get_password() {
        Ok(v) => v,
        Err(keyring::Error::NoEntry) if create => {
            let mut k = [0; 32];
            OsRng.fill_bytes(&mut k);
            let s = hex::encode(k);
            entry.set_password(&s)?;
            s
        }
        Err(e) => return Err(e).context("系统钥匙串不可用；没有将配置降级为明文保存"),
    };
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("系统钥匙串密钥长度错误"))
}
pub fn legacy_decrypt(text: &str) -> Result<String> {
    if text.starts_with("rust1:") {
        return decrypt(&text[6..], &key("local.ocs.rust", false)?);
    }
    if text.contains(':') {
        return decrypt(text, &key("ocs-desktop", false)?);
    }
    #[cfg(target_os = "macos")]
    {
        use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
        let bytes = STANDARD.decode(text)?;
        if !bytes.starts_with(b"v10") {
            bail!("此旧配置不是 macOS Electron v10 格式")
        }
        for name in ["ocs-desktop", "OCS Desktop", "OCS", "ocs"] {
            if let Ok(secret) =
                keyring::Entry::new(&format!("{name} Safe Storage"), name)?.get_password()
            {
                let mut k = [0; 16];
                pbkdf2::pbkdf2_hmac::<sha1::Sha1>(secret.as_bytes(), b"saltysalt", 1003, &mut k);
                if let Ok(raw) = cbc::Decryptor::<aes::Aes128>::new(&k.into(), &[32; 16].into())
                    .decrypt_padded_vec_mut::<Pkcs7>(&bytes[3..])
                {
                    if let Ok(v) = String::from_utf8(raw) {
                        if serde_json::from_str::<Value>(&v).is_ok() {
                            return Ok(v);
                        }
                    }
                }
            }
        }
    }
    bail!("旧版系统加密配置无法在此账户解密。请在原机器的旧版 OCS 中导出无加密配置，再通过导入功能迁移。")
}

pub struct Storage {
    pub root: PathBuf,
    pub resources: PathBuf,
    pub value: Mutex<Value>,
    key: [u8; 32],
    grants: Mutex<HashSet<PathBuf>>,
}
impl Storage {
    pub fn open(root: PathBuf, resources: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let k = key("local.ocs.rust", true)?;
        Self::with_key(root, resources, k)
    }
    pub fn with_key(root: PathBuf, resources: PathBuf, k: [u8; 32]) -> Result<Self> {
        fs::create_dir_all(&root)?;
        for folder in ["logs", "downloads/extensions", "userDataDirs"] {
            fs::create_dir_all(root.join(folder))?;
        }
        let file = root.join("config.json");
        let mut v = if file.exists() {
            let mut v: Value =
                serde_json::from_slice(&fs::read(file)?).context("配置文件损坏；原文件已保留")?;
            if let Some(s) = v["render"].as_str() {
                v["render"] = serde_json::from_str(&decrypt(
                    s.strip_prefix("rust1:").context("未知的配置加密版本")?,
                    &k,
                )?)?;
            }
            v
        } else {
            json!({"name":"OCS Rust","version":env!("CARGO_PKG_VERSION"),"app":{"video_frame_rate":1},"window":{"alwaysOnTop":false,"autoLaunch":false},"render":{},"server":{}})
        };
        if !v["paths"].is_object() {
            v["paths"] = json!({});
        }
        for (name, p) in [
            ("app-path", resources.clone()),
            ("user-data-path", root.clone()),
            ("exe-path", std::env::current_exe()?),
            ("logs-path", root.join("logs")),
            ("config-path", root.join("config.json")),
            ("userDataDirsFolder", root.join("userDataDirs")),
            ("downloadFolder", root.join("downloads")),
            ("extensionsFolder", root.join("downloads/extensions")),
        ] {
            if !["userDataDirsFolder", "downloadFolder", "extensionsFolder"].contains(&name)
                || !v["paths"][name].is_string()
            {
                v["paths"][name] = json!(p);
            }
        }
        v["name"] = json!("OCS Rust");
        v["version"] = json!(env!("CARGO_PKG_VERSION"));
        // A packaged app may move independently of its data directory.
        if let Some(scripts) = v
            .get_mut("render")
            .and_then(|render| render.get_mut("scripts"))
            .and_then(Value::as_array_mut)
        {
            for script in scripts {
                if script["isLocalScript"] == true
                    && script["url"].as_str().is_some_and(|p| {
                        Path::new(p)
                            .file_name()
                            .is_some_and(|n| n == "ocs-rust.user.js")
                    })
                {
                    let path = resources.join("assets/ocs-rust.user.js");
                    for field in ["url", "code_url"] {
                        script["info"][field] = json!(path);
                    }
                    script["url"] = json!(path);
                }
            }
        }
        let mut grants = HashSet::from([canonical_missing(&root)?, canonical_missing(&resources)?]);
        for n in ["userDataDirsFolder", "downloadFolder", "extensionsFolder"] {
            if let Some(p) = v["paths"][n].as_str() {
                grants.insert(canonical_missing(Path::new(p))?);
            }
        }
        Ok(Self {
            root,
            resources,
            value: Mutex::new(v),
            key: k,
            grants: Mutex::new(grants),
        })
    }
    pub fn encrypt(&self, text: &str) -> Result<String> {
        Ok(format!("rust1:{}", encrypt(text, &self.key)?))
    }
    pub fn decrypt(&self, text: &str) -> Result<String> {
        if let Some(s) = text.strip_prefix("rust1:") {
            decrypt(s, &self.key)
        } else {
            legacy_decrypt(text)
        }
    }
    pub fn grant(&self, path: &Path) -> Result<()> {
        self.grants.lock().unwrap().insert(canonical_missing(path)?);
        Ok(())
    }
    pub fn check(&self, path: &Path, write: bool) -> Result<PathBuf> {
        let p = canonical_missing(path)?;
        let root = canonical_missing(&self.root)?;
        if write
            && (p == root
                || p.starts_with(canonical_missing(&self.resources)?)
                || p == root.join("config.json"))
        {
            bail!("此路径由配置管理器保护")
        }
        if self.grants.lock().unwrap().iter().any(|g| p.starts_with(g)) {
            Ok(p)
        } else {
            bail!("路径未授权，请通过文件选择器选择：{}", path.display())
        }
    }
    pub fn snapshot(&self) -> Value {
        self.value.lock().unwrap().clone()
    }
    pub fn save(&self, mut value: Value) -> Result<()> {
        if !value.is_object() || !value["render"].is_object() {
            bail!("配置必须包含 render 对象")
        }
        let mut guard = self.value.lock().unwrap();
        // Read-only runtime paths and the per-run API capability cannot be overwritten by an import.
        for name in [
            "app-path",
            "user-data-path",
            "exe-path",
            "logs-path",
            "config-path",
        ] {
            value["paths"][name] = guard["paths"][name].clone();
        }
        for name in ["userDataDirsFolder", "downloadFolder", "extensionsFolder"] {
            let p = value["paths"][name].as_str().context("数据目录为空")?;
            self.check(Path::new(p), false)?;
        }
        value["server"] = guard["server"].clone();
        let mut disk = value.clone();
        disk["render"] = json!(self.encrypt(&serde_json::to_string(&value["render"])?)?);
        atomic_write(
            &self.root.join("config.json"),
            &serde_json::to_vec_pretty(&disk)?,
        )?;
        *guard = value;
        Ok(())
    }
}
pub fn canonical_missing(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("必须使用绝对路径")
    }
    let mut normalized = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            _ => normalized.push(c),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name().context("路径无效")?.to_owned());
        ancestor = ancestor.parent().context("路径无效")?;
    }
    let mut result = ancestor.canonicalize()?;
    for part in suffix.into_iter().rev() {
        result.push(part)
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tamper_and_atomic_roundtrip() {
        let k = [7; 32];
        let s = encrypt("中文秘密", &k).unwrap();
        assert_eq!(decrypt(&s, &k).unwrap(), "中文秘密");
        assert!(decrypt(&s, &[8; 32]).is_err());
        let t = tempfile::tempdir().unwrap();
        let store = Storage::with_key(t.path().into(), t.path().join("resources"), k).unwrap();
        let mut v = store.snapshot();
        v["render"] = json!({"password":"secret"});
        store.save(v).unwrap();
        let bytes = fs::read_to_string(t.path().join("config.json")).unwrap();
        assert!(!bytes.contains("secret"));
        let next = Storage::with_key(t.path().into(), t.path().join("resources"), k).unwrap();
        assert_eq!(next.snapshot()["render"]["password"], "secret");
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_denied() {
        let t = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let s = Storage::with_key(t.path().into(), t.path().join("resources"), [1; 32]).unwrap();
        std::os::unix::fs::symlink(outside.path(), t.path().join("escape")).unwrap();
        assert!(s.check(&t.path().join("escape/file"), true).is_err());
        assert!(s.check(&t.path().join("config.json"), true).is_err());
    }
    #[test]
    fn bundled_script_follows_relocated_app() {
        let t = tempfile::tempdir().unwrap();
        let store = Storage::with_key(t.path().into(), t.path().join("old-app"), [1; 32]).unwrap();
        let mut v = store.snapshot();
        v["render"]["scripts"] = json!([
            {"isLocalScript":true,"url":"/old-app/assets/ocs-rust.user.js","info":{}},
            {"isLocalScript":true,"url":"/personal/custom.user.js","info":{}}
        ]);
        store.save(v).unwrap();
        let moved = Storage::with_key(t.path().into(), t.path().join("new-app"), [1; 32]).unwrap();
        assert_eq!(
            moved.snapshot()["render"]["scripts"][0]["url"],
            json!(t.path().join("new-app").join("assets/ocs-rust.user.js"))
        );
        assert_eq!(
            moved.snapshot()["render"]["scripts"][1]["url"],
            "/personal/custom.user.js"
        );
    }
    #[test]
    fn portable_export_uses_password_without_machine_key() {
        let text = r#"{"browser":{"name":"跨机器资料"}}"#;
        let sealed = encrypt_export(text, "fixture-password").unwrap();
        assert!(sealed.starts_with("export1:"));
        assert!(!sealed.contains("跨机器资料"));
        assert_eq!(decrypt_export(&sealed, "fixture-password").unwrap(), text);
        assert!(decrypt_export(&sealed, "wrong-password").is_err());
        assert!(encrypt_export(text, "short").is_err());
    }
}
