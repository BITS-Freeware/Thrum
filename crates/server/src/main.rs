use std::net::SocketAddr;

use tokio::net::TcpListener;

use geist3_server::config::Config;
use geist3_server::state::AppState;

#[tokio::main]
async fn main() {
    let cfg = Config::default();
    let db_path = std::env::var("GEIST3_DB").unwrap_or_else(|_| "geist3.sqlite".to_string());
    let state = AppState::connect_sqlite(&db_path, cfg)
        .await
        .unwrap_or_else(|error| panic!("failed to open database {db_path}: {error}"));
    let port: u16 = std::env::var("GEIST3_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {addr}: {error}"));
    println!("geist3-server listening on {addr}");
    axum::serve(
        listener,
        geist3_server::build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap_or_else(|error| panic!("server error: {error}"));
}
