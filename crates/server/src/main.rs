use std::net::{IpAddr, SocketAddr};

use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use thrum_server::config::Config;
use thrum_server::routes::ws::spawn_sweeper;
use thrum_server::state::AppState;

#[tokio::main]
async fn main() {
    // `thrum-server healthcheck`: ask a running server on this host whether
    // it answers, for container health checks (the image has no curl).
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(healthcheck().await);
    }
    let cfg = Config::from_env().unwrap_or_else(|error| {
        eprintln!("error: bad configuration: {error}");
        std::process::exit(2);
    });
    println!("thrum-server configuration: {cfg:?}");
    // THRUM_DB is a SQLite file path (default) or a postgres:// URL.
    let db_url = std::env::var("THRUM_DB").unwrap_or_else(|_| "thrum.sqlite".to_string());
    let state = AppState::connect_database(&db_url, cfg)
        .await
        .unwrap_or_else(|error| panic!("failed to open database {db_url}: {error}"));
    spawn_sweeper(state.clone());
    let port: u16 = std::env::var("THRUM_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let bind: IpAddr = match std::env::var("THRUM_BIND") {
        Ok(value) => value
            .parse()
            .unwrap_or_else(|error| panic!("bad THRUM_BIND {value:?}: {error}")),
        // Loopback unless the operator opts in. The container image sets
        // THRUM_BIND=0.0.0.0 because the process has to accept the proxy's
        // connection from outside the network namespace.
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    };
    // The container image sets THRUM_BIND and records the same value in
    // THRUM_IMAGE_BIND. A bind that is still the image's own default gets a
    // one-line note instead of the full warning, so the warning means
    // something when an operator chose the address.
    let image_default = std::env::var("THRUM_IMAGE_BIND")
        .ok()
        .and_then(|value| value.parse::<IpAddr>().ok())
        == Some(bind);
    if !bind.is_loopback() && image_default {
        eprintln!(
            "note: listening on {bind} inside the container; publish port {port} on \
             127.0.0.1 only and put a TLS proxy in front (docs/self-hosting.md)."
        );
    } else if !bind.is_loopback() {
        eprintln!(
            "warning: thrum-server is bound to {bind}, which is not a loopback address. \
             Passwords and session tokens travel in the clear unless a TLS reverse proxy \
             terminates HTTPS in front of this process. Do not publish port {port} on a \
             public interface."
        );
    }
    let addr = SocketAddr::new(bind, port);
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {addr}: {error}"));

    // THRUM_WEB_DIR serves the built web client from the same origin, so
    // a small self-hosted install needs no reverse proxy: the web client
    // talks to `window.location.origin` by default. API routes take
    // precedence; everything else falls back to index.html (SPA routing).
    let db = state.db.clone();
    let router = match std::env::var("THRUM_WEB_DIR") {
        Ok(web_dir) => {
            let index = std::path::Path::new(&web_dir).join("index.html");
            thrum_server::build_router(state).fallback_service(
                ServeDir::new(&web_dir)
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new(index)),
            )
        }
        Err(_) => thrum_server::build_router(state),
    };

    println!("thrum-server listening on {addr}");
    axum::serve(
        listener,
        thrum_server::with_security_headers(router)
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap_or_else(|error| panic!("server error: {error}"));

    // Closing the pool lets SQLite checkpoint its write-ahead log into the
    // database file, so a copy of that file alone is a complete backup.
    if tokio::time::timeout(std::time::Duration::from_secs(5), db.close())
        .await
        .is_err()
    {
        eprintln!("warning: database connections did not close within 5 seconds");
    }
    println!("thrum-server stopped");
}

/// Exit status for `thrum-server healthcheck`: 0 when `GET /healthz` on
/// the configured port answers 200 within five seconds.
async fn healthcheck() -> i32 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port: u16 = std::env::var("THRUM_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let check = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.ok()?;
        Some(response.starts_with(b"HTTP/1.1 200"))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), check).await {
        Ok(Some(true)) => 0,
        _ => 1,
    }
}

/// Resolve on SIGTERM (how `docker stop` and systemd stop a service) or
/// Ctrl-C. Requests in flight finish; new connections are refused.
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
    println!("thrum-server shutting down");
}
