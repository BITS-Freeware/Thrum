mod common;

use common::TestApp;

#[tokio::test]
async fn healthz_answers_ok() {
    let app = TestApp::spawn().await;
    let response = app
        .client
        .get(app.url("/healthz"))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    let policy = response
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(policy.contains("wasm-unsafe-eval"), "{policy}");
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    assert_eq!(response.text().await.expect("body"), "ok");
}

#[tokio::test]
async fn unknown_route_is_404() {
    let app = TestApp::spawn().await;
    let response = app
        .client
        .get(app.url("/nope"))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 404);
}

/// An unknown API path answers a JSON 404, and a known path with the wrong
/// method still answers 405.
#[tokio::test]
async fn unknown_api_paths_are_json_404s() {
    let app = common::TestApp::spawn().await;
    let response = app.get("/v1/nope", None).await;
    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "not_found");
    let response = app.get("/v1/invites/redeem", None).await;
    assert_eq!(response.status(), 405);
}
