use crate::{answer::Question, bridge::Config};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, questions: &[Question], config: &Config) -> Result<Value>;
}
pub struct CodexRunner;

pub fn isolate(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x08000000 | 0x00000200);
    command.kill_on_drop(true);
}
pub struct ProcessTree(pub Option<u32>);
impl Drop for ProcessTree {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .creation_flags(0x08000000)
                    .output();
            }
        }
    }
}
async fn drain(mut reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if result.len() < 8 * 1024 * 1024 {
            result.extend_from_slice(&buf[..n]);
        }
    }
    Ok(result)
}
/// Drains both pipes concurrently, bounds retained output, and kills the process group on timeout/cancellation.
pub async fn captured(
    mut cmd: Command,
    input: &[u8],
    timeout: Option<Duration>,
) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    isolate(&mut cmd);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("无法启动程序")?;
    let tree = ProcessTree(child.id());
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let work = async {
        let write = async {
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let (_, out, err, status) = tokio::try_join!(
            async { write.await.map_err(anyhow::Error::from) },
            drain(stdout),
            drain(stderr),
            async { child.wait().await.map_err(anyhow::Error::from) }
        )?;
        Ok::<_, anyhow::Error>((status.success(), out, err))
    };
    let result = if let Some(t) = timeout {
        tokio::time::timeout(t, work)
            .await
            .map_err(|_| anyhow::anyhow!("Codex 请求超时，请稍后重试或调大本地服务超时"))?
    } else {
        work.await
    };
    drop(tree);
    result
}

pub fn command(config: &Config, directory: &Path, output: &Path, schema: &Path) -> Command {
    let mut c = Command::new(&config.codex_bin);
    c.args([
        "exec",
        "--ignore-user-config",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--json",
        "--color",
        "never",
        "-C",
    ])
    .arg(directory)
    .arg("--output-schema")
    .arg(schema)
    .arg("-o")
    .arg(output)
    .arg("-m")
    .arg(&config.model)
    .arg("-c")
    .arg(format!(
        "model_reasoning_effort={}",
        json!(config.reasoning_effort)
    ))
    .args([
        "-c",
        "project_doc_max_bytes=0",
        "-c",
        "web_search=\"live\"",
        "-c",
        "approval_policy=\"never\"",
    ]);
    for feature in [
        "shell_tool",
        "unified_exec",
        "apps",
        "plugins",
        "multi_agent",
        "memories",
        "hooks",
        "computer_use",
        "browser_use",
        "image_generation",
        "unbounded_connection_retries",
    ] {
        c.args(["--disable", feature]);
    }
    c.arg("-").env("CODEX_HOME", &config.codex_home);
    for k in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] {
        c.env_remove(k);
    }
    c
}

fn codex_failure(stdout: &[u8], stderr: &[u8]) -> &'static str {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
    .to_lowercase();
    if ["unexpected argument", "unknown feature", "unrecognized option"]
        .iter()
        .any(|s| text.contains(s))
    {
        return "Codex 命令行版本不兼容，请在 AI 题库设置中选择新版 Codex 程序";
    }
    if ["usage limit", "quota", "rate limit", "429"]
        .iter()
        .any(|s| text.contains(s))
    {
        return "Codex 额度或速率受限，请在 Codex 中查看额度后重试";
    }
    if ["unauthorized", "401", "not logged", "refresh_token"]
        .iter()
        .any(|s| text.contains(s))
    {
        return "Codex 登录已失效，请在终端执行 codex login";
    }
    if ["not supported", "model_not_found"]
        .iter()
        .any(|s| text.contains(s))
    {
        return "当前 Codex 账号不支持所选模型，请修改本地 config.json";
    }
    "Codex 调用失败，请检查登录状态和网络后重试"
}

#[async_trait]
impl Runner for CodexRunner {
    async fn run(&self, questions: &[Question], config: &Config) -> Result<Value> {
        if config.model.trim().is_empty() {
            bail!("请先在题库设置中选择当前账号可用的模型")
        }
        let dir = tempfile::Builder::new().prefix("ocs-rust-").tempdir()?;
        let schema = dir.path().join("schema.json");
        let output = dir.path().join("answer.json");
        std::fs::write(
            &schema,
            include_bytes!("../../../assets/bridge/batch.schema.json"),
        )?;
        let questions: Vec<_> = questions
            .iter()
            .map(|q| {
                let mut v = json!(q);
                v["id"] = json!(q.key());
                v
            })
            .collect();
        let prompt = format!(
            "{}\n当前日期：{}\n题目数据：\n{}",
            include_str!("../../../assets/bridge/prompt.txt"),
            chrono::Local::now().format("%Y-%m-%d"),
            json!({"questions":questions})
        );
        let (success, stdout, stderr) = captured(
            command(config, dir.path(), &output, &schema),
            prompt.as_bytes(),
            config.timeout(),
        )
        .await?;
        if !success {
            bail!(codex_failure(&stdout, &stderr))
        }
        if !output.is_file() {
            bail!("Codex 没有返回答案")
        }
        let mut result: Value = serde_json::from_slice(&std::fs::read(output)?)
            .context("Codex 返回的答案不是有效 JSON")?;
        let count = String::from_utf8_lossy(&stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|v| v["type"] == "item.completed" && v["item"]["type"] == "web_search")
            .count();
        result["web_search_calls"] = json!(count);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::codex_failure;

    #[test]
    fn incompatible_cli_is_not_reported_as_login_or_network_failure() {
        for error in [
            "error: unexpected argument '--ignore-user-config' found",
            "Error: Unknown feature flag: browser_use",
            "unrecognized option --ephemeral",
        ] {
            let message = codex_failure(b"", error.as_bytes());
            assert!(message.contains("版本不兼容"));
            assert!(!message.contains("登录"));
            assert!(!message.contains("网络"));
        }
    }

    #[test]
    fn existing_failure_categories_remain_distinct() {
        for (error, expected) in [
            ("usage limit", "额度"),
            ("401 Unauthorized", "登录已失效"),
            ("model_not_found", "不支持所选模型"),
            ("connection refused", "网络"),
        ] {
            assert!(codex_failure(error.as_bytes(), b"").contains(expected));
        }
    }
}
