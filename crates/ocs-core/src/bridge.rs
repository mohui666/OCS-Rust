use crate::{
    answer::{format_answer, normalized_title, Question},
    api_runner::ApiRunner,
    runner::{CodexRunner, Runner},
    storage::{atomic_write, Storage},
};
use anyhow::{bail, Context, Result};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{Method, StatusCode},
    response::Response,
    Router,
};
use futures_util::{FutureExt, StreamExt};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{watch, Mutex, Semaphore};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    pub token: String,
    pub model: String,
    pub reasoning_effort: String,
    pub codex_bin: String,
    pub codex_home: String,
    pub timeout_seconds: Option<f64>,
    pub max_concurrency: usize,
    pub provider: String,
    pub api_base_url: String,
    pub api_protocol: String,
    pub api_model: String,
    pub api_reasoning_effort: String,
    pub api_response_format: String,
    pub api_temperature: Option<f64>,
    pub api_max_output_tokens: Option<u32>,
    pub api_token_parameter: String,
    #[serde(skip_serializing)]
    pub api_key: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            port: 18766,
            token: uuid::Uuid::new_v4().simple().to_string(),
            model: "".into(),
            reasoning_effort: "low".into(),
            codex_bin: "codex".into(),
            codex_home: dirs::home_dir()
                .unwrap_or_default()
                .join(".codex")
                .to_string_lossy()
                .into(),
            timeout_seconds: Some(105.0),
            max_concurrency: 2,
            provider: "codex".into(),
            api_base_url: "https://api.openai.com/v1".into(),
            api_protocol: "chat_completions".into(),
            api_model: String::new(),
            api_reasoning_effort: String::new(),
            api_response_format: "json_object".into(),
            api_temperature: None,
            api_max_output_tokens: None,
            api_token_parameter: "max_completion_tokens".into(),
            api_key: String::new(),
        }
    }
}
impl Config {
    pub fn load(path: &Path, store: Option<&Storage>) -> Result<Self> {
        let value: Value =
            serde_json::from_slice(&std::fs::read(path)?).context("题库配置文件格式错误")?;
        let mut config: Self = serde_json::from_value(value.clone())?;
        if let Some(encrypted) = value["api_key_encrypted"].as_str() {
            config.api_key = match store {
                Some(store) => store.decrypt(encrypted)?,
                None => crate::storage::legacy_decrypt(encrypted)?,
            };
        }
        Ok(config)
    }
    pub fn save(&self, store: &Storage) -> Result<()> {
        let mut value = serde_json::to_value(self)?;
        if !self.api_key.is_empty() {
            value["api_key_encrypted"] = json!(store.encrypt(&self.api_key)?);
        }
        atomic_write(
            &store.root.join("bridge.config.json"),
            &serde_json::to_vec_pretty(&value)?,
        )
    }
    pub fn selected_model(&self) -> &str {
        if self.provider == "api" {
            &self.api_model
        } else {
            &self.model
        }
    }
    pub fn selected_reasoning(&self) -> &str {
        if self.provider == "api" {
            &self.api_reasoning_effort
        } else {
            &self.reasoning_effort
        }
    }
    pub fn validate_ready(&self) -> Result<()> {
        self.validate()?;
        if self.provider == "api" {
            if self.api_model.trim().is_empty() {
                bail!("请填写 AI API 模型 ID")
            }
            if self.api_key.trim().is_empty() {
                bail!("请填写 AI API Key")
            }
        }
        Ok(())
    }
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout_seconds
            .filter(|s| *s > 0.0)
            .map(Duration::from_secs_f64)
    }
    pub fn validate(&self) -> Result<()> {
        if !["codex", "api"].contains(&self.provider.as_str()) {
            bail!("未知的题库来源")
        }
        if self.provider == "api" {
            ApiRunner::endpoint(self)?;
            if !["prompt", "json_object", "json_schema"]
                .contains(&self.api_response_format.as_str())
            {
                bail!("未知的 API 答案格式")
            }
            if !["", "none", "minimal", "low", "medium", "high", "xhigh"]
                .contains(&self.api_reasoning_effort.as_str())
            {
                bail!("未知的 API 推理强度")
            }
            if self
                .api_temperature
                .is_some_and(|v| !v.is_finite() || !(0.0..=2.0).contains(&v))
            {
                bail!("温度必须在 0 到 2 之间，留空使用服务商默认值")
            }
            if !["max_completion_tokens", "max_tokens"].contains(&self.api_token_parameter.as_str())
            {
                bail!("未知的输出长度参数")
            }
        }
        if self.token.is_empty()
            || !self
                .token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
        {
            bail!("token 只能包含英文字母、数字及 -._~")
        }
        if !(1..=32).contains(&self.max_concurrency) {
            bail!("max_concurrency 必须在 1..32")
        }
        if self
            .timeout_seconds
            .is_some_and(|v| !v.is_finite() || v < 0.0 || v > 31536000.0)
        {
            bail!("timeout_seconds 必须为 null 或非负秒数")
        }
        Ok(())
    }
}
type Answer = std::result::Result<Value, String>;
type Flight = watch::Sender<Option<Answer>>;
pub struct Prepared {
    key: String,
    candidates: Vec<(String, Question)>,
}
pub fn prepare(data: &Value) -> Result<Prepared> {
    let mut q = Question::parse(data)?;
    let empty = vec![];
    let page = match data.get("pageQuestions") {
        None => &empty,
        Some(v) => v
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("整页题目必须是数组，每批最多 200 题"))?,
    };
    if page.len() > 200 {
        bail!("整页题目必须是数组，每批最多 200 题")
    }
    let page: Vec<_> = page.iter().map(Question::parse).collect::<Result<_>>()?;
    if q.kind.is_empty() && q.options.is_empty() {
        let matches: Vec<_> = page
            .iter()
            .filter(|p| normalized_title(&p.title) == normalized_title(&q.title))
            .collect();
        if matches.len() == 1 {
            q = matches[0].clone()
        }
    }
    let key = q.key();
    let mut candidates = Vec::<(String, Question)>::new();
    for q in std::iter::once(q).chain(page) {
        let k = q.key();
        if let Some(p) = candidates.iter_mut().find(|p| p.0 == k) {
            p.1 = q
        } else {
            candidates.push((k, q))
        }
    }
    if candidates.len() > 200 {
        bail!("每批最多 200 题，包含当前题目")
    }
    Ok(Prepared { key, candidates })
}
struct Inner {
    cache: LruCache<String, (Instant, Value)>,
    inflight: HashMap<String, Flight>,
    requests: u64,
    completed: u64,
    codex_calls: u64,
    cache_hits: u64,
    last_batch_size: usize,
    last_error: String,
}
pub struct Bridge {
    pub config: Config,
    inner: Mutex<Inner>,
    slots: Arc<Semaphore>,
    runner: Arc<dyn Runner>,
}
impl Bridge {
    pub fn new(config: Config) -> Result<Arc<Self>> {
        let runner: Arc<dyn Runner> = if config.provider == "api" {
            Arc::new(ApiRunner::new()?)
        } else {
            Arc::new(CodexRunner)
        };
        Self::with_runner(config, runner)
    }
    pub fn with_runner(config: Config, runner: Arc<dyn Runner>) -> Result<Arc<Self>> {
        config.validate()?;
        Ok(Arc::new(Self {
            slots: Arc::new(Semaphore::new(config.max_concurrency)),
            config,
            runner,
            inner: Mutex::new(Inner {
                cache: LruCache::new(NonZeroUsize::new(256).unwrap()),
                inflight: HashMap::new(),
                requests: 0,
                completed: 0,
                codex_calls: 0,
                cache_hits: 0,
                last_batch_size: 0,
                last_error: String::new(),
            }),
        }))
    }
    pub fn wrappers(&self) -> Value {
        let mut v: Value =
            serde_json::from_str(include_str!("../../../assets/bridge/wrapper-template.json"))
                .unwrap();
        v[0]["ocsManagedAI"] = json!(true);
        let base = format!(
            "http://127.0.0.1:{}/t/{}",
            self.config.port, self.config.token
        );
        v[0]["name"] = json!(format!(
            "{} 整页题库（{} {}）",
            if self.config.provider == "api" {
                "AI API"
            } else {
                "Codex"
            },
            self.config.selected_model(),
            self.config.selected_reasoning()
        ));
        v[0]["homepage"] = json!(format!("{base}/"));
        v[0]["url"] = json!(format!("{base}/answer"));
        v
    }
    pub async fn status(&self) -> Value {
        let i = self.inner.lock().await;
        json!({"service":"ocs-codex", "status":"running", "implementation":"rust", "provider":self.config.provider,"model":self.config.selected_model(),"reasoning_effort":self.config.selected_reasoning(),"timeout_seconds":self.config.timeout().map(|v| v.as_secs_f64()),"timeout_unlimited":self.config.timeout().is_none(),"requests":i.requests,"completed":i.completed,"codex_calls":i.codex_calls,"ai_calls":i.codex_calls,"cache_hits":i.cache_hits,"cache_size":i.cache.len(),"inflight":i.inflight.len(),"last_batch_size":i.last_batch_size,"last_error":i.last_error})
    }
    pub async fn answer(self: &Arc<Self>, request: Prepared) -> Answer {
        let started = Instant::now();
        let mut inner = self.inner.lock().await;
        inner.requests += 1;
        let expired: Vec<_> = inner
            .cache
            .iter()
            .filter(|(_, (t, _))| *t <= started)
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired {
            inner.cache.pop(&k);
        }
        if let Some((_, v)) = inner.cache.get(&request.key) {
            let mut v = v.clone();
            v["cached"] = json!(true);
            inner.cache_hits += 1;
            return Ok(v);
        }
        let shared = inner.inflight.contains_key(&request.key);
        let mut receiver;
        if let Some(s) = inner.inflight.get(&request.key) {
            receiver = s.subscribe();
        } else {
            let permit = self
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| "AI 正在处理其他页面，请稍后重试".to_string())?;
            let owned: Vec<_> = request
                .candidates
                .into_iter()
                .filter(|(k, _)| !inner.cache.contains(k) && !inner.inflight.contains_key(k))
                .collect();
            for (k, _) in &owned {
                inner.inflight.insert(k.clone(), watch::channel(None).0);
            }
            receiver = inner.inflight[&request.key].subscribe();
            inner.codex_calls += 1;
            inner.last_batch_size = owned.len();
            let this = self.clone();
            let primary = request.key.clone();
            // Detached from the HTTP connection: a cancelled client must not cancel a shared batch.
            tokio::spawn(async move {
                let result = std::panic::AssertUnwindSafe(this.run_batch(&owned, started))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err("批量搜题失败，请检查本地服务日志后重试".into()));
                let mut inner = this.inner.lock().await;
                match result {
                    Ok(values) => {
                        inner.last_error = values
                            .get(&primary)
                            .filter(|v| v["code"] == 0)
                            .and_then(|v| v["msg"].as_str())
                            .unwrap_or("")
                            .into();
                        for (k, v) in values {
                            let success = v["code"] == 1;
                            inner.completed += u64::from(success);
                            inner.cache.put(
                                k.clone(),
                                (
                                    Instant::now()
                                        + Duration::from_secs(if success { 3600 } else { 60 }),
                                    v.clone(),
                                ),
                            );
                            if let Some(s) = inner.inflight.get(&k) {
                                s.send_replace(Some(Ok(v)));
                            }
                        }
                    }
                    Err(e) => {
                        inner.last_error = e.clone();
                        for (k, _) in &owned {
                            if let Some(s) = inner.inflight.get(k) {
                                s.send_replace(Some(Err(e.clone())));
                            }
                        }
                    }
                }
                for (k, _) in owned {
                    inner.inflight.remove(&k);
                }
                drop(permit);
            });
        }
        drop(inner);
        let wait = async {
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return result;
                }
                if receiver.changed().await.is_err() {
                    return Err("批量搜题进程中断".into());
                }
            }
        };
        let mut value = if shared {
            if let Some(t) = self.config.timeout() {
                tokio::time::timeout(t + Duration::from_secs(5), wait)
                    .await
                    .map_err(|_| "等待同页答案超时，请稍后重试".to_string())??
            } else {
                wait.await?
            }
        } else {
            wait.await?
        };
        if shared {
            value["shared"] = json!(true)
        }
        Ok(value)
    }
    async fn run_batch(
        &self,
        owned: &[(String, Question)],
        started: Instant,
    ) -> std::result::Result<HashMap<String, Value>, String> {
        let questions: Vec<_> = owned.iter().map(|(_, q)| q.clone()).collect();
        let raw = self
            .runner
            .run(&questions, &self.config)
            .await
            .map_err(|e| e.to_string())?;
        let items = raw["results"]
            .as_array()
            .ok_or("AI 批量答案题号缺失、重复或不匹配，已停止分配答案")?;
        let ids: HashSet<_> = items.iter().filter_map(|v| v["id"].as_str()).collect();
        if items.len() != owned.len()
            || ids.len() != items.len()
            || !owned.iter().all(|(k, _)| ids.contains(k.as_str()))
        {
            return Err("AI 批量答案题号缺失、重复或不匹配，已停止分配答案".into());
        }
        let mut out = HashMap::new();
        for raw_item in items {
            let id = raw_item["id"].as_str().unwrap();
            let q = &owned.iter().find(|(k, _)| k == id).unwrap().1;
            let parsed = (|| -> Result<Value> {
                let mut v = format_answer(q, raw_item)?;
                let sources = raw_item.get("sources").cloned().unwrap_or(json!([]));
                let sources = sources
                    .as_array()
                    .filter(|a| a.iter().all(Value::is_string))
                    .ok_or_else(|| anyhow::anyhow!("AI 来源格式无效"))?;
                v["sources"] = json!(sources
                    .iter()
                    .filter(|s| s.as_str().unwrap().starts_with("https://")
                        || s.as_str().unwrap().starts_with("http://"))
                    .collect::<Vec<_>>());
                Ok(v)
            })();
            let mut v=parsed.unwrap_or_else(|e|json!({"code":0,"question":q.title,"answer":"","sources":[],"msg":e.to_string()}));
            v["cached"] = json!(false);
            v["elapsed_seconds"] = json!((started.elapsed().as_secs_f64() * 100.0).round() / 100.0);
            v["batch_size"] = json!(owned.len());
            v["web_search_calls"] = raw.get("web_search_calls").cloned().unwrap_or(json!(0));
            out.insert(id.to_string(), v);
        }
        Ok(out)
    }
}

