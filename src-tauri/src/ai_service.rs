use crate::{AppState, RunningBridge};
use anyhow::{bail, Context, Result};
use axum::{
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
};
use ocs_core::bridge::{Bridge, Config};
use ocs_core::storage::Storage;
use serde_json::{json, Value};
use std::sync::Arc;

fn managed(wrapper: &Value) -> bool {
    wrapper["ocsManagedAI"] == true
        || wrapper["name"].as_str().is_some_and(|name| {
            name.starts_with("Codex 整页题库") || name.starts_with("AI API 整页题库")
        })
}

pub fn notify_settings(s: &AppState) {
    let settings = s.store.snapshot()["render"]["setting"]["ocs"].clone();
    s.emit("ai-settings-changed", json!([settings]));
}

pub fn select(s: &AppState, selected: bool) -> Result<()> {
    let mut snapshot = s.store.snapshot();
    let store = &mut snapshot["render"]["setting"]["ocs"]["store"];
    let names: Vec<Value> = store["common.settings.answererWrappers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| managed(w))
        .map(|w| w["name"].clone())
        .collect();
    let mut disabled: Vec<Value> = store["common.settings.disabledAnswererWrapperNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|name| !names.contains(name))
        .cloned()
        .collect();
    if !selected {
        disabled.extend(names);
    }
    store["common.settings.disabledAnswererWrapperNames"] = json!(disabled);
    s.store.save(snapshot)?;
    notify_settings(s);
    Ok(())
}

pub fn save_sources(s: &AppState, data: &Value) -> Result<()> {
    let incoming = data["wrappers"].as_array().context("题库配置必须为数组")?;
    if incoming.len() > 10 {
        bail!("最多配置 10 个其他题库");
    }
    for wrapper in incoming {
        if managed(wrapper) {
            bail!("AI 题库请在 AI 设置中修改");
        }
        if wrapper["name"]
            .as_str()
            .is_none_or(|name| name.trim().is_empty())
        {
            bail!("题库名称不能为空");
        }
        ocs_core::platform::web_url(wrapper["url"].as_str().context("题库地址不能为空")?)?;
    }
    let mut snapshot = s.store.snapshot();
    let store = &mut snapshot["render"]["setting"]["ocs"]["store"];
    let ai: Vec<Value> = store["common.settings.answererWrappers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| managed(w))
        .cloned()
        .collect();
    let ai_names: Vec<Value> = ai.iter().map(|w| w["name"].clone()).collect();
    let mut disabled: Vec<Value> = store["common.settings.disabledAnswererWrapperNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|name| ai_names.contains(name))
        .cloned()
        .collect();
    disabled.extend(
        data["disabled"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|name| incoming.iter().any(|w| &w["name"] == *name))
            .cloned(),
    );
    store["common.settings.answererWrappers"] = json!(ai
        .into_iter()
        .chain(incoming.iter().cloned())
        .collect::<Vec<_>>());
    store["common.settings.disabledAnswererWrapperNames"] = json!(disabled);
    s.store.save(snapshot)?;
    notify_settings(s);
    Ok(())
}

async fn require_selection(
    State((store, name)): State<(Arc<Storage>, String)>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == axum::http::Method::POST && request.uri().path().ends_with("/answer") {
        let snapshot = store.snapshot();
        let settings = &snapshot["render"]["setting"]["ocs"]["store"];
        let present = settings["common.settings.answererWrappers"]
            .as_array()
            .is_some_and(|wrappers| wrappers.iter().any(|wrapper| wrapper["name"] == name));
        let disabled = settings["common.settings.disabledAnswererWrapperNames"]
            .as_array()
            .is_some_and(|names| names.iter().any(|value| value == &name));
        if !present || disabled {
            return ocs_core::bridge::reply(
                409,
                json!({"code":0,"msg":"尚未选用 AI 题库，请在答题来源中选择 AI 后再搜题"}),
            );
        }
    }
    next.run(request).await
}

fn register(s: &AppState, config: &Config) -> Result<()> {
    let mut snapshot = s.store.snapshot();
    let settings = &mut snapshot["render"]["setting"]["ocs"];
    let store = &mut settings["store"];
    let old = store["common.settings.answererWrappers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let disabled = store["common.settings.disabledAnswererWrapperNames"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // Keep an existing explicit choice. A newly available AI bank starts unselected.
    let selected = old
        .iter()
        .any(|wrapper| managed(wrapper) && !disabled.contains(&wrapper["name"]));
    let old_names: Vec<_> = old
        .iter()
        .filter(|wrapper| managed(wrapper))
        .map(|wrapper| wrapper["name"].clone())
        .collect();
    let mut wrappers: Vec<_> = old
        .into_iter()
        .filter(|wrapper| !managed(wrapper))
        .collect();
    let mut disabled: Vec<_> = disabled
        .into_iter()
        .filter(|name| !old_names.contains(name))
        .collect();
    let ai = Bridge::new(config.clone())?.wrappers();
    for wrapper in ai.as_array().context("AI 题库配置格式错误")? {
        if !selected {
            disabled.push(wrapper["name"].clone());
        }
        wrappers.push(wrapper.clone());
    }
    store["common.settings.answererWrappers"] = json!(wrappers);
    store["common.settings.disabledAnswererWrapperNames"] = json!(disabled);
    settings["openSync"] = json!(true);
    s.store.save(snapshot)?;
    notify_settings(s);
    Ok(())
}

pub async fn start(s: &Arc<AppState>) -> Result<()> {
    let result = async {
        let mut running = s.bridge.lock().await;
        if running.is_some() {
            return Ok(());
        }
        let config = s.bridge_config.lock().await.clone();
        // Binding the local service and constructing a runner never queries a model.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
            .await
            .context("AI 题库端口被占用，请在高级设置中修改端口")?;
        let bridge = Bridge::new(config)?;
        let name = bridge.wrappers()[0]["name"]
            .as_str()
            .context("AI 题库名称缺失")?
            .to_owned();
        let router = ocs_core::bridge::router(bridge.clone()).layer(
            middleware::from_fn_with_state((s.store.clone(), name), require_selection),
        );
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        *running = Some(RunningBridge { bridge, task });
        Ok(())
    }
    .await;
    *s.bridge_error.lock().await = result
        .as_ref()
        .err()
        .map(|e: &anyhow::Error| format!("{e:#}"));
    result
}

pub async fn initialize(s: &Arc<AppState>) -> Result<()> {
    let mut config = s.bridge_config.lock().await.clone();
    if config.provider == "codex" {
        let paths = crate::codex_paths::detect(&config.codex_bin, &config.codex_home);
        if let Some(path) = paths["codex_bin"].as_str() {
            config.codex_bin = path.into();
        }
        if let Some(path) = paths["codex_home"].as_str() {
            config.codex_home = path.into();
        }
    }
    if config.port == 0 {
        config.port = 18766;
    }
    config.save(&s.store)?;
    register(s, &config)?;
    *s.bridge_config.lock().await = config;
    start(s).await
}

pub async fn save(s: &Arc<AppState>, config: Config) -> Result<()> {
    let mut running = s.bridge.lock().await;
    if let Some(current) = running.as_ref() {
        if current.bridge.status().await["inflight"]
            .as_u64()
            .unwrap_or(0)
            > 0
        {
            bail!("AI 正在回答题目，请完成后再保存设置")
        }
    }
    config.save(&s.store)?;
    register(s, &config)?;
    *s.bridge_config.lock().await = config;
    if let Some(current) = running.take() {
        current.task.abort();
        let _ = current.task.await;
    }
    drop(running);
    start(s).await
}
