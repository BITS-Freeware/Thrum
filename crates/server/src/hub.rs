//! Live delivery hub: which devices are connected right now.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thrum_core::protocol::ServerFrame;
use tokio::sync::mpsc;

/// Tracks the WebSocket connection of every authenticated device.
///
/// A device is registered under its device id; a re-login or reconnect
/// replaces the previous entry, and the previous connection's channel is
/// closed by dropping its receiver clone.
#[derive(Debug, Default, Clone)]
pub struct WsHub {
    inner: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<ServerFrame>>>>,
}

impl WsHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a connected device, replacing any previous connection.
    pub fn register(&self, device_id: &str, sender: mpsc::UnboundedSender<ServerFrame>) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(device_id.to_string(), sender);
    }

    /// Forget a device connection.
    pub fn remove(&self, device_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(device_id);
    }

    /// Push a frame to every listed device that is currently connected.
    /// Devices that are offline simply miss it; the delivery queue covers
    /// them when they reconnect.
    pub fn broadcast(&self, device_ids: &[String], frame: &ServerFrame) {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for device_id in device_ids {
            if let Some(sender) = map.get(device_id) {
                let _ = sender.send(frame.clone());
            }
        }
    }
}
