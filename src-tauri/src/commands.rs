use crate::AppState;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ocs_core::{
    bridge::{Bridge, Config},
    platform,
    storage::atomic_write,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};
use tauri::{Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

fn text(args: &[Value], i: usize) -> Result<&str> {
    args.get(i)
        .and_then(Value::as_str)
        .context("缺少字符串参数")
}
fn value(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}
fn mapped<T>(r: Result<T>) -> std::result::Result<T, String> {
    r.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn bootstrap(app: tauri::AppHandle) -> std::result::Result<Value, String> {
    let mut ready = app.state::<crate::Initialization>().0.subscribe();
    loop {
        let result = ready.borrow().clone();
        if let Some(result) = result {
            result?;
            break;
        }
        ready.changed().await.map_err(|e| e.to_string())?;
    }
    let s = app.try_state::<Arc<AppState>>().ok_or("应用未完成初始化")?;
    let resource = &s.store.resources;
    let meta = if resource.join("adapter/scripts.json").exists() {
        resource.join("adapter/scripts.json")
    } else {
        resource.join("native-adapter/dist/scripts.json")
    };
    mapped((|| {
        Ok(
            json!({"store":s.store.snapshot(),"platform":platform::platform(),"scripts":serde_json::from_slice::<Value>(&fs::read(meta)?)?}),
        )
    })())
}
#[tauri::command]
pub async fn native_call(
    state: State<'_, Arc<AppState>>,
    op: String,
    args: Vec<Value>,
) -> std::result::Result<Value, String> {
    mapped(call(&state, &op, &args).await)
}

async fn dialog(s: &AppState, save: bool, options: &Value) -> Result<Vec<String>> {
    let mut builder = s.app.dialog().file();
    if let Some(v) = options["title"].as_str() {
        builder = builder.set_title(v);
    }
    if let Some(v) = options["defaultPath"].as_str() {
        builder = builder.set_file_name(v);
    }
    if let Some(filters) = options["filters"].as_array() {
        for f in filters {
            if let (Some(n), Some(ext)) = (f["name"].as_str(), f["extensions"].as_array()) {
                let ext: Vec<_> = ext.iter().filter_map(Value::as_str).collect();
                builder = builder.add_filter(n, &ext);
            }
        }
    }
    let folder = options["properties"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v == "openDirectory"));
    let multi = options["properties"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v == "multiSelections"));
    let paths = tauri::async_runtime::spawn_blocking(move || {
        if save {
            builder.blocking_save_file().into_iter().collect::<Vec<_>>()
        } else if folder {
            builder.blocking_pick_folder().into_iter().collect()
        } else if multi {
            builder.blocking_pick_files().unwrap_or_default()
        } else {
            builder.blocking_pick_file().into_iter().collect()
        }
    })
    .await?;
    let mut result = vec![];
    for p in paths {
        let p = p.into_path().map_err(|e| anyhow::anyhow!(e))?;
        s.store.grant(&p)?;
        // Export appends .ocsdata in the original UI. Grant that exact sibling, not its directory.
        if save {
            s.store
                .grant(Path::new(&format!("{}.ocsdata", p.display())))?;
        }
        result.push(p.to_string_lossy().into_owned());
    }
    Ok(result)
}
pub async fn call(s: &Arc<AppState>, op: &str, a: &[Value]) -> Result<Value> {
    let window = || s.app.get_webview_window("main").context("主窗口不存在");
    match op {
        "methods.saveStore"=>{s.store.save(serde_json::from_str(text(a,0)?)?)?;Ok(Value::Null)},
        "methods.isEncryptionAvailable"=>Ok(json!(true)),
        "methods.encryptRenderString"=>Ok(json!(s.store.encrypt(text(a,0)?)?)),
        "methods.decryptRenderString"=>Ok(json!(s.store.decrypt(text(a,0)?)?)),
        "methods.encryptExport"|"methods.decryptExport"=>{
            let data=text(a,0)?.to_owned();let password=text(a,1)?.to_owned();let encrypt=op=="methods.encryptExport";
            Ok(json!(tauri::async_runtime::spawn_blocking(move||if encrypt{ocs_core::storage::encrypt_export(&data,&password)}else{ocs_core::storage::decrypt_export(&data,&password)}).await??))
        },
        "methods.getRawScripts"=>{let p=s.store.resources.join(if s.store.resources.join("adapter").exists(){"adapter/scripts.json"}else{"native-adapter/dist/scripts.json"});Ok(serde_json::from_slice(&fs::read(p)?)?)},
        "methods.importLegacy"=>{
            if !s.workers.active().await.is_empty(){bail!("请先关闭当前 Rust 浏览器")}
            let paths=dialog(s,false,&json!({"title":"选择旧版 OCS 的 config.json（将复制浏览器资料）","filters":[{"name":"JSON","extensions":["json"]}]})).await?;
            let path=PathBuf::from(paths.first().context("已取消迁移")?);let state=s.clone();
            let report=tauri::async_runtime::spawn_blocking(move||ocs_core::migration::import_legacy(&state.store,&path)).await??;
            Ok(json!({"report":report,"store":s.store.snapshot()}))
        },
        "methods.pinnedScript"=>Ok(json!(s.store.resources.join("assets/ocs-rust.user.js"))),
        "os.platform"|"methods.getPlatform"=>Ok(json!(platform::platform())),
        "os.release"=>{
            let mut command=std::process::Command::new(if cfg!(windows){"cmd"}else{"uname"});
            if cfg!(windows){command.args(["/D","/C","ver"]);}else{command.arg("-r");}
            let output=command.output()?;if !output.status.success(){bail!("无法读取系统版本")}
            if cfg!(windows){
                let text=String::from_utf8_lossy(&output.stdout);
                let version=text.split(|c:char| !c.is_ascii_digit()&&c!='.')
                    .find(|part| part.split('.').count()>=3).context("无法解析 Windows 版本")?;
                Ok(json!(version))
            }else{Ok(json!(String::from_utf8(output.stdout)?.trim()))}
        },
        "app.getVersion"=>Ok(json!(env!("CARGO_PKG_VERSION"))),
        "app.getAppPath"=>Ok(json!(s.store.resources)),
        "app.getPath"=>{let p=match text(a,0)?{"logs"=>s.store.root.join("logs"),"userData"=>s.store.root.clone(),"exe"=>std::env::current_exe()?,"downloads"=>dirs::download_dir().context("没有下载目录")?,_=>bail!("不支持的应用路径")};Ok(json!(p))},
        "app.relaunch"=>{s.restarting.store(true,Ordering::SeqCst);s.emit("close",json!([]));Ok(Value::Null)},
        "app.exit"=>{s.stop().await?;s.exit_ready.store(true,Ordering::SeqCst);if s.restarting.load(Ordering::SeqCst){s.app.restart();}else{s.app.exit(0);}Ok(Value::Null)},
        "win.setTitle"=>{window()?.set_title(&format!("{} · Rust",text(a,0)?))?;Ok(Value::Null)},
        "win.setAlwaysOnTop"=>{window()?.set_always_on_top(value(a,0).as_bool().unwrap_or(false))?;Ok(Value::Null)},
        "webContents.openDevTools"=>{window()?.open_devtools();Ok(Value::Null)},
        "methods.autoLaunch"=>{let enabled=value(a,0).as_bool().unwrap_or(false);let manager=s.app.autolaunch();if enabled {manager.enable()?}else if manager.is_enabled()?{manager.disable()?}Ok(Value::Null)},
        "path.join"|"path.resolve"=>{let mut p=PathBuf::new();for v in a{p.push(v.as_str().context("路径参数须为字符串")?);}if !p.is_absolute(){p=s.store.resources.join(p);}Ok(json!(normalize_path(&p)))},
        "fs.existsSync"=>Ok(json!(Path::new(text(a,0)?).exists())),
        "methods.isDirectory"=>Ok(json!(Path::new(text(a,0)?).is_dir())),
        "fs.readdirSync"=>{let p=s.store.check(Path::new(text(a,0)?),false)?;let mut files=fs::read_dir(p)?.map(|e|e.map(|e|e.file_name().to_string_lossy().into_owned())).collect::<std::io::Result<Vec<_>>>()?;files.sort();Ok(json!(files))},
        "fs.readFileSync"|"fs.readFileBytes"=>{let p=s.store.check(Path::new(text(a,0)?),false)?;let bytes=fs::read(p)?;if op.ends_with("Bytes"){Ok(json!(bytes))}else{Ok(json!(String::from_utf8(bytes)?))}},
        "fs.writeFileSync"=>{let p=s.store.check(Path::new(text(a,0)?),true)?;atomic_write(&p,text(a,1)?.as_bytes())?;Ok(Value::Null)},
        "fs.rmSync"|"fs.unlinkSync"=>{let p=s.store.check(Path::new(text(a,0)?),true)?;if !p.exists()&&value(a,1)["force"]==true{return Ok(Value::Null)}if p.is_dir(){if value(a,1)["recursive"]!=true{fs::remove_dir(p)?;}else{fs::remove_dir_all(p)?;}}else{fs::remove_file(p)?;}Ok(Value::Null)},
        "methods.statisticFolderSize"=>{let p=s.store.check(Path::new(text(a,0)?),false)?;Ok(json!(tauri::async_runtime::spawn_blocking(move||platform::folder_size(&p)).await??))},
        "dialog.showOpenDialog"|"dialog.showOpenDialogSync"|"dialog.showSaveDialog"=>{let save=op.ends_with("SaveDialog");let paths=dialog(s,save,&value(a,0)).await?;Ok(if op.ends_with("Sync"){json!(paths)}else if save{json!({"canceled":paths.is_empty(),"filePath":paths.first()})}else{json!({"canceled":paths.is_empty(),"filePaths":paths})})},
        "shell.openExternal"=>{let url=text(a,0)?;platform::web_url(url)?;s.app.opener().open_url(url,None::<&str>)?;Ok(Value::Null)},
        "shell.openPath"|"methods.runInstaller"=>{let path=s.store.check(Path::new(text(a,0)?),false)?;s.app.opener().open_path(path.to_string_lossy(),None::<&str>)?;Ok(Value::Null)},
        "clipboard.writeText"=>{arboard::Clipboard::new()?.set_text(text(a,0)?.to_owned())?;Ok(Value::Null)},
        "clipboard.readText"=>Ok(json!(arboard::Clipboard::new()?.get_text()?)),
        "methods.get"|"methods.getWithStatus"=>platform::fetch(text(a,0)?,&value(a,1),op.ends_with("WithStatus")).await,
        "OCSApi.getInfos"=>platform::fetch("https://cdn.ocsjs.com/api/ocs-app-infos.json",&Value::Null,false).await,
        "methods.download"=>{let path=s.store.check(Path::new(text(a,2)?),true)?;let channel=text(a,0)?;platform::download(text(a,1)?,&path,|r,t,n|s.emit("download",json!([channel,r,t,n]))).await?;Ok(json!(path))},
        "methods.unzip"=>{let input=s.store.check(Path::new(text(a,0)?),false)?;let output=s.store.check(Path::new(text(a,1)?),true)?;tauri::async_runtime::spawn_blocking(move||platform::unzip(&input,&output)).await??;Ok(Value::Null)},
        "methods.getValidBrowsers"=>{let browsers=platform::valid_browsers(&s.store.root,&s.store.resources);for b in &browsers{if let Some(p)=b["path"].as_str(){s.store.grant(Path::new(p))?;}}Ok(json!(browsers))},
        "methods.getBrowserMajorVersion"=>Ok(json!(platform::browser_version(Path::new(text(a,0)?))?)),
        "methods.exportExcel"=>{let paths=dialog(s,true,&json!({"title":"导出 Excel","defaultPath":text(a,1)?,"filters":[{"name":"Excel","extensions":["xlsx"]}]})).await?;if let Some(p)=paths.first(){platform::export_excel(&value(a,0),Path::new(p))?;}Ok(Value::Null)},
        "methods.updateApp"=>bail!("这是 OCS Rust 独立版本。上游 Electron 安装包不能覆盖此应用；请使用本项目构建的新版程序。"),
        "methods.installBrowser"=>install_browser(s).await,
        "methods.launchBrowserOnly"=>launch_only(s,a).await,
        "methods.captureDesktopScreen"=>{let sources=tauri::async_runtime::spawn_blocking(||->Result<Value>{Ok(json!(xcap::Window::all()?.into_iter().filter_map(|w|Some(json!({"id":w.id().ok()?,"name":w.title().ok()?}))).collect::<Vec<_>>()))}).await??;Ok(sources)},
        "methods.captureWindow"=>{let id=value(a,0).as_u64().context("缺少窗口 ID")?;let bytes=tauri::async_runtime::spawn_blocking(move||->Result<Vec<u8>>{let w=xcap::Window::all()?.into_iter().find(|w|w.id().ok()==Some(id as u32)).context("被监控窗口已关闭")?;let img=w.capture_image()?;let mut bytes=vec![];image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes,65).encode_image(&image::DynamicImage::ImageRgba8(img))?;Ok(bytes)}).await??;Ok(json!(format!("data:image/jpeg;base64,{}",STANDARD.encode(bytes))))},
        "logger.info"|"logger.error"|"logger.warn"|"logger.log"=>{let p=s.store.root.join("logs").join(format!("{}.log",chrono::Local::now().format("%Y-%m-%d")));let mut text=a.iter().map(|v|v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string())).collect::<Vec<_>>().join(" ");let snap=s.store.snapshot();if let Some(token)=snap["server"]["authToken"].as_str(){text=text.replace(token,"[redacted]");}let mut file=fs::OpenOptions::new().create(true).append(true).open(p)?;writeln!(file,"{} {} {}",chrono::Local::now().to_rfc3339(),op,text.chars().take(8192).collect::<String>())?;Ok(Value::Null)},
        _=>bail!("未允许的本地命令：{op}"),
    }
}
fn normalize_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            _ => out.push(c),
        }
    }
    out
}
async fn install_browser(s: &Arc<AppState>) -> Result<Value> {
    let info=platform::fetch("https://googlechromelabs.github.io/chrome-for-testing/latest-versions-per-milestone-with-downloads.json",&Value::Null,false).await?;
    let platform_name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-arm64",
        ("macos", _) => "mac-x64",
        ("windows", "x86_64") => "win64",
        ("windows", _) => "win32",
        ("linux", "x86_64") => "linux64",
        _ => bail!("此平台请手动选择 Chromium 路径"),
    };
    let candidates = info["milestones"]["136"]["downloads"]["chrome"]
        .as_array()
        .context("未找到兼容浏览器版本")?;
    let url = candidates
        .iter()
        .find(|v| v["platform"] == platform_name)
        .and_then(|v| v["url"].as_str())
        .context("没有此平台的浏览器下载")?;
    let dest = s.store.root.join("downloads/browser.zip");
    platform::download(url, &dest, |r, t, n| {
        s.emit("download", json!(["browser", r, t, n]))
    })
    .await?;
    let target = s.store.root.join("browser");
    if target.exists() {
        bail!("浏览器目录已存在，请在设置中选择现有浏览器")
    }
    let copy = target.clone();
    tauri::async_runtime::spawn_blocking(move || platform::unzip(&dest, &copy)).await??;
    let list = platform::valid_browsers(&s.store.root, &s.store.resources);
    let p = list
        .iter()
        .find(|v| {
            v["path"]
                .as_str()
                .is_some_and(|p| Path::new(p).starts_with(&target))
        })
        .and_then(|v| v["path"].as_str())
        .context("解压后未找到浏览器")?;
    Ok(json!(p))
}
async fn launch_only(s: &Arc<AppState>, a: &[Value]) -> Result<Value> {
    use std::process::Stdio;
    let executable = Path::new(text(a, 0)?);
    if !executable.is_file() {
        bail!("浏览器路径不存在")
    }
    let profile = s.store.check(Path::new(text(a, 1)?), false)?;
    let snap = s.store.snapshot();
    let token = snap["server"]["authToken"]
        .as_str()
        .context("服务尚未启动")?;
    let url = format!(
        "http://localhost:{}/index.html?token={}#/bookmarks?uid={}",
        snap["server"]["port"],
        token,
        text(a, 2)?
    );
    let mut cmd = tokio::process::Command::new(executable);
    if snap["render"]["setting"]["launchOptions"]["restoreSession"]
        .as_bool()
        .unwrap_or(true)
    {
        cmd.arg("--restore-last-session");
    }
    cmd.args(["--no-first-run", "--no-default-browser-check"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(folder) = snap["paths"]["extensionsFolder"].as_str() {
        let extensions: Vec<_> = fs::read_dir(folder)?
            .filter_map(|f| f.ok())
            .map(|f| f.path())
            .filter(|p| p.join("manifest.json").is_file())
            .collect();
        if !extensions.is_empty() {
            cmd.arg(format!(
                "--load-extension={}",
                extensions
                    .iter()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    }
    // A user-requested standalone browser is deliberately independent of automation workers.
    let mut child = cmd.spawn()?;
    tauri::async_runtime::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(Value::Null)
}

#[tauri::command]
pub async fn worker_start(
    state: State<'_, Arc<AppState>>,
    uid: String,
) -> std::result::Result<(), String> {
    mapped(state.workers.start(&uid).await)
}
#[tauri::command]
pub async fn worker_call(
    state: State<'_, Arc<AppState>>,
    uid: String,
    event: String,
    args: Value,
) -> std::result::Result<Value, String> {
    mapped(
        async {
            if event == "init" {
                if let Some(p) = args[0]["cachePath"].as_str() {
                    state.store.check(Path::new(p), false)?;
                }
            }
            if event == "launch" {
                let p = args[0]["userDataDir"].as_str().context("用户目录缺失")?;
                state.store.check(Path::new(p), false)?;
            }
            state.workers.call(&uid, &event, args).await
        }
        .await,
    )
}
#[tauri::command]
pub async fn worker_stop(
    state: State<'_, Arc<AppState>>,
    uid: String,
) -> std::result::Result<(), String> {
    let _ = state.workers.call(&uid, "close", json!([])).await;
    mapped(state.workers.stop(&uid).await)
}

#[tauri::command]
pub async fn bridge_control(
    state: State<'_, Arc<AppState>>,
    action: String,
    config: Option<Value>,
) -> std::result::Result<Value, String> {
    mapped(bridge_action(&state, &action, config).await)
}
async fn submitted_bridge_config(s: &Arc<AppState>, value: Value) -> Result<Config> {
    let mut c: Config = serde_json::from_value(value.clone())?;
    let previous = s.bridge_config.lock().await;
    if value["clear_api_key"] == true {
        c.api_key.clear();
    } else if c.api_key.trim().is_empty() {
        if !previous.api_key.is_empty() && c.api_base_url.trim() != previous.api_base_url.trim() {
            bail!("修改接口地址时，请重新输入该接口的 API Key")
        }
        c.api_key = previous.api_key.clone();
    }
    Ok(c)
}

async fn bridge_action(s: &Arc<AppState>, action: &str, config: Option<Value>) -> Result<Value> {
    match action {
        "get" => {}
        "select" => {
            crate::ai_service::select(
                s,
                config.context("缺少配置")?["selected"]
                    .as_bool()
                    .context("缺少选择状态")?,
            )?;
        }
        "sources" => {
            crate::ai_service::save_sources(s, &config.context("缺少题库配置")?)?;
        }
        "detect_paths" => {
            let c = config.context("缺少配置")?;
            return Ok(crate::codex_paths::detect(
                c["codex_bin"].as_str().unwrap_or_default(),
                c["codex_home"].as_str().unwrap_or_default(),
            ));
        }
        "models" => {
            let value = config.context("缺少配置")?;
            if value["provider"].as_str().unwrap_or("codex") == "codex" {
                let home = value["codex_home"]
                    .as_str()
                    .context("缺少 Codex 登录目录")?;
                let path = Path::new(home).join("models_cache.json");
                let cache: Value = serde_json::from_slice(&fs::read(path).context(
                    "没有找到本机登录态模型缓存；请在 Codex 刷新模型后重试，或手动填写模型 ID",
                )?)
                .context("本机模型缓存格式错误；可手动填写模型 ID")?;
                let mut models: Vec<_> = cache["models"]
                    .as_array()
                    .context("本机模型缓存没有模型列表")?
                    .iter()
                    .filter_map(|model| model["slug"].as_str())
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect();
                models.sort();
                models.dedup();
                if models.is_empty() {
                    bail!("本机模型缓存为空；可手动填写模型 ID")
                }
                return Ok(json!({"models":models,"source":"local-cache"}));
            }
            let c = submitted_bridge_config(s, value).await?;
            return Ok(json!({"models":ocs_core::api_runner::ApiRunner::models(&c).await?}));
        }
        "save" | "import" => {
            let mut c: Config = if action == "import" {
                let paths=dialog(s,false,&json!({"title":"选择旧版题库 config.json","filters":[{"name":"JSON","extensions":["json"]}]})).await?;
                let p = paths.first().context("已取消导入")?;
                let mut c = Config::load(Path::new(p), Some(&s.store))?;
                c.port = 18766;
                c.token = uuid::Uuid::new_v4().simple().to_string();
                c
            } else {
                submitted_bridge_config(s, config.context("缺少配置")?).await?
            };
            c.api_key = c.api_key.trim().to_string();
            c.api_model = c.api_model.trim().to_string();
            c.api_base_url = c.api_base_url.trim().to_string();
            c.validate()?;
            if c.port == 0 {
                c.port = 18766;
            }
            crate::ai_service::save(s, c).await?;
        }
        "start" => {
            crate::ai_service::start(s).await?;
        }
        "stop" => {
            let mut r = s.bridge.lock().await;
            if let Some(b) = r.as_ref() {
                if b.bridge.status().await["inflight"].as_u64().unwrap_or(0) > 0 {
                    bail!("还有题目正在处理，请完成后停止服务")
                }
            }
            if let Some(b) = r.take() {
                b.task.abort();
            }
        }
        _ => bail!("未知的题库操作"),
    }
    let c = s.bridge_config.lock().await.clone();
    let r = s.bridge.lock().await;
    let status = if let Some(b) = r.as_ref() {
        b.bridge.status().await
    } else {
        json!({"status":"stopped", "last_error":s.bridge_error.lock().await.clone()})
    };
    let url = format!("http://127.0.0.1:{}/t/{}/config.json", c.port, c.token);
    let wrappers = Bridge::new(c.clone())?.wrappers();
    Ok(
        json!({"config":c,"api_key_configured":!c.api_key.is_empty(),"status":status,"subscription":url,"wrappers":wrappers,
            "ocs":s.store.snapshot()["render"]["setting"]["ocs"]}),
    )
}

/// The embedded course settings page may edit AI options, but cannot replace
/// local executables, read arbitrary login directories, or rotate capabilities.
pub(crate) async fn browser_bridge_action(
    s: &Arc<AppState>,
    action: &str,
    config: Option<Value>,
) -> Result<Value> {
    if !["get", "save", "models", "detect_paths", "select", "sources"].contains(&action) {
        bail!("不支持的 AI 设置操作");
    }
    let config = if ["save", "models", "detect_paths"].contains(&action) {
        let mut merged = serde_json::to_value(s.bridge_config.lock().await.clone())?;
        if let Some(input) = config {
            for key in [
                "provider",
                "model",
                "reasoning_effort",
                "api_base_url",
                "api_protocol",
                "api_model",
                "api_reasoning_effort",
                "api_response_format",
                "api_temperature",
                "api_max_output_tokens",
                "api_token_parameter",
                "api_key",
                "clear_api_key",
                "timeout_seconds",
                "max_concurrency",
            ] {
                if let Some(value) = input.get(key) {
                    merged[key] = value.clone();
                }
            }
        }
        if merged["provider"] == "codex" {
            let paths = crate::codex_paths::detect(
                merged["codex_bin"].as_str().unwrap_or_default(),
                merged["codex_home"].as_str().unwrap_or_default(),
            );
            for key in ["codex_bin", "codex_home"] {
                if paths[key].is_string() {
                    merged[key] = paths[key].clone();
                }
            }
        }
        Some(merged)
    } else {
        config
    };
    let mut result = bridge_action(s, action, config).await?;
    if let Some(config) = result.get_mut("config").and_then(Value::as_object_mut) {
        config.remove("token");
    }
    Ok(result)
}
