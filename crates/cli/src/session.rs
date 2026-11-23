//! The session file: where the CLI keeps its login between commands.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// One logged-in device on one server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub server: String,
    pub username: String,
    pub token: String,
    pub device_id: String,
}

/// Load the session from `path`.
pub fn load(path: &Path) -> Result<Session, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("corrupt state file: {error}"))
}

/// Save the session to `path`, readable by the owner only: it holds the
/// device's bearer token.
pub fn save(path: &Path, session: &Session) -> Result<(), String> {
    let text = serde_json::to_string_pretty(session).expect("session serializes");
    thrum_core::mls::write_private_file(path, text.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Load the session and require it to belong to `server`.
pub fn load_for_server(path: &Path, server: &str) -> Result<Session, String> {
    let session = load(path)?;
    if session.server != server {
        return Err(format!(
            "state file belongs to {}, not {server}; log in again",
            session.server
        ));
    }
    Ok(session)
}
