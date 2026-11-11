mod common;

use common::{TestApp, login, register};
use serde_json::{Value, json};

#[tokio::test]
async fn devices_list_and_revoke() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let first = login(&app, "alice", "correct horse battery", "laptop").await;
    let second = login(&app, "alice", "correct horse battery", "phone").await;
    let t1 = first["token"].as_str().expect("token").to_string();
    let t2 = second["token"].as_str().expect("token").to_string();
    let d1 = first["device_id"].as_str().expect("device id").to_string();
    let d2 = second["device_id"].as_str().expect("device id").to_string();

    // Two logins, two devices, both active.
    let response = app.get("/v1/devices", Some(&t1)).await;
    assert_eq!(response.status(), 200);
    let devices: Vec<Value> = response.json().await.expect("devices");
    assert_eq!(devices.len(), 2);
    assert!(devices.iter().all(|device| device["active"] == json!(true)));
    let labels: Vec<&str> = devices
        .iter()
        .filter_map(|device| device["label"].as_str())
        .collect();
    assert!(labels.contains(&"laptop") && labels.contains(&"phone"));

    // Revoke the second device; revoking again is fine.
    let response = app.delete(&format!("/v1/devices/{d2}"), Some(&t1)).await;
    assert_eq!(response.status(), 204);
    let response = app.delete(&format!("/v1/devices/{d2}"), Some(&t1)).await;
    assert_eq!(response.status(), 204);

    // The revoked device's token is dead.
    let response = app.get("/v1/devices", Some(&t2)).await;
    assert_eq!(response.status(), 401);

    // The list still shows the device, as inactive.
    let response = app.get("/v1/devices", Some(&t1)).await;
    assert_eq!(response.status(), 200);
    let devices: Vec<Value> = response.json().await.expect("devices");
    assert_eq!(devices.len(), 2);
    let revoked = devices
        .iter()
        .find(|device| device["id"].as_str() == Some(d2.as_str()))
        .expect("revoked device still listed");
    assert_eq!(revoked["active"], json!(false));

    // Another user cannot revoke alice's device, and unknown ids are 404.
    register(&app, "bob", "correct horse battery").await;
    let bob = login(&app, "bob", "correct horse battery", "phone").await;
    let bob_token = bob["token"].as_str().expect("token");
    let response = app
        .delete(&format!("/v1/devices/{d1}"), Some(bob_token))
        .await;
    assert_eq!(response.status(), 404);
    let response = app
        .delete(
            "/v1/devices/00000000-0000-0000-0000-000000000000",
            Some(&t1),
        )
        .await;
    assert_eq!(response.status(), 404);

    // No token at all is refused.
    let response = app.get("/v1/devices", None).await;
    assert_eq!(response.status(), 401);
}
