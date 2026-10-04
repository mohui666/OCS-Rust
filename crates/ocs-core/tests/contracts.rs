use anyhow::{bail, Result};
use async_trait::async_trait;
use ocs_core::{
    answer::{format_answer, Question},
    bridge::{self, prepare, Bridge, Config},
    runner::{captured, Runner},
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[test]
fn decrypts_node_aes_gcm_fixture() {
    let v: Value = serde_json::from_str(include_str!("crypto.json")).unwrap();
    let k: [u8; 32] = hex::decode(v["key"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        ocs_core::storage::decrypt(v["encrypted"].as_str().unwrap(), &k).unwrap(),
        v["text"]
    );
}
#[test]
fn archive_rejects_traversal_and_preserves_existing_destination() {
    use std::io::Write;
    let t = tempfile::tempdir().unwrap();
    let z = t.path().join("bad.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
    writer
        .start_file("../escaped", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"bad").unwrap();
    writer.finish().unwrap();
    assert!(ocs_core::platform::unzip(&z, &t.path().join("out")).is_err());
    assert!(!t.path().join("escaped").exists());
}
#[cfg(unix)]
#[test]
fn archive_supports_contained_framework_symlinks() {
    use std::io::Write;
    let t = tempfile::tempdir().unwrap();
    let z = t.path().join("framework.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
    writer
        .start_file(
            "Versions/137/binary",
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )
        .unwrap();
    writer.write_all(b"binary").unwrap();
    writer
        .add_symlink(
            "Versions/Current",
            "137",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.finish().unwrap();
    let out = t.path().join("out");
    ocs_core::platform::unzip(&z, &out).unwrap();
    assert_eq!(
        std::fs::read(out.join("Versions/Current/binary")).unwrap(),
        b"binary"
    );
    assert!(ocs_core::platform::unzip(&z, &out).is_err());
}

#[test]
fn python_differential_contracts() {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("contracts.json")).unwrap();
    for (index, f) in fixtures.iter().enumerate() {
        let q = Question::parse(&f["input"]);
        if f["error"].is_string() {
            assert_eq!(
                q.unwrap_err().to_string(),
                f["error"].as_str().unwrap(),
                "case {index}"
            );
            continue;
        }
        let q = q.unwrap();
        assert_eq!(json!(q), f["normalized"], "normalize {index}");
        assert_eq!(q.key(), f["key"], "hash {index}");
        let answer = format_answer(&q, &f["raw"]);
        if f["answer_error"].is_string() {
            assert_eq!(
                answer.unwrap_err().to_string(),
                f["answer_error"].as_str().unwrap(),
                "answer error {index}"
            )
        } else {
            assert_eq!(answer.unwrap(), f["answer"], "answer {index}")
        }
    }
}
#[derive(Default)]
struct Fake {
    calls: AtomicUsize,
    mode: AtomicUsize,
    delay_ms: u64,
}
#[async_trait]
impl Runner for Fake {
    async fn run(&self, questions: &[Question], _: &Config) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        let mut items:Vec<_>=questions.iter().map(|q|json!({"id":q.key(),"answers":[q.title],"confident":true,"explanation":"synthetic","sources":["https://example.com","file:///private"]})).collect();
        match self.mode.load(Ordering::SeqCst) {
            1 => {
                items.pop();
            }
            2 => {
                if items.len() > 1 {
                    items[1] = items[0].clone();
                }
            }
            3 => bail!("登录失效"),
            4 => {
                items[0]["id"] = json!("foreign");
            }
            5 => items[0]["confident"] = json!(false),
            6 => items[0]["sources"] = json!([5]),
            _ => {}
        }
        items.reverse();
        Ok(json!({"results":items,"web_search_calls":2}))
    }
}
fn question(i: usize) -> Value {
    json!({"title":format!("题目 {i}"),"options":"","type":"completion"})
}
fn page(n: usize) -> Vec<Value> {
    (0..n).map(question).collect()
}
fn batch(q: Value, qs: &[Value]) -> Value {
    let mut q = q;
    q["pageQuestions"] = json!(qs);
    q
}
fn make(fake: Arc<Fake>) -> Arc<Bridge> {
    Bridge::with_runner(
        Config {
            model: "mock".into(),
            timeout_seconds: Some(0.0),
            ..Default::default()
        },
        fake,
    )
    .unwrap()
}

#[tokio::test]
async fn full_page_and_cache() {
    for n in [4, 85, 200] {
        let f = Arc::new(Fake::default());
        let b = make(f.clone());
        let qs = page(n);
        for (i, q) in qs.iter().enumerate() {
            let v = b
                .answer(prepare(&batch(q.clone(), &qs)).unwrap())
                .await
                .unwrap();
            assert_eq!(v["answer"], q["title"]);
            assert_eq!(v["batch_size"], n);
            assert_eq!(v["cached"], i > 0);
            assert_eq!(v["sources"], json!(["https://example.com"]));
        }
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn missing_duplicate_foreign_ids_do_not_cache() {
    for mode in [1, 2, 4] {
        let f = Arc::new(Fake::default());
        f.mode.store(mode, Ordering::SeqCst);
        let b = make(f.clone());
        assert!(b
            .answer(prepare(&batch(question(0), &page(3))).unwrap())
            .await
            .is_err());
        assert_eq!(b.status().await["cache_size"], 0);
        assert_eq!(b.status().await["inflight"], 0);
        f.mode.store(0, Ordering::SeqCst);
        assert!(b.answer(prepare(&question(0)).unwrap()).await.is_ok());
    }
}
#[tokio::test]
async fn parallel_requests_share_one_batch() {
    let f = Arc::new(Fake {
        delay_ms: 80,
        ..Default::default()
    });
    let b = make(f.clone());
    let b1 = b.clone();
    let first = tokio::spawn(async move {
        b1.answer(prepare(&batch(question(0), &page(10))).unwrap())
            .await
    });
    while f.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let second = b.answer(prepare(&question(8)).unwrap()).await.unwrap();
    assert_eq!(second["shared"], true);
    assert!(first.await.unwrap().is_ok());
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn disconnected_client_does_not_cancel_batch() {
    let f = Arc::new(Fake {
        delay_ms: 80,
        ..Default::default()
    });
    let b = make(f.clone());
    let b1 = b.clone();
    let first = tokio::spawn(async move {
        b1.answer(prepare(&batch(question(0), &page(3))).unwrap())
            .await
    });
    while f.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    first.abort();
    let next = b.answer(prepare(&question(1)).unwrap()).await.unwrap();
    assert_eq!(next["answer"], "题目 1");
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn capacity_rejects_unrelated_pages() {
    let f = Arc::new(Fake {
        delay_ms: 80,
        ..Default::default()
    });
    let b = Bridge::with_runner(
        Config {
            max_concurrency: 1,
            ..Default::default()
        },
        f.clone(),
    )
    .unwrap();
    let b1 = b.clone();
    let first = tokio::spawn(async move { b1.answer(prepare(&question(0)).unwrap()).await });
    while f.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert!(b
        .answer(prepare(&question(1)).unwrap())
        .await
        .unwrap_err()
        .contains("其他页面"));
    first.await.unwrap().unwrap();
}
#[tokio::test]
async fn failures_release_capacity_and_are_visible() {
    let f = Arc::new(Fake::default());
    let b = make(f.clone());
    f.mode.store(3, Ordering::SeqCst);
    assert_eq!(
        b.answer(prepare(&question(0)).unwrap()).await.unwrap_err(),
        "登录失效"
    );
    assert_eq!(b.status().await["cache_size"], 0);
    f.mode.store(0, Ordering::SeqCst);
    assert!(b.answer(prepare(&question(0)).unwrap()).await.is_ok());
}
#[tokio::test]
async fn changed_options_are_not_cached() {
    let f = Arc::new(Fake::default());
    let b = make(f.clone());
    b.answer(prepare(&question(0)).unwrap()).await.unwrap();
    let mut changed = question(0);
    changed["options"] = json!("新选项");
    b.answer(prepare(&changed).unwrap()).await.unwrap();
    assert_eq!(f.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn manual_title_enriches_only_unique_match() {
    let f = Arc::new(Fake::default());
    let b = make(f.clone());
    let result = b
        .answer(prepare(&json!({"title":"题目 0","pageQuestions":page(3)})).unwrap())
        .await
        .unwrap();
    assert_eq!(result["batch_size"], 3);
    assert_eq!(
        b.answer(prepare(&question(0)).unwrap()).await.unwrap()["cached"],
        true
    );
}
#[tokio::test]
async fn lru_capacity_is_bounded() {
    let b = make(Arc::new(Fake::default()));
    for i in 0..270 {
        b.answer(prepare(&question(i)).unwrap()).await.unwrap();
    }
    assert_eq!(b.status().await["cache_size"], 256);
}
#[tokio::test]
async fn invalid_sources_become_per_question_failure() {
    let f = Arc::new(Fake::default());
    f.mode.store(6, Ordering::SeqCst);
    let b = make(f);
    let v = b
        .answer(prepare(&batch(question(0), &page(2))).unwrap())
        .await
        .unwrap();
    assert_eq!(v["code"], 0);
    assert_eq!(
        b.answer(prepare(&question(1)).unwrap()).await.unwrap()["code"],
        1
    );
}
#[test]
fn invalid_batches_rejected() {
    for v in [
        Value::Null,
        json!({}),
        json!(page(201)),
        json!([{"title":""}]),
    ] {
        assert!(prepare(&json!({"title":"test","pageQuestions":v})).is_err());
    }
    assert!(prepare(&batch(question(300), &page(200))).is_err());
}
#[test]
fn timeout_semantics_and_env() {
    for (s, unlimited) in [(None, true), (Some(0.0), true), (Some(600.0), false)] {
        let c = Config {
            timeout_seconds: s,
            ..Default::default()
        };
        assert_eq!(c.timeout().is_none(), unlimited);
    }
    let p = std::path::Path::new("/tmp/fixture");
    let c = ocs_core::runner::command(&Config::default(), p, p, p);
    let args: Vec<_> = c.as_std().get_args().collect();
    assert!(args.iter().any(|v| *v == "--ignore-user-config"));
    for k in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] {
        assert!(c
            .as_std()
            .get_envs()
            .any(|(name, val)| name == k && val.is_none()));
    }
}

async fn server() -> (Arc<Bridge>, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let b = Bridge::with_runner(
        Config {
            port,
            token: "test-token".into(),
            timeout_seconds: Some(0.0),
            ..Default::default()
        },
        Arc::new(Fake::default()),
    )
    .unwrap();
    let router = bridge::router(b.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (b, format!("http://127.0.0.1:{port}"), task)
}
#[tokio::test]
async fn http_contract_and_security() {
    let (b, base, task) = server().await;
    let c = reqwest::Client::new();
    let answer = format!("{base}/t/test-token/answer");
    assert_eq!(
        c.head(format!("{base}/?t=1"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        c.head(format!("{base}/unknown"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        c.get(format!("{base}/health"))
            .header("Host", "evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        c.post(format!("{base}/t/wrong/answer"))
            .json(&question(0))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        c.post(&answer)
            .header("Content-Type", "text/plain")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        415
    );
    assert_eq!(
        c.post(&answer)
            .header("Content-Type", "application/json")
            .body("x".repeat(262145))
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    assert_eq!(
        c.post(&answer)
            .header("Content-Type", "application/json")
            .body("bad")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(b.status().await["codex_calls"], 0);
    let v: Value = c
        .post(&answer)
        .json(&question(0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["code"], 1);
    let v: Value = c
        .get(format!("{base}/t/test-token/config.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v[0]["method"], "post");
    assert_eq!(v[0]["url"], answer);
    let r = c
        .request(reqwest::Method::OPTIONS, &answer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(r.headers()["Access-Control-Allow-Private-Network"], "true");
    let status: Value = c
        .get(format!("{base}/t/test-token/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["timeout_seconds"], Value::Null);
    assert_eq!(status["timeout_unlimited"], true);
    task.abort();
}
#[cfg(unix)]
#[tokio::test]
async fn child_pipes_do_not_deadlock() {
    let mut c = tokio::process::Command::new("python3");
    c.args([
        "-c",
        "import sys;sys.stdout.write('o'*200000);sys.stderr.write('e'*200000)",
    ]);
    let (ok, out, err) = captured(c, b"", Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(ok);
    assert_eq!(out.len(), 200000);
    assert_eq!(err.len(), 200000);
}
#[cfg(unix)]
#[tokio::test]
async fn finite_timeout_kills_owned_process_group() {
    let mut c = tokio::process::Command::new("python3");
    c.args(["-c", "import time;time.sleep(30)"]);
    let start = std::time::Instant::now();
    assert!(captured(c, b"", Some(Duration::from_millis(100)))
        .await
        .unwrap_err()
        .to_string()
        .contains("超时"));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[tokio::test]
async fn worker_supervision_request_and_shutdown() {
    use ocs_core::worker::WorkerPool;
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("worker.py");
    std::fs::write(
        &entry,
        r#"import sys,json
for line in sys.stdin:
 r=json.loads(line)
 print(json.dumps({'id':r['id'],'result':r['args']}),flush=True)
 if r['event']=='close':break
"#,
    )
    .unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = events.clone();
    let pool = WorkerPool::new(
        "python3".into(),
        entry,
        Arc::new(move |v| sink.lock().unwrap().push(v)),
    );
    pool.start("one").await.unwrap();
    assert!(pool.start("one").await.is_err());
    assert_eq!(
        pool.call("one", "init", json!([1, 2])).await.unwrap(),
        json!([1, 2])
    );
    assert!(pool.call("one", "arbitrary_eval", json!([])).await.is_err());
    pool.shutdown().await.unwrap();
    assert!(pool.active().await.is_empty());
    pool.start("two").await.unwrap();
    pool.stop("two").await.unwrap();
    assert!(pool.active().await.is_empty());
}
