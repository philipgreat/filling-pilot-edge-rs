//! Monitor - matches Java Monitor.java + MonitorProcessor.java
//! 
//! Features:
//! - Reads PLC data at configured frequency and sends to cloud/local endpoint
//! - Only sends when data changes or resendInterval has elapsed
//! - Persists monitor config to ~/.edge/monitor-meta.json every 60s
//! - HTTP POST to localAddress with decoded data + pilotCallback support

use std::collections::{HashSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

use crate::s7::S7Manager;
use crate::deploy::{PLCSerializeMetaFactory, Deployment, Deployments};

/// Monitor configuration (parsed from plcInfo JSON, persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: String,
    pub merchant_id: String,
    pub ip: String,
    pub port: u16,
    pub name: String,
    pub code: String,
    pub db_number: u16,
    pub offset: u32,
    pub block_size: u32,
    #[serde(rename = "type")]
    pub monitor_type: String,
    /// Collection frequency in ms (min 50ms)
    pub frequency: u64,
    /// Resend interval in ms if data unchanged (min 1000ms)
    pub resend_interval: u64,
    pub forward_address: String,
    pub local_address: String,
}

impl Monitor {
    /// Parse monitors from plcInfo JSON (array of PLC objects)
    pub fn parse_from_plc_info(plc_info: &[serde_json::Value]) -> Vec<Self> {
        let mut monitors = Vec::new();
        for plc in plc_info {
            let ip = match plc.get("ipAddress").and_then(|v| v.as_str()) {
                Some(ip) => ip.to_string(),
                None => continue,
            };
            let port = match plc.get("portNumber").and_then(|v| v.as_u64()) {
                Some(p) => p as u16,
                None => continue,
            };

            if let Some(monitor_list) = plc.get("monitorList").and_then(|v| v.as_array()) {
                for m in monitor_list {
                    let frequency = m.get("frequency").and_then(|v| v.as_u64()).unwrap_or(50).max(50);
                    let resend_interval = m.get("resendInterval").and_then(|v| v.as_u64()).unwrap_or(1000).max(1000);
                    
                    let monitor = Monitor {
                        id: m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        merchant_id: m.get("merchantId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        ip: ip.clone(),
                        port,
                        name: m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        code: m.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        db_number: m.get("dbNumber").and_then(|v| v.as_u64()).unwrap_or(0) as u16,
                        offset: m.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        block_size: m.get("blockSize").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        monitor_type: m.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        frequency,
                        resend_interval,
                        forward_address: m.get("forwardAddress").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        local_address: m.get("localAddress").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    };
                    monitors.push(monitor);
                }
            }
        }
        monitors
    }

    /// Load monitors from disk (blocking, called on startup)
    pub fn load_from_disk(path: &std::path::PathBuf) -> Vec<Monitor> {
        if !path.exists() {
            return Vec::new();
        }
        match std::fs::read_to_string(path) {
            Ok(content) => {
                match serde_json::from_str::<Vec<Monitor>>(&content) {
                    Ok(monitors) => {
                        info!("[MONITOR] Loaded {} monitors from {}", monitors.len(), path.display());
                        monitors
                    }
                    Err(e) => {
                        warn!("[MONITOR] Failed to parse {}: {}", path.display(), e);
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                warn!("[MONITOR] Failed to read {}: {}", path.display(), e);
                Vec::new()
            }
        }
    }

    /// Persist monitors to disk (called every 60s)
    pub fn persist_to_disk(monitors: &[Monitor], path: &std::path::PathBuf) {
        if monitors.is_empty() {
            return;
        }
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(monitors) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    warn!("[MONITOR] Failed to write {}: {}", path.display(), e);
                } else {
                    info!("[MONITOR] Persisted {} monitors to {}", monitors.len(), path.display());
                }
            }
            Err(e) => {
                warn!("[MONITOR] Failed to serialize monitors: {}", e);
            }
        }
    }
}

/// Runtime state for a monitor (last read/send time, data cache)
#[derive(Debug, Clone)]
struct MonitorState {
    last_read_time: Option<u64>,
    last_send_time: Option<u64>,
    data: String,
    running: bool,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self {
            last_read_time: None,
            last_send_time: None,
            data: String::new(),
            running: false,
        }
    }
}

