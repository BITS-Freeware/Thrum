use std::net::SocketAddr;

use tokio::net::TcpListener;

use thrum_server::config::Config;
use thrum_server::routes::ws::spawn_sweeper;
use thrum_server::state::AppState;

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();
    let db_path = std::env::var("THRUM_DB").unwrap_or_else(|_| "thrum.sqlite".to_string());
    let state = AppState::connect_sqlite(&db_path, cfg)
        .await
        .unwrap_or_else(|error| panic!("failed to open database {db_path}: {error}"));
    spawn_sweeper(state.clone());
    let port: u16 = std::env::var("THRUM_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {addr}: {error}"));
    println!("thrum-server listening on {addr}");
    axum::serve(
        listener,
        thrum_server::build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap_or_else(|error| panic!("server error: {error}"));
}
