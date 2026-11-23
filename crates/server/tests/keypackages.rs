mod common;

use common::{TestApp, key_package, last_resort_package, login, register};

#[tokio::test]
async fn upload_and_fetch_key_packages() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id").to_string();

    // Upload two key packages for alice's device.
    let uploaded = [key_package(&alice), key_package(&alice)];
    for content in uploaded.clone() {
        let response = app
            .client
            .post(app.url("/v1/key-packages"))
            .bearer_auth(&token)
            .body(content)
            .send()
            .await
            .expect("upload");
        assert_eq!(response.status(), 201);
    }

    // Another user fetches them, oldest first, one at a time.
    register(&app, "bob", "correct horse battery").await;
    let bob = login(&app, "bob", "correct horse battery", "phone").await;
    let bob_token = bob["token"].as_str().expect("token");

    let response = app
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(bob_token),
        )
        .await;
    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.expect("key package");
    assert_eq!(&bytes[..], &uploaded[0][..]);

    let response = app
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(bob_token),
        )
        .await;
    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.expect("key package");
    assert_eq!(&bytes[..], &uploaded[1][..]);

    // Both packages are consumed now.
    let response = app
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(bob_token),
        )
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn key_packages_need_auth_and_respect_limits() {
    let app = TestApp::spawn().await;

    // Uploading without a token is refused.
    let response = app
        .client
        .post(app.url("/v1/key-packages"))
        .body(b"kp".to_vec())
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), 401);

    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token");

    // An empty package is refused.
    let response = app
        .client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(token)
        .body(Vec::new())
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), 400);

    // An oversized package is refused.
    let oversized = vec![0u8; 64 * 1024 + 1];
    let response = app
        .client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(token)
        .body(oversized)
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), 413);
}

#[tokio::test]
async fn revoked_device_key_packages_are_unfetchable() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id").to_string();

    let response = app
        .client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(&token)
        .body(key_package(&alice))
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), 201);

    register(&app, "bob", "correct horse battery").await;
    let bob = login(&app, "bob", "correct horse battery", "phone").await;
    let bob_token = bob["token"].as_str().expect("token");

    // Revoking the device removes its key packages.
    let response = app
        .delete(&format!("/v1/devices/{device_id}"), Some(&token))
        .await;
    assert_eq!(response.status(), 204);
    let response = app
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(bob_token),
        )
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn anonymous_key_package_fetch_is_rejected() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id");
    app.client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(&token)
        .body(key_package(&alice))
        .send()
        .await
        .expect("upload");
    let response = app
        .get(&format!("/v1/devices/{device_id}/key-package"), None)
        .await;
    assert_eq!(response.status(), 401);
    let response = app.get("/v1/key-packages/count", Some(&token)).await;
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn key_package_stock_is_capped_and_last_resort_survives_drain() {
    let app = TestApp::spawn_with(thrum_server::config::Config {
        key_package_max_per_device: 2,
        ..thrum_server::config::Config::default()
    })
    .await;
    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id").to_string();
    register(&app, "bob", "correct horse battery").await;
    let bob = login(&app, "bob", "correct horse battery", "phone").await;
    let bob_token = bob["token"].as_str().expect("token").to_string();

    let stock = [key_package(&alice), key_package(&alice)];
    for body in stock.clone() {
        let response = app
            .client
            .post(app.url("/v1/key-packages"))
            .bearer_auth(&token)
            .body(body)
            .send()
            .await
            .expect("upload");
        assert_eq!(response.status(), 201);
    }
    let response = app
        .client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(&token)
        .body(key_package(&alice))
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), 409);

    let resort = last_resort_package(&alice);
    let response = app
        .client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(&token)
        .header("x-thrum-key-package-role", "last-resort")
        .body(resort.clone())
        .send()
        .await
        .expect("last resort");
    assert_eq!(response.status(), 201);

    for expected in &stock {
        let response = app
            .get(
                &format!("/v1/devices/{device_id}/key-package"),
                Some(&bob_token),
            )
            .await;
        assert_eq!(response.status(), 200);
        assert_eq!(&response.bytes().await.unwrap()[..], &expected[..]);
    }
    for _ in 0..2 {
        let response = app
            .get(
                &format!("/v1/devices/{device_id}/key-package"),
                Some(&bob_token),
            )
            .await;
        assert_eq!(response.status(), 200);
        assert_eq!(&response.bytes().await.unwrap()[..], &resort[..]);
    }
}