/// Monitor processor - manages all monitors and triggers reads
pub struct MonitorProcessor {
    /// Active monitors (config) - keyed by id for uniqueness
    monitors: Arc<RwLock<HashSet<Monitor>>>,
    /// Per-monitor runtime state
    states: Arc<RwLock<HashMap<String, MonitorState>>>,
    /// S7 manager for PLC reads
    s7_manager: Arc<S7Manager>,
    /// Metadata factory for codec decoding
    meta_factory: Arc<PLCSerializeMetaFactory>,
    /// Fill stations map: id -> (ip, port)
    fill_stations: Arc<RwLock<HashMap<String, (String, u16)>>>,
    /// Persistence file path
    persist_path: std::path::PathBuf,
    /// Last persistence time
    last_persist: std::sync::Mutex<u64>,
}

impl MonitorProcessor {
    /// Create a new processor
    pub fn new(
        s7_manager: Arc<S7Manager>,
        meta_factory: Arc<PLCSerializeMetaFactory>,
    ) -> Self {
        let persist_path = PLCSerializeMetaFactory::monitor_meta_file_path();
        Self {
            monitors: Arc::new(RwLock::new(HashSet::new())),
            states: Arc::new(RwLock::new(HashMap::new())),
            s7_manager,
            meta_factory,
            fill_stations: Arc::new(RwLock::new(HashMap::new())),
            persist_path,
            last_persist: std::sync::Mutex::new(0),
        }
    }