pub fn router(bridge: Arc<Bridge>) -> Router {
    Router::new().fallback(handler).with_state(bridge)
}
pub fn reply(status: u16, value: Value) -> Response {
    response(
        status,
        serde_json::to_vec(&value).unwrap(),
        "application/json; charset=utf-8",
    )
}
pub fn response(status: u16, bytes: Vec<u8>, content_type: &str) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type")
        .header("Access-Control-Allow-Private-Network", "true")
        .body(Body::from(bytes))
        .unwrap()
}
pub fn allowed_host(req: &Request, port: u16) -> bool {
    req.headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"))
}
async fn handler(State(b): State<Arc<Bridge>>, req: Request) -> Response {
    if !allowed_host(&req, b.config.port) {
        return reply(403, json!({"code":0,"msg":"本地题库访问验证失败"}));
    }
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    if method == Method::HEAD && (path == "/" || path == "/health") {
        return response(200, vec![], "application/json");
    }
    if method == Method::HEAD {
        return response(404, vec![], "application/json");
    }
    if method == Method::GET && path == "/health" {
        return reply(
            200,
            json!({"service":"ocs-codex","status":"running","provider":b.config.provider,"model":b.config.selected_model()}),
        );
    }
    let parts: Vec<_> = path.splitn(4, '/').collect();
    if parts.len() != 4
        || parts[1] != "t"
        || !bool::from(parts[2].as_bytes().ct_eq(b.config.token.as_bytes()))
    {
        return reply(403, json!({"code":0,"msg":"本地题库访问验证失败"}));
    }
    if method == Method::OPTIONS {
        return response(204, vec![], "application/json");
    }
    let route = parts[3];
    if method == Method::GET {
        return match route {
            "config.json" => reply(200, b.wrappers()),
            "status" => reply(200, b.status().await),
            "" => response(
                200,
                include_bytes!("../../../assets/bridge/index.html").to_vec(),
                "text/html; charset=utf-8",
            ),
            "page_collector.js" => response(
                200,
                include_bytes!("../../../assets/bridge/page_collector.js").to_vec(),
                "text/javascript; charset=utf-8",
            ),
            "batch-test.html" => response(
                200,
                include_bytes!("../../../assets/bridge/batch-test.html").to_vec(),
                "text/html; charset=utf-8",
            ),
            _ => reply(404, json!({"code":0,"msg":"接口不存在"})),
        };
    }
    if method != Method::POST || route != "answer" {
        return reply(404, json!({"code":0,"msg":"接口不存在"}));
    }
    if !req
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| {
            s.split(';')
                .next()
                .unwrap()
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
    {
        return reply(415, json!({"code":0,"msg":"请使用 application/json"}));
    }
    let len = req
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    if !(1..=262144).contains(&len) {
        // Dropping an unread upload can reset the TCP connection on Windows
        // before the client receives 413. Discard it without buffering, using
        // the same deadline as accepted requests so a stalled upload is bounded.
        let mut body = req.into_body().into_data_stream();
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(chunk) = body.next().await {
                if chunk.is_err() {
                    break;
                }
            }
        })
        .await;
        return reply(413, json!({"code":0,"msg":"请求须在 1 至 262144 字节之间"}));
    }
    let invalid = || reply(400, json!({"code":0,"msg":"题目参数无效或 JSON 格式错误"}));
    let body = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(req.into_body(), 262144),
    )
    .await
    {
        Ok(Ok(b)) => b,
        _ => return invalid(),
    };
    let data = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v,
        Err(_) => return invalid(),
    };
    let prepared = match prepare(&data) {
        Ok(v) => v,
        Err(_) => return invalid(),
    };
    match b.answer(prepared).await {
        Ok(v) => reply(200, v),
        Err(e) => reply(200, json!({"code":0,"msg":e})),
    }
}
