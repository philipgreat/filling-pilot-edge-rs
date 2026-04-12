//! Status processor

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct StatusCache {
    pub cache: Arc<RwLock<HashMap<String, String>>>,
}

impl StatusCache {
    pub fn new() -> Self {
        Self { cache: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn update(&self, station_id: &str, status: &str) {
        let mut cache = self.cache.write().await;
        cache.insert(station_id.to_string(), status.to_string());
    }

    pub async fn get_all(&self) -> Vec<serde_json::Value> {
        let cache = self.cache.read().await;
        cache.iter()
            .map(|(k, v)| serde_json::json!({ "fillStation": k, "status": v }))
            .collect()
    }
}

impl Default for StatusCache {
    fn default() -> Self { Self::new() }
}

pub struct StatusProcessor {
    status_cache: Arc<StatusCache>,
}

impl StatusProcessor {
    pub fn new(status_cache: Arc<StatusCache>) -> Self {
        Self { status_cache }
    }

    pub async fn get_status(&self) -> String {
        let statuses = self.status_cache.get_all().await;
        serde_json::to_string(&statuses).unwrap_or_else(|_| "[]".to_string())
    }
}
