use anyhow::{bail, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::LazyLock};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Question {
    pub title: String,
    pub options: String,
    #[serde(rename = "type")]
    pub kind: String,
}

pub fn lines(s: &str) -> impl Iterator<Item = &str> {
    s.split([
        '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
        '\u{2029}',
    ])
}
fn space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}
pub fn trimmed(s: &str) -> &str {
    s.trim_matches(space)
}
pub fn normalized_title(s: &str) -> String {
    s.split(space)
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Question {
    pub fn parse(data: &Value) -> Result<Self> {
        let o = data
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("请求必须是 JSON 对象"))?;
        let title = o.get("title").and_then(Value::as_str).unwrap_or("");
        if trimmed(title).is_empty() {
            bail!("题目不能为空")
        }
        let options = match o.get("options") {
            None => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(a)) if a.iter().all(Value::is_string) => a
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => bail!("options 和 type 必须是字符串"),
        };
        let kind = match o.get("type") {
            None => "",
            Some(Value::String(s)) => s,
            _ => bail!("options 和 type 必须是字符串"),
        };
        if title.chars().count() > 16000 || options.chars().count() > 16000 {
            bail!("题目或选项过长（最多 16000 字符）")
        }
        Ok(Self {
            title: trimmed(title).into(),
            options: if ["${options}", "undefined", "null"].contains(&options.as_str()) {
                String::new()
            } else {
                trimmed(&options).into()
            },
            kind: if ["${type}", "undefined", "null"].contains(&kind) {
                String::new()
            } else {
                trimmed(kind).into()
            },
        })
    }

    /// Python's ensure_ascii=False, sort_keys=True serialization is a wire contract.
    pub fn key(&self) -> String {
        static HSPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new("[ \\t\u{a0}]+").unwrap());
        let options = lines(&self.options)
            .filter(|x| !trimmed(x).is_empty())
            .map(|x| trimmed(&HSPACE.replace_all(x, " ")).to_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let canonical = format!(
            "{{\"options\": {}, \"title\": {}, \"type\": {}}}",
            json!(options),
            json!(normalized_title(&self.title)),
            json!(self.kind)
        );
        hex::encode(Sha256::digest(canonical.as_bytes()))
    }
}

pub fn format_answer(q: &Question, raw: &Value) -> Result<Value> {
    let confident = raw
        .get("confident")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("Codex 返回格式无效"))?;
    let explanation = raw
        .get("explanation")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Codex 返回格式无效"))?;
    let a = raw
        .get("answers")
        .and_then(Value::as_array)
        .filter(|v| v.iter().all(Value::is_string))
        .ok_or_else(|| anyhow::anyhow!("Codex 返回格式无效"))?;
    if !confident {
        return Ok(
            json!({"code":0,"question":q.title,"answer":"","msg":if explanation.is_empty() {"信息不足，未生成可靠答案"} else {explanation}}),
        );
    }
    let mut answers: Vec<String> = a
        .iter()
        .map(|x| trimmed(x.as_str().unwrap()).to_owned())
        .collect();
    if answers.is_empty() || answers.iter().any(String::is_empty) {
        bail!("Codex 未返回有效答案")
    }
    if ["single", "judgement"].contains(&q.kind.as_str()) && answers.len() != 1 {
        bail!("答案数量与题型不匹配")
    }
    if ["single", "multiple"].contains(&q.kind.as_str()) && !q.options.is_empty() {
        static LABEL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?s)^\s*([A-ZＡ-Ｚ])[.．、:：)）]\s*(.+)$").unwrap());
        let options: Vec<_> = lines(&q.options)
            .map(trimmed)
            .filter(|x| !x.is_empty())
            .collect();
        let mut labels = HashMap::new();
        let texts: Vec<_> = options
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let m = LABEL.captures(s);
                let text = m
                    .as_ref()
                    .map(|m| trimmed(m.get(2).unwrap().as_str()))
                    .unwrap_or(s);
                let label = m
                    .as_ref()
                    .map(|m| m[1].chars().next().unwrap())
                    .filter(char::is_ascii)
                    .unwrap_or(char::from_u32(65 + i as u32).unwrap());
                labels.insert(label.to_string(), text);
                text
            })
            .collect();
        let mut resolved = Vec::new();
        for a in answers {
            let value = if texts.contains(&a.as_str()) {
                a
            } else if let Some(i) = options.iter().position(|s| *s == a) {
                texts[i].into()
            } else if let Some(s) = labels.get(&a) {
                (*s).into()
            } else {
                bail!("生成的选择题答案与给定选项不匹配，请人工核对")
            };
            if !resolved.contains(&value) {
                resolved.push(value)
            }
        }
        answers = resolved;
    }
    if q.kind == "judgement" {
        answers = vec![match answers[0].to_lowercase().trim_matches(['。', '.']) {
            "正确" | "对" | "是" | "true" | "√" | "1" => "正确",
            "错误" | "错" | "否" | "false" | "×" | "0" => "错误",
            _ => bail!("判断题答案格式无效"),
        }
        .into()];
    }
    Ok(
        json!({"code":1,"question":q.title,"answer":answers.join("#"),"answers":answers,"explanation":explanation,"msg":"成功"}),
    )
}