#[tokio::test]
async fn key_package_fetch_is_rate_limited_per_caller_and_device() {
    let app = TestApp::spawn_with(thrum_server::config::Config {
        key_package_fetch_per_caller_per_minute: 2,
        key_package_fetch_per_device_per_minute: 100,
        ..thrum_server::config::Config::default()
    })
    .await;
    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id").to_string();
    for _ in 0..3 {
        app.client
            .post(app.url("/v1/key-packages"))
            .bearer_auth(&token)
            .body(key_package(&alice))
            .send()
            .await
            .expect("upload");
    }
    register(&app, "bob", "correct horse battery").await;
    let bob = login(&app, "bob", "correct horse battery", "phone").await;
    let bob_token = bob["token"].as_str().expect("token").to_string();
    for _ in 0..2 {
        let response = app
            .get(
                &format!("/v1/devices/{device_id}/key-package"),
                Some(&bob_token),
            )
            .await;
        assert_eq!(response.status(), 200);
    }
    let response = app
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(&bob_token),
        )
        .await;
    assert_eq!(response.status(), 429);

    let limited = TestApp::spawn_with(thrum_server::config::Config {
        key_package_fetch_per_caller_per_minute: 100,
        key_package_fetch_per_device_per_minute: 1,
        ..thrum_server::config::Config::default()
    })
    .await;
    register(&limited, "alice", "correct horse battery").await;
    let alice = login(&limited, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id").to_string();
    for _ in 0..2 {
        limited
            .client
            .post(limited.url("/v1/key-packages"))
            .bearer_auth(&token)
            .body(key_package(&alice))
            .send()
            .await
            .expect("upload");
    }
    register(&limited, "bob", "correct horse battery").await;
    let bob = login(&limited, "bob", "correct horse battery", "phone").await;
    register(&limited, "carol", "correct horse battery").await;
    let carol = login(&limited, "carol", "correct horse battery", "tablet").await;
    let response = limited
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(bob["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(response.status(), 200);
    let response = limited
        .get(
            &format!("/v1/devices/{device_id}/key-package"),
            Some(carol["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(response.status(), 429);
}

#[tokio::test]
async fn a_key_package_must_be_for_the_uploading_device_and_keep_its_key() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await;
    let token = alice["token"].as_str().expect("token").to_string();
    let device_id = alice["device_id"].as_str().expect("device id").to_string();
    register(&app, "bob", "correct horse battery").await;
    let bob = login(&app, "bob", "correct horse battery", "phone").await;
    let upload = |body: Vec<u8>| {
        app.client
            .post(app.url("/v1/key-packages"))
            .bearer_auth(&token)
            .body(body)
            .send()
    };

    // Bytes that are not a key package, and a package for another device.
    assert_eq!(
        upload(b"not a key package".to_vec())
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(upload(key_package(&bob)).await.unwrap().status(), 400);
    let elsewhere = thrum_core::mls::DeviceCrypto::generate(format!("alice#{device_id}-x"));
    assert_eq!(
        upload(elsewhere.new_key_package().unwrap())
            .await
            .unwrap()
            .status(),
        400
    );

    // The first valid package binds the key; another key is refused.
    assert_eq!(upload(key_package(&alice)).await.unwrap().status(), 201);
    let impostor = thrum_core::mls::DeviceCrypto::generate(format!("alice#{device_id}"));
    assert_eq!(
        upload(impostor.new_key_package().unwrap())
            .await
            .unwrap()
            .status(),
        409
    );

    // The device list carries the bound key.
    let response = app
        .get(
            "/v1/users/alice/devices",
            Some(bob["token"].as_str().unwrap()),
        )
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    use base64::Engine as _;
    let bound = base64::engine::general_purpose::STANDARD
        .decode(
            body["signature_keys"][&device_id]
                .as_str()
                .expect("bound key"),
        )
        .unwrap();
    assert_eq!(bound, common::device_keys(&alice).signature_key());
}
