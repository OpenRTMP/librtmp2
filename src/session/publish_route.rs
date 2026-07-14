use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Shared publish-route ownership table used by the built-in server to enforce
/// single-publisher-per-route without storing non-UnwindSafe closure trait objects
/// on `Conn`.
#[derive(Clone)]
pub(crate) struct PublishRouteRegistry {
    routes: Arc<Mutex<HashMap<(String, String), u64>>>,
}

impl PublishRouteRegistry {
    pub(crate) fn new(routes: Arc<Mutex<HashMap<(String, String), u64>>>) -> Self {
        Self { routes }
    }

    pub(crate) fn claim(&self, conn_id: u64, app: &str, stream: &str) -> bool {
        let key = (app.to_string(), stream.to_string());
        let Ok(mut map) = self.routes.lock() else {
            return false;
        };
        match map.get(&key) {
            Some(&owner) if owner != conn_id => false,
            _ => {
                map.insert(key, conn_id);
                true
            }
        }
    }

    pub(crate) fn release(&self, conn_id: u64, app: &str, stream: &str) {
        let key = (app.to_string(), stream.to_string());
        if let Ok(mut map) = self.routes.lock() {
            if map.get(&key) == Some(&conn_id) {
                map.remove(&key);
            }
        }
    }
}
