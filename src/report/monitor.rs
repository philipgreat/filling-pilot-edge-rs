//! Monitor - matches Java Monitor.java + MonitorProcessor.java
//! 
//! Reads PLC data at configured frequency and sends to cloud/local endpoint.
//! Only sends when data changes or resendInterval has elapsed.

use std::collections::{HashSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::s7::S7Manager;
use super::fill_station::FillStation;

/// Monitor configuration (parsed from plcInfo JSON)
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
    /// Active monitors (config)
    monitors: Arc<RwLock<HashSet<Monitor>>>,
    /// Per-monitor runtime state
    states: Arc<RwLock<std::collections::HashMap<String, MonitorState>>>,
    /// S7 manager for PLC reads
    s7_manager: Arc<S7Manager>,
}

impl MonitorProcessor {
    pub fn new(s7_manager: Arc<S7Manager>) -> Self {
        Self {
            monitors: Arc::new(RwLock::new(HashSet::new())),
            states: Arc::new(RwLock::new(std::collections::HashMap::new())),
            s7_manager,
        }
    }

    /// Update monitors from plcInfo (like Java MonitorProcessor.saveMonitors)
    pub async fn update_monitors(&self, new_monitors: Vec<Monitor>) {
        let new_set: HashSet<Monitor> = new_monitors.into_iter().collect();
        let mut monitors = self.monitors.write().await;
        
        // Remove monitors no longer in config
        let to_remove: Vec<Monitor> = monitors.difference(&new_set).cloned().collect();
        for m in to_remove {
            monitors.remove(&m);
            let mut states = self.states.write().await;
            states.remove(&m.id);
        }
        
        // Add new monitors
        let to_add: Vec<Monitor> = new_set.difference(&monitors).cloned().collect();
        for m in to_add {
            info!("[MONITOR] Added: {} ({}:{}, DB{}, {}ms)", m.id, m.ip, m.port, m.db_number, m.frequency);
            monitors.insert(m);
        }
    }

    /// Trigger all monitors that are due (like Java MonitorProcessor.saveAndTrySendMonitorStatus)
    /// Returns list of (msg_type, message) pairs to send
    /// Called every 10ms in a loop (matches Java Thread.sleep(10))
    pub async fn trigger_monitors_direct(&self) -> Vec<(String, String)> {
        let monitors = self.monitors.read().await;
        let now = crate::report::cache::now_millis();
        let mut messages = Vec::new();
        
        for monitor in monitors.iter() {
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
                
                // Read from PLC
                let read_size = monitor.block_size.saturating_sub(monitor.offset);
                match self.s7_manager.read_bytes(&monitor.ip, monitor.port, monitor.db_number, monitor.offset as u16, read_size as u16).await {
                    Ok(data) => {
                        state.last_read_time = Some(now);
                        
                        // Build full block: offset bytes of 0 + actual data
                        let mut full = vec![0u8; monitor.block_size as usize];
                        let offset = monitor.offset as usize;
                        let data_len = data.len().min(full.len().saturating_sub(offset));
                        full[offset..offset + data_len].copy_from_slice(&data[..data_len]);
                        let new_data = hex::encode(&full);
                        
                        if new_data != old_data {
                            state.data = new_data;
                            state.last_send_time = Some(now);
                            state.running = false;
                            
                            // Data changed, send
                            let msg = self.build_monitor_message(monitor, &state.data);
                            messages.push(("monitorStatus".to_string(), msg));
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
                None => true,
                Some(last) => now - last >= monitor.resend_interval,
            };
            
            if should_resend && !state.data.is_empty() {
                state.last_send_time = Some(now);
                let msg = self.build_monitor_message(monitor, &state.data);
                messages.push(("monitorStatus".to_string(), msg));
            }
        }
        
        messages
    }

    /// Build JSON message for monitor status (matches Java Monitor.sendData)
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
