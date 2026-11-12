use std::net::{IpAddr, SocketAddr};

use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use geist3_server::config::Config;
use geist3_server::routes::ws::spawn_sweeper;
use geist3_server::state::AppState;

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();
    // GEIST3_DB is a SQLite file path (default) or a postgres:// URL.
    let db_url = std::env::var("GEIST3_DB").unwrap_or_else(|_| "geist3.sqlite".to_string());
    let state = AppState::connect_database(&db_url, cfg)
        .await
        .unwrap_or_else(|error| panic!("failed to open database {db_url}: {error}"));
    spawn_sweeper(state.clone());
    let port: u16 = std::env::var("GEIST3_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let bind: IpAddr = match std::env::var("GEIST3_BIND") {
        Ok(value) => value
            .parse()
            .unwrap_or_else(|error| panic!("bad GEIST3_BIND {value:?}: {error}")),
        Err(_) => IpAddr::from([0, 0, 0, 0]),
    };
    let addr = SocketAddr::new(bind, port);
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {addr}: {error}"));

    // GEIST3_WEB_DIR serves the built web client from the same origin, so
    // a small self-hosted install needs no reverse proxy: the web client
    // talks to `window.location.origin` by default. API routes take
    // precedence; everything else falls back to index.html (SPA routing).
    let router = match std::env::var("GEIST3_WEB_DIR") {
        Ok(web_dir) => {
            let index = std::path::Path::new(&web_dir).join("index.html");
            geist3_server::build_router(state).fallback_service(
                ServeDir::new(&web_dir)
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new(index)),
            )
        }
        Err(_) => geist3_server::build_router(state),
    };

    println!("geist3-server listening on {addr}");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap_or_else(|error| panic!("server error: {error}"));
}
