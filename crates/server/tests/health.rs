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
