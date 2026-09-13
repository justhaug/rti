//! `rti serve`: a long-running process that owns the archive, runs the
//! research loop in a background thread, exposes a JSON API and serves a
//! single-page UI with an "operator" agent the user can direct.

pub mod api;
pub mod operator;
pub mod state;
pub mod ui;

use std::path::Path;
use std::sync::Arc;

use state::AppState;

/// Bind on `addr` (e.g. "127.0.0.1:0"), start worker threads, return the
/// bound port. The server keeps running in background threads.
pub fn serve_on(root: &Path, addr: &str) -> anyhow::Result<(u16, Arc<AppState>)> {
    let state = Arc::new(AppState::open(root)?);
    let server = tiny_http::Server::http(addr).map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let server = Arc::new(server);
    for i in 0..4 {
        let server = server.clone();
        let state = state.clone();
        std::thread::Builder::new()
            .name(format!("rti-http-{i}"))
            .spawn(move || {
                for req in server.incoming_requests() {
                    api::handle(&state, req);
                }
            })?;
    }
    state.push_event("server", &format!("listening on port {port}"));
    Ok((port, state))
}

/// Serve forever on `127.0.0.1:port`.
pub fn serve(root: &Path, port: u16) -> anyhow::Result<()> {
    let (port, _state) = serve_on(root, &format!("127.0.0.1:{port}"))?;
    println!("RTI server: http://127.0.0.1:{port}/");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
