use std::net::{IpAddr, SocketAddr};

use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use thrum_server::config::Config;
use thrum_server::routes::ws::spawn_sweeper;
use thrum_server::state::AppState;

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();
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
    if !bind.is_loopback() {
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
    .await
    .unwrap_or_else(|error| panic!("server error: {error}"));
}
