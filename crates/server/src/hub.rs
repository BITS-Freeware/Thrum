//! Live delivery hub: which devices are connected right now.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use thrum_core::protocol::ServerFrame;
use tokio::sync::mpsc;

/// One registered connection: its id and the only sender to its socket.
#[derive(Debug)]
struct Connection {
    id: u64,
    sender: mpsc::UnboundedSender<ServerFrame>,
}

/// Tracks the WebSocket connection of every authenticated device.
///
/// A device has at most one live connection. The hub holds the only sender
/// into each connection's outbound channel, so dropping an entry closes
/// that socket once its buffered frames are written: a reconnect replaces,
/// and closes, the older connection, and ending a device closes its socket.
/// A connection that ends only removes its own entry, never a newer one.
#[derive(Debug, Default, Clone)]
pub struct WsHub {
    inner: Arc<Mutex<HashMap<String, Connection>>>,
    next_id: Arc<AtomicU64>,
}

impl WsHub {
    pub fn new() -> Self {
        Self::default()
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Connection>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Register a connected device, replacing (and so closing) any previous
    /// connection. Returns the connection id to pass to [`WsHub::remove`].
    /// The caller must not keep a clone of `sender`.
    pub fn register(&self, device_id: &str, sender: mpsc::UnboundedSender<ServerFrame>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.map()
            .insert(device_id.to_string(), Connection { id, sender });
        id
    }

    /// Forget a connection that ended, if it is still the registered one.
    pub fn remove(&self, device_id: &str, connection_id: u64) {
        let mut map = self.map();
        if map
            .get(device_id)
            .is_some_and(|connection| connection.id == connection_id)
        {
            map.remove(device_id);
        }
    }

    /// Close the devices' live connections, if any: they were revoked,
    /// logged out or deleted.
    pub fn disconnect(&self, device_ids: &[String]) {
        let mut map = self.map();
        for device_id in device_ids {
            map.remove(device_id);
        }
    }

    /// Push a frame to every listed device that is currently connected.
    /// Devices that are offline simply miss it; the delivery queue covers
    /// them when they reconnect.
    pub fn broadcast(&self, device_ids: &[String], frame: &ServerFrame) {
        let map = self.map();
        for device_id in device_ids {
            if let Some(connection) = map.get(device_id) {
                let _ = connection.sender.send(frame.clone());
            }
        }
    }
}
