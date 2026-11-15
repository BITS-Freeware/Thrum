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
