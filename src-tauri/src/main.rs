#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ai_service;
mod codex_paths;
mod commands;
mod server;
use anyhow::{Context, Result};
use ocs_core::{
    bridge::{Bridge, Config},
    storage::Storage,
    worker::WorkerPool,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

pub struct Initialization(tokio::sync::watch::Sender<Option<std::result::Result<(), String>>>);

struct RunningBridge {
    bridge: Arc<Bridge>,
    task: tokio::task::JoinHandle<()>,
}
pub struct AppState {
    store: Arc<Storage>,
    workers: WorkerPool,
    bridge: Mutex<Option<RunningBridge>>,
    bridge_config: Mutex<Config>,
    bridge_error: Mutex<Option<String>>,
    app: tauri::AppHandle,
    exit_ready: AtomicBool,
    restarting: AtomicBool,
}
impl AppState {
    fn emit(&self, event: &str, args: Value) {
        let _ = self
            .app
            .emit("native-event", json!({"event":event,"args":args}));
    }
    async fn stop(&self) -> Result<()> {
        self.workers.shutdown().await?;
        if let Some(b) = self.bridge.lock().await.take() {
            b.task.abort();
        }
        Ok(())
    }
}
fn resource_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
    let bundled = app.path().resource_dir()?;
    if bundled.join("adapter/worker.cjs").exists() {
        return Ok(bundled);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    Ok(root)
}
fn setup(handle: tauri::AppHandle) -> Result<()> {
    let resources = resource_dir(&handle)?;
    let root = std::env::var_os("OCS_RUST_DATA")
        .map(PathBuf::from)
        .unwrap_or(handle.path().app_data_dir()?);
    let storage = Arc::new(Storage::open(root.clone(), resources.clone())?);
    let adapter = if resources.join("adapter").exists() {
        resources.join("adapter")
    } else {
        resources.join("native-adapter/dist")
    };
    let node = adapter.join(if cfg!(windows) { "node.exe" } else { "node" });
    if !node.is_file() {
        anyhow::bail!("缺少浏览器适配器，请先运行 pnpm build:adapter")
    }
    let sink_handle = handle.clone();
    let log_root = root.join("logs/browsers");
    std::fs::create_dir_all(&log_root)?;
    let workers = WorkerPool::new(
        node,
        adapter.join("worker.cjs"),
        Arc::new(move |v| {
            if v["event"] == "open-external" {
                if let Some(url) = v["args"][0].as_str() {
                    if ocs_core::platform::web_url(url).is_ok() {
                        let _ = sink_handle.opener().open_url(url, None::<&str>);
                    }
                }
            }
            if v["event"] == "log" || v["event"] == "worker-error" {
                use std::io::Write;
                if let Some(uid) = v["uid"].as_str().filter(|id| {
                    id.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                }) {
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log_root.join(format!("{uid}.log")))
                    {
                        let text = v["args"][0].as_str().unwrap_or("");
                        let _ = writeln!(
                            file,
                            "{} {}",
                            chrono::Local::now().to_rfc3339(),
                            text.chars().take(8192).collect::<String>()
                        );
                    }
                }
            }
            let _ = sink_handle.emit("worker-event", v);
        }),
    );
    let listener = tauri::async_runtime::block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut v = storage.value.lock().unwrap();
        v["server"] = json!({"port":port,"authToken":token});
        // A fresh Rust profile does not reuse Chromium login data from the legacy installation.
        if v["render"].as_object().is_some_and(|v| v.is_empty()) {
            let browsers = ocs_core::platform::valid_browsers(&root, &resources);
            if let Some(path) = browsers.first().and_then(|v| v["path"].as_str()) {
                v["render"] =
                    json!({"setting":{"launchOptions":{"executablePath":path,"custom":true}}});
            }
        }
    }
    let config_path = root.join("bridge.config.json");
    let bridge_config = if config_path.exists() {
        Config::load(&config_path, Some(&storage))?
    } else {
        Config::default()
    };
    let state = Arc::new(AppState {
        store: storage,
        workers,
        bridge: Mutex::new(None),
        bridge_config: Mutex::new(bridge_config),
        bridge_error: Mutex::new(None),
        app: handle.clone(),
        exit_ready: AtomicBool::new(false),
        restarting: AtomicBool::new(false),
    });
    handle.manage(state.clone());
    if let Err(error) = tauri::async_runtime::block_on(ai_service::initialize(&state)) {
        tauri::async_runtime::block_on(async {
            *state.bridge_error.lock().await = Some(format!("{error:#}"));
        });
    }
    let event_handle = handle.clone();
    let ai_state = state.clone();
    let local = Arc::new(ocs_core::local_server::LocalServer {
        store: state.store.clone(),
        ai_control: Arc::new(move |action, config| {
            let state = ai_state.clone();
            Box::pin(async move { commands::browser_bridge_action(&state, &action, config).await })
        }),
        events: Arc::new(move |value| {
            if value["event"] == "show-browser-in-app" {
                if let Some(window) = event_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            let _ = event_handle.emit("native-event", value);
        }),
    });
    tauri::async_runtime::spawn(server::serve(listener, local));
    let window = handle.get_webview_window("main").context("主窗口不存在")?;
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            if !state.exit_ready.load(Ordering::SeqCst) {
                api.prevent_close();
                state.emit("close", json!([]));
            }
        }
    });
    Ok(())
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            let (ready, _) = tokio::sync::watch::channel(None);
            app.manage(Initialization(ready.clone()));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let result = setup(handle).map_err(|e| format!("{e:#}"));
                ready.send_replace(Some(result));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::native_call,
            commands::worker_start,
            commands::worker_call,
            commands::worker_stop,
            commands::bridge_control
        ])
        .build(tauri::generate_context!())
        .expect("Unable to build OCS Rust")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if let Some(state) = app.try_state::<Arc<AppState>>() {
                    if !state.exit_ready.load(Ordering::SeqCst) {
                        api.prevent_exit();
                        state.emit("close", json!([]));
                    }
                }
            }
        });
}