    /// Load monitors from disk on startup (blocking)
    pub fn load_from_disk(&self) {
        let monitors = Monitor::load_from_disk(&self.persist_path);
        if !monitors.is_empty() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let mut mons = self.monitors.write().await;
                for m in monitors {
                    mons.insert(m);
                }
            });
        }
    }

    /// Start persistence loop (every 60 seconds, blocking thread)
    pub fn start_persistence_loop(&self) {
        let monitors = Arc::clone(&self.monitors);
        let persist_path = self.persist_path.clone();
        let last_persist = &self.last_persist;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let now = rt.block_on(async {
                    let monitors = monitors.read().await;
                    let list: Vec<Monitor> = monitors.iter().cloned().collect();
                    list
                });
                if !now.is_empty() {
                    Monitor::persist_to_disk(&now, &persist_path);
                }
            }
        });
    }

    /// Update monitors from plcInfo (like Java MonitorProcessor.saveMonitors)
    pub async fn update_monitors(&self, new_monitors: Vec<Monitor>) {
        let mut monitors = self.monitors.write().await;
        
        // Remove monitors no longer in config
        let new_set: HashSet<Monitor> = new_monitors.iter().cloned().collect();
        let to_remove: Vec<Monitor> = monitors.difference(&new_set).cloned().collect();
        for m in to_remove {
            monitors.remove(&m);
            let mut states = self.states.write().await;
            states.remove(&m.id);
        }
        
        // Add new monitors
        let to_add: Vec<Monitor> = new_set.difference(&monitors).cloned().collect();
        for m in to_add {
            info!("[MONITOR] Added: {} ({}:{}, DB{}/{}, {}ms)", 
                m.id, m.ip, m.port, m.db_number, m.block_size, m.frequency);
            monitors.insert(m);
        }
    }

    /// Save DataBlockDefinition metadata (like Java PLCSerializeMetaFactory.saveMeta)
    pub async fn save_meta(&self, plc_info: &[serde_json::Value]) {
        self.meta_factory.save_meta(plc_info).await;
    }

    /// Update fill stations map (used for deploy callbacks)
    pub async fn update_fill_stations(&self, stations: Vec<(String, String, u16)>) {
        // stations: (id, ip, port)
        let mut fs = self.fill_stations.write().await;
        for (id, ip, port) in stations {
            fs.insert(id, (ip, port));
        }
    }

    /// Trigger all monitors that are due
    /// Returns list of (msg_type, message) pairs to send to cloud
    pub async fn trigger_monitors_direct(&self) -> Vec<(String, String)> {
        // Clone all monitors FIRST, then drop the lock before any .await
        let monitors: Vec<Monitor> = {
            let mons = self.monitors.read().await;
            mons.iter().cloned().collect()
        };  // ← Lock released here, safe to .await below

        let now = crate::report::cache::now_millis();
        let mut messages = Vec::new();
        
        for monitor in monitors {
            // Acquire per-monitor lock
            let mut states = self.states.write().await;
            let state = states.entry(monitor.id.clone()).or_default();
            
            if state.running {
                continue;
            }
            
            // Check if it's time to read
            let should_read = match state.last_read_time {
                None => true,
                Some(last) => now - last >= monitor.frequency,
            };
            
            if should_read {
                state.running = true;
                let old_data = state.data.clone();
                
                // Read from PLC (lock still held here, but no .await in read call)
                let read_size = monitor.block_size.saturating_sub(monitor.offset);
                let result = self.s7_manager
                    .read_bytes(&monitor.ip, monitor.port, monitor.db_number, monitor.offset as u16, read_size as u16)
                    .await;
                
                match result {
                    Ok(data) => {
                        state.last_read_time = Some(now);
                        
                        // Build full block: offset bytes of 0 + actual data
                        let mut full = vec![0u8; monitor.block_size as usize];
                        let offset = monitor.offset as usize;
                        let data_len = data.len().min(full.len().saturating_sub(offset));
                        full[offset..offset + data_len].copy_from_slice(&data[..data_len]);
                        let new_data = hex::encode(&full);
                        
                        let data_changed = new_data != old_data;
                        
                        if data_changed {
                            state.data = new_data.clone();
                            state.last_send_time = Some(now);
                            state.running = false;
                            
                            // Data changed: send to cloud + localAddress HTTP POST
                            let cloud_msg = self.build_monitor_message(&monitor, &new_data);
                            messages.push(("monitorStatus".to_string(), cloud_msg));
                            
                            // HTTP POST to localAddress (matches Java Monitor.sendData)
                            if !monitor.local_address.is_empty() {
                                self.send_local_http_post(&monitor, &new_data).await;
                            }
                            continue;
                        }
                    }
                    Err(e) => {
                        warn!("[MONITOR] Read failed for {} ({}:{} DB{}): {}", 
                            monitor.id, monitor.ip, monitor.port, monitor.db_number, e);
                    }
                }
                
                state.running = false;
            }
            
            // Check if resend is needed (data unchanged but interval elapsed)
            let should_resend = match state.last_send_time {
                None => false,
                Some(last) => now - last >= monitor.resend_interval,
            };
            
            if should_resend && !state.data.is_empty() {
                state.last_send_time = Some(now);
                let cloud_msg = self.build_monitor_message(&monitor, &state.data);
                messages.push(("monitorStatus".to_string(), cloud_msg));
                
                // Also resend to localAddress if configured
                if !monitor.local_address.is_empty() {
                    self.send_local_http_post(&monitor, &state.data).await;
                }
            }
        }
        
        messages
    }

    /// Send HTTP POST to localAddress with decoded data + handle pilotCallback
    /// (matches Java Monitor.sendData → HttpUtil.post → tryCallBack)
    async fn send_local_http_post(&self, monitor: &Monitor, hex_data: &str) {
        let local_address = &monitor.local_address;
        if local_address.is_empty() {
            return;
        }

        // Decode hex data using codec registry
        let decoded_data = if !monitor.monitor_type.is_empty() {
            match hex::decode(hex_data) {
                Ok(bytes) => {
                    PLCSerializeMetaFactory::decode_as_map(
                        &bytes,
                        &monitor.monitor_type,
                        &self.meta_factory.cache(),
                    ).ok()
                }
                Err(e) => {
                    warn!("[MONITOR] Failed to decode hex for {}: {}", monitor.id, e);
                    None
                }
            }
        } else {
            None
        };

        // Build the request body
        let mut request_map = serde_json::Map::new();
        request_map.insert("id".into(), serde_json::Value::String(monitor.id.clone()));
        request_map.insert("merchantId".into(), serde_json::Value::String(monitor.merchant_id.clone()));
        request_map.insert("ip".into(), serde_json::Value::String(monitor.ip.clone()));
        request_map.insert("port".into(), serde_json::Value::Number(monitor.port.into()));
        request_map.insert("name".into(), serde_json::Value::String(monitor.name.clone()));
        request_map.insert("code".into(), serde_json::Value::String(monitor.code.clone()));
        request_map.insert("dbNumber".into(), serde_json::Value::Number(monitor.db_number.into()));
        request_map.insert("offset".into(), serde_json::Value::Number(monitor.offset.into()));
        request_map.insert("blockSize".into(), serde_json::Value::Number(monitor.block_size.into()));
        request_map.insert("type".into(), serde_json::Value::String(monitor.monitor_type.clone()));
        request_map.insert("frequency".into(), serde_json::Value::Number(monitor.frequency.into()));
        request_map.insert("resendInterval".into(), serde_json::Value::Number(monitor.resend_interval.into()));
        request_map.insert("forwardAddress".into(), serde_json::Value::String(monitor.forward_address.clone()));
        request_map.insert("localAddress".into(), serde_json::Value::String(monitor.local_address.clone()));
        request_map.insert("data".into(), serde_json::Value::String(hex_data.to_string()));
        
        // Merge decoded fields into the request
        if let Some(decoded) = decoded_data {
            for (k, v) in decoded {
                request_map.insert(k, v);
            }
        }

        let request_json = serde_json::to_string_pretty(&serde_json::Value::Object(request_map.clone()))
            .unwrap_or_else(|_| "{}".to_string());

        info!("[MONITOR] HTTP POST to {}: {}", local_address, request_json);

        // Fire-and-forget HTTP POST (blocking client in a spawned task)
        let url = local_address.clone();
        let body = request_json;
        let monitor_ip = monitor.ip.clone();
        let monitor_port = monitor.port;
        let meta_factory = Arc::clone(&self.meta_factory);
        let s7_manager = Arc::clone(&self.s7_manager);
        let fill_stations = Arc::clone(&self.fill_stations);

        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    error!("[MONITOR] Failed to build HTTP client: {}", e);
                    return;
                }
            };

            match client.post(&url).body(body).send().await {
                Ok(resp) => {
                    if let Ok(text) = resp.text().await {
                        info!("[MONITOR] localAddress {} response: {}", url, text);
                        // Handle pilotCallback (matches Java tryCallBack)
                        Self::try_call_back(&text, &s7_manager, &meta_factory, &fill_stations, Some(&monitor_ip), Some(monitor_port)).await;
                    }
                }
                Err(e) => {
                    warn!("[MONITOR] HTTP POST to {} failed: {}", url, e);
                }
            }
        });
    }

    /// Handle pilotCallback from HTTP response (matches Java Monitor.tryCallBack)
    /// 
    /// Supported callbacks:
    /// - "pilotService/deploy": call deploy() with Deployment
    /// - "pilotService/deployAll": call deploy_all() with Deployments
    async fn try_call_back(
        result: &str,
        s7_manager: &Arc<S7Manager>,
        meta_factory: &Arc<PLCSerializeMetaFactory>,
        fill_stations: &Arc<RwLock<HashMap<String, (String, u16)>>>,
        monitor_ip: Option<&str>,
        monitor_port: Option<u16>,
    ) {
        let result = result.trim();
        if result.is_empty() {
            return;
        }

        let resp: crate::deploy::PilotCallbackResponse = match serde_json::from_str(result) {
            Ok(r) => r,
            Err(_) => {
                // Try to parse as generic JSON
                warn!("[MONITOR] Unrecognized callback response: {}", result);
                return;
            }
        };

        let callback = match resp.pilot_callback {
            Some(c) => c,
            None => return,
        };

        info!("[MONITOR] pilotCallback: {}", callback);

        match callback.as_str() {
            "pilotService/deploy" => {
                let deployment: Deployment = match serde_json::from_str(result) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("[MONITOR] Failed to parse Deployment: {}", e);
                        return;
                    }
                };
                if let Err(e) = crate::deploy::deploy(
                    &deployment,
                    s7_manager,
                    meta_factory,
                    fill_stations,
                    monitor_ip,
                    monitor_port,
                ).await {
                    error!("[MONITOR] deploy failed: {}", e);
                }
            }
            "pilotService/deployAll" => {
                let deployments: Deployments = match serde_json::from_str(result) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("[MONITOR] Failed to parse Deployments: {}", e);
                        return;
                    }
                };
                crate::deploy::deploy_all(
                    &deployments,
                    s7_manager,
                    meta_factory,
                    fill_stations,
                    monitor_ip,
                    monitor_port,
                ).await;
            }
            _ => {
                warn!("[MONITOR] Unknown pilotCallback: {}", callback);
            }
        }
    }

    /// Build JSON message for monitor status (matches Java Monitor.sendData → MonitorData)
    fn build_monitor_message(&self, monitor: &Monitor, data: &str) -> String {
        serde_json::json!({
            "id": monitor.id,
            "merchantId": monitor.merchant_id,
            "ip": monitor.ip,
            "port": monitor.port,
            "name": monitor.name,
            "code": monitor.code,
            "dbNumber": monitor.db_number,
            "offset": monitor.offset,
            "blockSize": monitor.block_size,
            "type": monitor.monitor_type,
            "frequency": monitor.frequency,
            "resendInterval": monitor.resend_interval,
            "forwardAddress": monitor.forward_address,
            "localAddress": monitor.local_address,
            "data": data,
        }).to_string()
    }
}
