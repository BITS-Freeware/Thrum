use std::net::SocketAddr;

use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("GEIST3_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let app = geist3_server::build_router();
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {addr}: {error}"));
    println!("geist3-server listening on {addr}");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("server error: {error}"));
}
