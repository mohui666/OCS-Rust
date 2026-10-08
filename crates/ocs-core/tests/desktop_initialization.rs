use ocs_core::{
    local_server::{self, LocalServer},
    storage::Storage,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn fresh_storage_preserves_missing_scripts_for_frontend_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let store = Storage::with_key(
        temp.path().join("data"),
        temp.path().join("resources"),
        [7; 32],
    )
    .unwrap();
    assert!(!store.snapshot()["render"]
        .as_object()
        .unwrap()
        .contains_key("scripts"));
}

#[tokio::test]
async fn userscript_install_url_retains_capability_and_registration_checks() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Storage::with_key(
            temp.path().join("data"),
            temp.path().join("resources"),
            [7; 32],
        )
        .unwrap(),
    );
    let script = temp.path().join("test.user.js");
    std::fs::write(
        &script,
        "// ==UserScript==\n// @name Test\n// ==/UserScript==",
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    {
        let mut value = store.value.lock().unwrap();
        value["server"] = json!({"port": address.port(), "authToken": "test-capability"});
        value["render"]["scripts"] = json!([{"isLocalScript": true, "url": script}]);
    }
    let state = Arc::new(LocalServer {
        store,
        events: Arc::new(|_| {}),
        ai_control: Arc::new(|_, _| Box::pin(async { Ok(json!({})) })),
    });
    let server = tokio::spawn(local_server::serve(listener, state));
    let client = reqwest::Client::new();
    for route in ["/api/local-userscript", "/api/local-userscript/ocs.user.js"] {
        let mut url = reqwest::Url::parse(&format!("http://{address}{route}")).unwrap();
        url.query_pairs_mut()
            .append_pair("path", script.to_str().unwrap());
        assert_eq!(client.get(url.clone()).send().await.unwrap().status(), 403);
        url.query_pairs_mut()
            .append_pair("token", "test-capability");
        let response = client.get(url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("// @name Test"));
    }
    let mut url = reqwest::Url::parse(&format!(
        "http://{address}/api/local-userscript/ocs.user.js"
    ))
    .unwrap();
    url.query_pairs_mut()
        .append_pair(
            "path",
            temp.path().join("unregistered.user.js").to_str().unwrap(),
        )
        .append_pair("token", "test-capability");
    assert_eq!(client.get(url).send().await.unwrap().status(), 403);
    server.abort();
}
