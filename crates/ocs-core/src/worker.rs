use crate::runner::{isolate, ProcessTree};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{oneshot, Mutex, Notify},
};

pub type EventSink = Arc<dyn Fn(Value) + Send + Sync>;
type Pending = Arc<StdMutex<HashMap<String, oneshot::Sender<Result<Value>>>>>;
struct Worker {
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    kill: Notify,
    finished: AtomicBool,
    done: Notify,
}
pub struct WorkerPool {
    node: PathBuf,
    entry: PathBuf,
    workers: Arc<Mutex<HashMap<String, Arc<Worker>>>>,
    sink: EventSink,
}
impl WorkerPool {
    pub fn new(node: PathBuf, entry: PathBuf, sink: EventSink) -> Self {
        Self {
            node,
            entry,
            workers: Arc::new(Mutex::new(HashMap::new())),
            sink,
        }
    }
    pub async fn start(&self, uid: &str) -> Result<()> {
        if uid.is_empty()
            || uid.len() > 128
            || !uid
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!("无效的浏览器标识")
        }
        let mut all = self.workers.lock().await;
        if all.contains_key(uid) {
            bail!("该浏览器已经运行")
        }
        let mut cmd = Command::new(&self.node);
        cmd.arg(&self.entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for k in [
            "NODE_OPTIONS",
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "CODEX_ACCESS_TOKEN",
        ] {
            cmd.env_remove(k);
        }
        isolate(&mut cmd);
        let mut child = cmd.spawn().context("无法启动浏览器适配器")?;
        let mut tree = Some(ProcessTree(child.id()));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let w = Arc::new(Worker {
            stdin: Mutex::new(child.stdin.take().unwrap()),
            pending: Arc::new(StdMutex::new(HashMap::new())),
            kill: Notify::new(),
            finished: AtomicBool::new(false),
            done: Notify::new(),
        });
        all.insert(uid.into(), w.clone());
        drop(all);
        let sink = self.sink.clone();
        let name = uid.to_string();
        let pending = w.pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Some(id) = v["id"].as_str() {
                        if let Some(tx) = pending.lock().unwrap().remove(id) {
                            let _ = tx.send(if let Some(e) = v["error"].as_str() {
                                Err(anyhow::anyhow!(e.to_string()))
                            } else {
                                Ok(v["result"].clone())
                            });
                        }
                    } else {
                        sink(json!({"uid":name,"event":v["event"],"args":v["args"]}));
                    }
                } else {
                    sink(json!({"uid":name,"event":"log","args":[line]}));
                }
            }
        });
        let sink = self.sink.clone();
        let name = uid.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                sink(json!({"uid":name,"event":"log","args":[line]}));
            }
        });
        let sink = self.sink.clone();
        let name = uid.to_string();
        let workers = self.workers.clone();
        tokio::spawn(async move {
            let status = tokio::select! {s=child.wait()=>s.ok().and_then(|s|s.code()),_=w.kill.notified()=>{drop(tree.take());let _=child.kill().await;let _=child.wait().await;None}};
            drop(tree);
            for (_, tx) in w.pending.lock().unwrap().drain() {
                let _ = tx.send(Err(anyhow::anyhow!("浏览器适配器已退出")));
            }
            workers.lock().await.remove(&name);
            w.finished.store(true, Ordering::SeqCst);
            w.done.notify_waiters();
            sink(json!({"uid":name,"event":"exit","args":[status]}));
        });
        Ok(())
    }
    pub async fn call(&self, uid: &str, event: &str, args: Value) -> Result<Value> {
        if ![
            "init",
            "launch",
            "close",
            "bringToFront",
            "gotoWebRTCPage",
            "closeWebRTCPage",
            "kill",
            "snapshot",
            "goto",
        ]
        .contains(&event)
        {
            bail!("未允许的浏览器命令")
        }
        let w = self
            .workers
            .lock()
            .await
            .get(uid)
            .cloned()
            .context("浏览器未运行")?;
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        w.pending.lock().unwrap().insert(id.clone(), tx);
        let bytes = serde_json::to_vec(&json!({"id":id,"event":event,"args":args}))?;
        let written = async {
            let mut stdin = w.stdin.lock().await;
            stdin.write_all(&bytes).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        }
        .await;
        if let Err(e) = written {
            w.pending.lock().unwrap().remove(&id);
            return Err(e.into());
        }
        // Launch can include multiple extension install pages; close remains bounded.
        let timeout = Duration::from_secs(if event == "launch" {
            600
        } else if event == "close" {
            8
        } else {
            30
        });
        let result = tokio::time::timeout(timeout, rx).await;
        w.pending.lock().unwrap().remove(&id);
        match result {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => bail!("浏览器适配器已退出"),
            Err(_) => {
                if event == "close" {
                    w.kill.notify_one();
                }
                bail!("浏览器命令超时：{event}")
            }
        }
    }
    pub async fn stop(&self, uid: &str) -> Result<()> {
        let worker = self.workers.lock().await.get(uid).cloned();
        if let Some(w) = worker {
            let done = w.done.notified();
            w.kill.notify_one();
            if !w.finished.load(Ordering::SeqCst) {
                tokio::time::timeout(Duration::from_secs(5), done)
                    .await
                    .context("浏览器进程尚未完成退出")?;
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<()> {
        let ids: Vec<_> = self.workers.lock().await.keys().cloned().collect();
        for id in ids {
            let _ = self.call(&id, "close", json!([])).await;
            self.stop(&id).await?;
        }
        Ok(())
    }
    pub async fn active(&self) -> Vec<String> {
        self.workers.lock().await.keys().cloned().collect()
    }
}
