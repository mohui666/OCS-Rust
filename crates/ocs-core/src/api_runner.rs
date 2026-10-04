use crate::{answer::Question, bridge::Config, runner::Runner};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{header, Client, Url};
use serde_json::{json, Value};
use std::time::Duration;

/// Direct HTTPS transport. It never starts a CLI, local model, or browser.
pub struct ApiRunner {
    client: Client,
}

impl ApiRunner {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }

    pub fn endpoint(config: &Config) -> Result<Url> {
        let suffix = match config.api_protocol.as_str() {
            "chat_completions" => "/chat/completions",
            "responses" => "/responses",
            _ => bail!("未知的 AI API 协议"),
        };
        let mut url = Url::parse(config.api_base_url.trim()).context("AI API 地址格式错误")?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            bail!("AI API 地址必须使用 HTTPS；本机网关可使用 HTTP")
        }
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("AI API 地址不能包含账号、密钥、查询参数或片段")
        }
        let path = url.path().trim_end_matches('/');
        // Accept either a versioned base URL or an explicit endpoint.
        let endpoint = if !path.ends_with(suffix) {
            if path.ends_with("/responses") || path.ends_with("/chat/completions") {
                bail!("AI API 地址与所选协议不一致")
            }
            format!("{path}{suffix}")
        } else {
            path.to_string()
        };
        url.set_path(&endpoint);
        Ok(url)
    }

    fn payload(questions: &[Question], config: &Config) -> Result<Value> {
        let schema: Value =
            serde_json::from_str(include_str!("../../../assets/bridge/batch.schema.json"))?;
        let questions: Vec<_> = questions
            .iter()
            .map(|q| {
                let mut value = json!(q);
                value["id"] = json!(q.key());
                value
            })
            .collect();
        let instructions = format!(
            "{}\n返回的 JSON 必须符合以下 Schema：\n{}",
            include_str!("../../../assets/bridge/api-prompt.txt"),
            schema
        );
        let messages = json!([
            {"role":"system", "content":instructions},
            {"role":"user", "content":json!({"questions":questions}).to_string()}
        ]);
        let mut body = json!({"model":config.api_model.trim(),"stream":false});
        if let Some(temperature) = config.api_temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(tokens) = config.api_max_output_tokens.filter(|v| *v > 0) {
            let field = if config.api_protocol == "responses" {
                "max_output_tokens"
            } else {
                &config.api_token_parameter
            };
            body[field] = json!(tokens);
        }
        let format = match config.api_response_format.as_str() {
            "json_schema" => Some(
                json!({"type":"json_schema","name":"ocs_answers","strict":true,"schema":schema}),
            ),
            "json_object" => Some(json!({"type":"json_object"})),
            "prompt" => None,
            _ => bail!("未知的 AI API 答案格式"),
        };
        if config.api_protocol == "responses" {
            body["input"] = messages;
            body["store"] = json!(false);
            if let Some(format) = format {
                body["text"] = json!({"format":format});
            }
            if !config.api_reasoning_effort.is_empty() {
                body["reasoning"] = json!({"effort":config.api_reasoning_effort});
            }
        } else {
            body["messages"] = messages;
            if let Some(mut format) = format {
                if config.api_response_format == "json_schema" {
                    format.as_object_mut().unwrap().remove("type");
                    format = json!({"type":"json_schema","json_schema":format});
                }
                body["response_format"] = format;
            }
            if !config.api_reasoning_effort.is_empty() {
                body["reasoning_effort"] = json!(config.api_reasoning_effort);
            }
        }
        Ok(body)
    }

    async fn send_json(&self, request: reqwest::RequestBuilder, config: &Config) -> Result<Value> {
        let mut authorization =
            header::HeaderValue::from_str(&format!("Bearer {}", config.api_key.trim()))
                .context("AI API Key 格式错误")?;
        authorization.set_sensitive(true);
        let response = request
            .header(header::AUTHORIZATION, authorization)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("AI API 连接失败：{}", e.without_url()))?;
        let status = response.status();
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| anyhow::anyhow!("AI API 响应读取失败：{}", e.without_url()))?;
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                bail!("AI API 响应超过 8 MiB")
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let detail = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| match status.as_u16() {
                    401 | 403 => "请检查 API Key 和模型访问权限".into(),
                    404 => "请检查接口地址、协议和模型 ID".into(),
                    429 => "额度不足或触发速率限制".into(),
                    _ => "请检查接口配置或稍后重试".into(),
                });
            let detail: String = detail
                .replace(config.api_key.trim(), "[redacted]")
                .chars()
                .take(512)
                .collect();
            bail!("AI API HTTP {}：{}", status.as_u16(), detail)
        }
        serde_json::from_slice(&bytes).context("AI API 响应不是有效 JSON")
    }

    pub async fn models(config: &Config) -> Result<Vec<String>> {
        config.validate()?;
        if config.api_key.trim().is_empty() {
            bail!("请先输入 API Key")
        }
        let runner = Self::new()?;
        let mut url = Self::endpoint(config)?;
        let base = url
            .path()
            .strip_suffix("/chat/completions")
            .or_else(|| url.path().strip_suffix("/responses"))
            .context("无法确定模型列表地址")?;
        url.set_path(&format!("{base}/models"));
        let value = tokio::time::timeout(
            Duration::from_secs(30),
            runner.send_json(runner.client.get(url), config),
        )
        .await
        .context("获取模型列表超时，可手动填写模型 ID")??;
        let entries = value["data"]
            .as_array()
            .context("接口未返回标准模型列表，可手动填写模型 ID")?;
        let mut models: Vec<String> = entries
            .iter()
            .filter_map(|v| v["id"].as_str())
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .take(2000)
            .map(str::to_owned)
            .collect();
        models.sort();
        models.dedup();
        if models.is_empty() {
            bail!("接口返回的模型列表为空，可手动填写模型 ID")
        }
        Ok(models)
    }

    async fn request(&self, questions: &[Question], config: &Config) -> Result<Value> {
        let value = self
            .send_json(
                self.client
                    .post(Self::endpoint(config)?)
                    .json(&Self::payload(questions, config)?),
                config,
            )
            .await?;
        let text = if config.api_protocol == "responses" {
            if matches!(
                value["status"].as_str(),
                Some("incomplete" | "failed" | "cancelled")
            ) {
                bail!("AI API 未完成答案生成；请检查模型输出限制和服务状态")
            }
            let mut text = String::new();
            if let Some(output) = value["output"].as_array() {
                for item in output {
                    if item["type"] != "message" {
                        continue;
                    }
                    if let Some(content) = item["content"].as_array() {
                        for part in content {
                            if part["type"] == "refusal" {
                                bail!("AI API 拒绝返回本批答案")
                            }
                            if part["type"] == "output_text" {
                                text.push_str(part["text"].as_str().unwrap_or(""));
                            }
                        }
                    }
                }
            }
            text
        } else {
            let choice = &value["choices"][0];
            if matches!(
                choice["finish_reason"].as_str(),
                Some("length" | "content_filter")
            ) {
                bail!("AI API 答案被截断或过滤；未分配不完整答案")
            }
            if choice["message"]["refusal"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
            {
                bail!("AI API 拒绝返回本批答案")
            }
            choice["message"]["content"]
                .as_str()
                .context("AI API 缺少答案正文")?
                .to_owned()
        };
        let text = text.trim();
        let text = if text.starts_with("```") && text.ends_with("```") {
            text.split_once('\n')
                .context("AI API JSON 代码块不完整")?
                .1
                .strip_suffix("```")
                .unwrap()
                .trim()
        } else {
            text
        };
        let mut result: Value =
            serde_json::from_str(text).context("AI API 返回的答案不是有效 JSON；请调整答案格式")?;
        if !result.is_object() || !result["results"].is_array() {
            bail!("AI API 答案缺少 results 数组")
        }
        // No web-search tool is enabled in this transport; never imply otherwise.
        result["web_search_calls"] = json!(0);
        for item in result["results"].as_array_mut().unwrap() {
            if !item.is_object() {
                bail!("AI API 单题答案格式错误")
            }
            item["sources"] = json!([]);
        }
        Ok(result)
    }
}

#[async_trait]
impl Runner for ApiRunner {
    async fn run(&self, questions: &[Question], config: &Config) -> Result<Value> {
        config.validate_ready()?;
        let request = self.request(questions, config);
        if let Some(timeout) = config.timeout() {
            tokio::time::timeout(timeout, request).await.map_err(|_| {
                anyhow::anyhow!("AI API 请求超时；可增加超时秒数，或设为 0 无限等待")
            })?
        } else {
            request.await
        }
    }
}
