use crate::{
    bridge::{allowed_host, reply, response},
    platform,
};
use crate::{storage::Storage, worker::EventSink};
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::Method,
    response::Response,
    Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    path::{Component, Path},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
pub struct LocalServer {
    pub store: Arc<Storage>,
    pub events: EventSink,
    pub ai_control: Arc<
        dyn Fn(String, Option<Value>) -> Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>
            + Send
            + Sync,
    >,
}
impl LocalServer {
    fn emit(&self, event: &str, args: Value) {
        (self.events)(json!({"event": event, "args": args}));
    }
}

pub async fn serve(listener: tokio::net::TcpListener, state: Arc<LocalServer>) {
    if let Err(e) = axum::serve(
        listener,
        Router::new().fallback(handle).with_state(state.clone()),
    )
    .await
    {
        state.emit("server-error", json!([e.to_string()]));
    }
}
fn find_browser(root: &Value, uid: &str) -> Option<Value> {
    if root["type"] == "browser" && root["uid"] == uid {
        return Some(
            json!({"uid":root["uid"],"name":root["name"],"notes":root["notes"],"tags":root["tags"]}),
        );
    }
    root["children"]
        .as_object()?
        .values()
        .find_map(|v| find_browser(v, uid))
}
async fn handle(State(s): State<Arc<LocalServer>>, req: Request) -> Response {
    let snap = s.store.snapshot();
    let port = snap["server"]["port"].as_u64().unwrap_or(0) as u16;
    if !allowed_host(&req, port) {
        return reply(403, json!({"error":"Invalid local Host"}));
    }
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let params: HashMap<_, _> = reqwest::Url::parse(&format!("http://localhost{}", req.uri()))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    if method == Method::OPTIONS {
        return response(204, vec![], "application/json");
    }
    if method == Method::GET
        && (path == "/"
            || path == "/index.html"
            || path.starts_with("/assets/")
            || path.starts_with("/site-icons/")
            || path.starts_with("/js/")
            || path.starts_with("/hljs-styles/")
            || path == "/favicon.ico"
            || path == "/favicon.png")
    {
        let relative = path.trim_start_matches('/');
        let relative = if relative.is_empty() {
            "index.html"
        } else {
            relative
        };
        if Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return reply(403, json!({"error":"Invalid asset path"}));
        }
        let web = if s.store.resources.join("web/index.html").exists() {
            s.store.resources.join("web")
        } else {
            s.store.resources.join("packages/web/dist")
        };
        let ty = if relative.ends_with(".html") {
            "text/html; charset=utf-8"
        } else if relative.ends_with(".js") {
            "text/javascript"
        } else if relative.ends_with(".css") {
            "text/css"
        } else if relative.ends_with(".svg") {
            "image/svg+xml"
        } else if relative.ends_with(".png") {
            "image/png"
        } else if relative.ends_with(".ico") {
            "image/x-icon"
        } else if relative.ends_with(".woff2") {
            "font/woff2"
        } else {
            "application/octet-stream"
        };
        return match tokio::fs::read(web.join(relative)).await {
            Ok(bytes) => response(200, bytes, ty),
            Err(_) => reply(404, json!({"error":"Asset not found"})),
        };
    }
    let token = snap["server"]["authToken"].as_str().unwrap_or("");
    let provided = req
        .headers()
        .get("auth-token")
        .and_then(|h| h.to_str().ok())
        .or_else(|| params.get("token").map(String::as_str))
        .unwrap_or("");
    use subtle::ConstantTimeEq;
    if token.is_empty() || !bool::from(token.as_bytes().ct_eq(provided.as_bytes())) {
        return reply(403, json!({"error":"Local capability required"}));
    }
    if method == Method::GET {
        return match path.as_str() {
            "/state" => reply(
                200,
                json!({"implementation":"rust","project":s.store.resources}),
            ),
            "/ocs-global-setting" => reply(200, snap["render"]["setting"]["ocs"].clone()),
            "/ai-question-bank" => {
                let settings = &snap["render"]["setting"]["ocs"]["store"];
                let wrappers: Vec<_> = settings["common.settings.answererWrappers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|wrapper| wrapper["ocsManagedAI"] == true)
                    .cloned()
                    .collect();
                reply(
                    200,
                    json!({"wrappers":wrappers,"disabled":settings["common.settings.disabledAnswererWrapperNames"]}),
                )
            }
            "/browser" => reply(
                200,
                if snap["render"]["setting"]["ocs"]["openSync"] == true {
                    snap["render"]["setting"]["ocs"]["store"].clone()
                } else {
                    json!({})
                },
            ),
            "/is-browser-config-sync" => response(
                200,
                if snap["render"]["setting"]["ocs"]["openSync"] == true {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                },
                "text/plain",
            ),
            "/ocs-script-actions" => reply(200, json!({"allow":true})),
            "/get-actions-key" => response(200, token.as_bytes().to_vec(), "text/plain"),
            "/api/bookmark/browser-info" => reply(
                200,
                params
                    .get("uid")
                    .and_then(|id| find_browser(&snap["render"]["browser"]["root"], id))
                    .unwrap_or(Value::Null),
            ),
            "/api/bookmark/show-browser-in-app" => {
                if let Some(uid) = params.get("uid") {
                    s.emit("show-browser-in-app", json!([uid]));
                }
                reply(200, json!({"ok":true}))
            }
            "/api/local-userscript" | "/api/local-userscript.user.js" => {
                let requested = params.get("path").map(String::as_str).unwrap_or("");
                let allowed = snap["render"]["scripts"].as_array().is_some_and(|a| {
                    a.iter().any(|v| {
                        v["isLocalScript"] == true
                            && (v["url"] == requested || v["info"]["code_url"] == requested)
                    })
                });
                if !allowed {
                    return reply(403, json!({"error":"Script is not registered"}));
                }
                match tokio::fs::read(requested).await {
                    Ok(v) => response(200, v, "application/javascript"),
                    Err(_) => reply(404, json!({"error":"Script not found"})),
                }
            }
            "/icon" => icon(params.get("url").map(String::as_str).unwrap_or("")).await,
            _ if path.starts_with("/ocs-action_") => {
                response(200, b"OCS action".to_vec(), "text/plain")
            }
            _ => reply(404, json!({"error":"Not found"})),
        };
    }
    if method != Method::POST {
        return reply(405, json!({"error":"Method not allowed"}));
    }
    let body = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(req.into_body(), 10 * 1024 * 1024),
    )
    .await
    {
        Ok(Ok(v)) => v,
        _ => return reply(413, json!({"error":"Invalid or oversized body"})),
    };
    let data: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return reply(400, json!({"error":"Invalid JSON"})),
    };
    match path.as_str() {
        "/ai-control" => {
            let action = data["action"].as_str().unwrap_or("").to_owned();
            match (s.ai_control)(action, data.get("config").cloned()).await {
                Ok(value) => reply(200, value),
                Err(error) => reply(400, json!({"error":error.to_string()})),
            }
        }
        "/ai-selection" => {
            let Some(selected) = data["selected"].as_bool() else {
                return reply(400, json!({"error":"selected must be a boolean"}));
            };
            let mut snapshot = s.store.snapshot();
            let settings = &mut snapshot["render"]["setting"]["ocs"]["store"];
            let names: Vec<_> = settings["common.settings.answererWrappers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|wrapper| wrapper["ocsManagedAI"] == true)
                .map(|wrapper| wrapper["name"].clone())
                .collect();
            let mut disabled: Vec<_> = settings["common.settings.disabledAnswererWrapperNames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|name| !names.contains(name))
                .cloned()
                .collect();
            if !selected {
                disabled.extend(names);
            }
            settings["common.settings.disabledAnswererWrapperNames"] = json!(disabled);
            if let Err(error) = s.store.save(snapshot) {
                return reply(500, json!({"error":error.to_string()}));
            }
            s.emit("ai-selection-changed", json!([disabled]));
            reply(200, json!({"disabled":disabled}))
        }
        // No OCR model is installed or started. Page scripts retain their manual verification flow.
        "/ocr" => reply(200, json!({"canOCR":false})),
        "/proxy" => {
            let result = async {
                let url = platform::web_url(
                    data["url"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("URL missing"))?,
                )?;
                let method = reqwest::Method::from_bytes(
                    data["method"].as_str().unwrap_or("GET").as_bytes(),
                )?;
                if ![
                    reqwest::Method::GET,
                    reqwest::Method::POST,
                    reqwest::Method::PUT,
                    reqwest::Method::DELETE,
                    reqwest::Method::PATCH,
                    reqwest::Method::HEAD,
                ]
                .contains(&method)
                {
                    anyhow::bail!("Unsupported method")
                }
                let mut r = platform::client().request(method, url);
                if !data["data"].is_null() {
                    r = r.json(&data["data"]);
                }
                if let Some(h) = data["headers"].as_object() {
                    for (k, v) in h {
                        if !["host", "auth-token", "connection", "content-length"]
                            .contains(&k.to_lowercase().as_str())
                        {
                            if let Some(v) = v.as_str() {
                                r = r.header(k, v);
                            }
                        }
                    }
                }
                let res = r.send().await?;
                let status = res.status().as_u16();
                let ty = res
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("application/json")
                    .to_string();
                let bytes = res.bytes().await?;
                Ok::<_, anyhow::Error>(response(status, bytes.to_vec(), &ty))
            }
            .await;
            result.unwrap_or_else(|e| reply(502, json!({"error":e.to_string()})))
        }
        _ => reply(404, json!({"error":"Not found"})),
    }
}
async fn icon(url: &str) -> Response {
    if let Ok(parsed) = platform::web_url(url) {
        let fallback = parsed.join("/favicon.ico").ok();
        for candidate in std::iter::once(parsed).chain(fallback) {
            if let Ok(r) = platform::client()
                .get(candidate)
                .timeout(Duration::from_secs(3))
                .send()
                .await
            {
                let ty = r
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                if r.status().is_success() && ty.starts_with("image/") {
                    if let Ok(bytes) = r.bytes().await {
                        if bytes.len() < 2 * 1024 * 1024 {
                            return response(200, bytes.to_vec(), &ty);
                        }
                    }
                }
            }
        }
    }
    response(200,br##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><circle cx="24" cy="24" r="20" fill="#e8f1ff" stroke="#4678b4"/><path d="M4 24h40M24 4v40" stroke="#4678b4"/></svg>"##.to_vec(),"image/svg+xml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    #[tokio::test]
    async fn userscript_install_url_requires_capability_and_registration() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fixture.user.js");
        let code = "// ==UserScript==\n// @name Fixture\n// ==/UserScript==";
        std::fs::write(&script, code).unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            serde_json::to_vec(&json!({
                "render":{"scripts":[{"isLocalScript":true,"url":script}]},
                "server":{"port":12345,"authToken":"test-token"}
            })).unwrap(),
        ).unwrap();
        let state = Arc::new(LocalServer {
            store: Arc::new(Storage::with_key(dir.path().into(), dir.path().into(), [3;32]).unwrap()),
            events: Arc::new(|_| {}),
            ai_control: Arc::new(|_, _| Box::pin(async { Ok(Value::Null) })),
        });
        for (endpoint, token, file, expected) in [
            ("local-userscript.user.js", "test-token", script.clone(), 200),
            ("local-userscript", "test-token", script.clone(), 200),
            ("local-userscript.user.js", "wrong-token", script, 403),
            ("local-userscript.user.js", "test-token", dir.path().join("unregistered.user.js"), 403),
        ] {
            let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:12345/api/{endpoint}")).unwrap();
            url.query_pairs_mut().append_pair("path", &file.to_string_lossy()).append_pair("token", token);
            let request = Request::builder().uri(url.as_str()).header("Host", "127.0.0.1:12345").body(Body::empty()).unwrap();
            let response = handle(State(state.clone()), request).await;
            assert_eq!(response.status(), expected);
            if expected == 200 {
                assert_eq!(response.headers()["Content-Type"], "application/javascript");
                assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), code.as_bytes());
            }
        }
    }
}
