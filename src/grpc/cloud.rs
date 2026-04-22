//! Cloud session - gRPC connection to cloud server
//! 
//! Implements:
//! - ECDSA signing (direct signing with p256, matches Java's SHA256withECDSA)
//!   NOTE: Java's SecureUtil.sign() internally hashes with SHA-256, so we pass raw bytes.
//!         p256's Signer trait also hashes internally with SHA-256 automatically.
//! - register() on startup (streaming ServerCommand responses)
//! - heartBeat() every N ms
//! - handle ServerCommand messages

pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/pilot.rs"));
}

use std::sync::Arc;
use tokio::sync::RwLock;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio::time::{interval, Duration};
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tracing::{info, warn};
use p256::ecdsa::{SigningKey, Signature, signature::Signer};
use p256::pkcs8::DecodePrivateKey;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::context::Context;
use crate::logger::Logger;
use crate::s7::{PlcConnectionStatus, PlcConfig};
use crate::report::{ReportCache, FillStation, Monitor, MonitorProcessor};
use crate::deploy::PLCSerializeMetaFactory;

/// Cloud session - manages gRPC connection to cloud server
pub struct CloudSession {
    /// gRPC client (interior mutability for async calls)
    client: RwLock<Option<pb::filling_pilot_client::FillingPilotClient<Channel>>>,
    /// Edge node ID
    id: String,
    /// Random number for ECN authentication (atomic for dynamic update from reConnect)
    random_number: AtomicI64,
    /// ECDSA signing key (Base64 PEM encoded)
    private_key_pem: String,
    /// Heartbeat interval in milliseconds (atomic for dynamic update from server config)
    heart_beat_ms: AtomicU64,
    /// Cloud server address
    server_address: String,
    /// Cloud server port
    port: u16,
    /// UDP logger
    udp_logger: Arc<Logger>,
    /// S7 Manager reference for updating PLC list
    s7_manager: std::sync::Arc<crate::s7::S7Manager>,
    /// Last PLC info detail (skip redundant TCP tests when unchanged)
    last_plc_detail: std::sync::Mutex<String>,
    /// Report cache (for report/status reading and sending)
    report_cache: Arc<ReportCache>,
    /// Monitor processor (for monitor reading and sending)
    monitor_processor: Arc<MonitorProcessor>,
    /// Metadata factory for codec decoding and deploy
    meta_factory: Arc<PLCSerializeMetaFactory>,
    /// Report interval in ms (atomic for dynamic update)
    report_interval_ms: AtomicU64,
    /// Status interval in ms (atomic for dynamic update)
    status_interval_ms: AtomicU64,
}

impl CloudSession {
    /// Create a new cloud session
    pub fn new(
        ctx: &Context,
        udp_logger: Arc<Logger>,
        s7_manager: std::sync::Arc<crate::s7::S7Manager>,
        meta_factory: Arc<PLCSerializeMetaFactory>,
    ) -> Self {
        let random_number_init = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        Self {
            client: RwLock::new(None),
            id: ctx.id.clone(),
            random_number: AtomicI64::new(random_number_init),
            private_key_pem: ctx.private_key.clone().unwrap_or_default(),
            heart_beat_ms: AtomicU64::new(ctx.heart_beat),
            server_address: ctx.server_address.clone(),
            port: ctx.port,
            udp_logger,
            s7_manager: s7_manager.clone(),
            last_plc_detail: std::sync::Mutex::new(String::new()),
            report_cache: Arc::new(ReportCache::new(s7_manager.clone())),
            monitor_processor: Arc::new(MonitorProcessor::new(s7_manager, meta_factory.clone())),
            meta_factory,
            report_interval_ms: AtomicU64::new(ctx.report_interval),
            status_interval_ms: AtomicU64::new(ctx.status_interval),
        }
    }

    /// Build ECDSA signature (P-256 + SHA-256, matches Java's SHA256withECDSA)
    /// 
    /// Java: SecureUtil.sign(SignAlgorithm.SHA256withECDSA, privateKey, null)
    ///       signer.signHex(ByteUtil.longToBytes(randomNumber))
    /// Output: hex string of signature bytes
    fn sign(&self, data: i64) -> String {
        if self.private_key_pem.is_empty() {
            println!("[DEBUG_SIGN] private_key is EMPTY");
            return String::new();
        }

        // privateKey in id file is Base64-encoded PKCS#8 DER (not PEM)
        let signing_key = match BASE64.decode(&self.private_key_pem) {
            Ok(bytes) => {
                println!("[DEBUG_SIGN] base64 decode OK, bytes_len={}", bytes.len());
                match SigningKey::from_pkcs8_der(&bytes) {
                    Ok(k) => {
                        println!("[DEBUG_SIGN] from_pkcs8_der OK!");
                        k
                    },
                    Err(e) => {
                        println!("[DEBUG_SIGN] from_pkcs8_der FAILED: {}, trying from_pkcs8_pem...", e);
                        match SigningKey::from_pkcs8_pem(&self.private_key_pem) {
                            Ok(k) => k,
                            Err(e2) => {
                                warn!("[SIGN] Failed to parse private key: pkcs8_der={}, pkcs8_pem={}", e, e2);
                                return String::new();
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!("[DEBUG_SIGN] base64 decode FAILED: {}, trying from_pkcs8_pem...", e);
                match SigningKey::from_pkcs8_pem(&self.private_key_pem) {
                    Ok(k) => k,
                    Err(e2) => {
                        warn!("[SIGN] Failed to decode/parse private key: base64={}, pem={}", e, e2);
                        return String::new();
                    }
                }
            }
        };

        // Sign raw bytes directly - matches Java's SHA256withECDSA behavior
        // Java: SecureUtil.sign() internally hashes with SHA-256, so we pass raw bytes
        // Rust k256::ecdsa::Signer trait also hashes internally with SHA-256
        let signature: Signature = signing_key.sign(&data.to_be_bytes());
        
        // Hex encode signature bytes - matches Java Hex.encodeHexString(sig.toByteArray())
        hex::encode(signature.to_bytes())
    }

    /// Build ECNInfo protobuf message
    fn build_ecn_info(&self) -> pb::EcnInfo {
        let random_number = self.random_number.load(Ordering::Relaxed);
        let sign = self.sign(random_number);
        pb::EcnInfo {
            id: self.id.clone(),
            random_number,
            sign,
        }
    }

    /// Check if gRPC client is connected
    pub async fn is_connected(&self) -> bool {
        self.client.read().await.is_some()
    }

    /// Connect to the cloud server
    pub async fn connect(&self) -> Result<(), String> {
        let addr = format!("http://{}:{}", self.server_address, self.port);
        
        self.log_udp("CLOUD", &format!("Connecting to cloud server: {}:{}", self.server_address, self.port)).await;

        match tonic::transport::Endpoint::new(addr)
            .map_err(|e| format!("Invalid endpoint: {}", e))?
            .connect()
            .await
        {
            Ok(channel) => {
                let client = pb::filling_pilot_client::FillingPilotClient::new(channel);
                *self.client.write().await = Some(client);
                self.log_udp("CLOUD", "Connected to cloud server").await;
                Ok(())
            }
            Err(e) => {
                let msg = format!("Failed to connect: {}", e);
                warn!("[CLOUD] {}", msg);
                self.log_udp("CLOUD", &msg).await;
                Err(msg)
            }
        }
    }

    /// Register with cloud server - runs on startup, keeps streaming for commands
    pub async fn register(&self) {
        // Loop: re-register when server sends reConnect (matches Java behavior)
        loop {
            let result = self.register_once().await;
            if result {
                // ReConnect received, loop again with updated random
                continue;
            }
            // Stream ended or error, stop
            break;
        }
    }

    /// Single register attempt (returns true if reConnect was received)
    async fn register_once(&self) -> bool {
        let ecn_info = self.build_ecn_info();
        
        self.log_udp("REGISTER", &format!("Sending register (id={}, random={})", self.id, self.random_number.load(Ordering::Relaxed))).await;

        // Try to get mutable client from read lock then upgrade... 
        // For simplicity, hold the write lock during register
        let mut stream_result = {
            let mut guard = self.client.write().await;
            if let Some(ref mut client) = *guard {
                let req = tonic::Request::new(ecn_info);
                client.register(req).await
            } else {
                self.log_udp("REGISTER", "Not connected, skipping register").await;
                return false;
            }
        };

        match stream_result {
            Ok(response) => {
                self.log_udp("REGISTER", "Register successful, waiting for server commands...").await;
                
                let mut stream = response.into_inner();
                while let Some(cmd) = stream.next().await {
                    match cmd {
                        Ok(cmd) => {
                            // Log command type and PLC summary only
                            let plc_summary = if !cmd.detail.is_empty() {
                                format!(" plc={}", self.extract_plc_summary(&cmd.detail))
                            } else {
                                String::new()
                            };
                            self.log_udp("CMD", &format!("type={}{}", cmd.r#type, plc_summary)).await;
                            if self.handle_command(&cmd).await { return true; }
                        }
                        Err(e) => {
                            warn!("[REGISTER] stream error: {}", e);
                            self.log_udp("REGISTER", &format!("Stream error: {}", e)).await;
                            return false;
                        }
                    }
                }
                self.log_udp("REGISTER", "Command stream ended").await;
                return false;
            }
            Err(e) => {
                warn!("[REGISTER] failed: {}", e);
                self.log_udp("REGISTER", &format!("Register failed: {}", e)).await;
                return false;
            }
        }
    }

        /// Update heartbeat interval (called when server sends new config)
    pub fn update_heartbeat(&self, ms: u64) {
        let old = self.heart_beat_ms.load(Ordering::Relaxed);
        if old != ms {
            self.heart_beat_ms.store(ms, Ordering::Relaxed);
            info!("[HEARTBEAT] Interval updated: {}ms -> {}ms", old, ms);
        }
    }

    /// Start heartbeat loop - runs forever until shutdown (matches Java keepHeartBeat)
    pub async fn start_heartbeat_loop(&self) {
        let mut hb_ms = self.heart_beat_ms.load(Ordering::Relaxed);
        self.log_udp("HEARTBEAT", &format!("Starting heartbeat loop (interval={}ms)", hb_ms)).await;

        let mut tick = interval(Duration::from_millis(hb_ms));
        
        loop {
            tick.tick().await;
            
            // Check if heartbeat interval was updated dynamically
            let current_ms = self.heart_beat_ms.load(Ordering::Relaxed);
            if current_ms != hb_ms {
                info!("[HEARTBEAT] Interval changed: {}ms -> {}ms, resetting ticker", hb_ms, current_ms);
                tick = interval(Duration::from_millis(current_ms));
                hb_ms = current_ms;
            }
            
            // Java keepHeartBeat: sendHeartBeat() every tick, errors don't break the loop
            self.send_heartbeat().await;
        }
    }

    /// Send one heartbeat and log the result (matches Java sendHeartBeat)
    async fn send_heartbeat(&self) {
        let ecn_info = self.build_ecn_info();

        // Send heartbeat and get response (hold lock only during gRPC call)
        let result = {
            let mut guard = self.client.write().await;
            let client = match guard.as_mut() {
                Some(c) => c,
                None => {
                    self.log_udp("HEARTBEAT", "SKIP (client not connected)").await;
                    return;
                }
            };

            let req = tonic::Request::new(ecn_info);
            // Log sending heartbeat
            self.log_udp("HB_SEND", &format!(
                "id={}, random={}, server={}:{}",
                self.id, self.random_number.load(Ordering::Relaxed), self.server_address, self.port
            )).await;
            
            // Send heartbeat and collect response (lock released after this block)
            client.heart_beat(req).await.map(|r| r.into_inner())
        };  // Lock released here!

        match result {
            Ok(cmd) => {
                // Log received heartbeat response (no lock held)
                self.log_udp("HB_RECV", &format!(
                    "OK id={}, random={}, resp_type={}, detail_len={}",
                    self.id, self.random_number.load(Ordering::Relaxed), cmd.r#type, cmd.detail.len()
                )).await;
                
                // Process PLC info summary from detail
                if !cmd.detail.is_empty() {
                    let plc_summary = self.extract_plc_summary(&cmd.detail);
                    self.log_udp("HB_RECV", &format!("detail={}", plc_summary)).await;
                }
                
                // Process any commands (safe now — lock is released)
                if !cmd.r#type.is_empty() {
                    let _reconnect = self.handle_command(&cmd).await;
                }
            }
            Err(e) => {
                warn!("[HEARTBEAT] failed: {}", e);
                self.log_udp("HB_RECV", &format!("FAILED: {}", e)).await;
            }
        }
    }

    /// Extract PLC summary (ipAddress:portNumber) from detail JSON
    /// Returns a compact string like "192.168.1.10:102, 192.168.1.11:102"
    fn extract_plc_summary(&self, detail: &str) -> String {
        // Try to parse as JSON and extract PLC info
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(detail) {
            let mut plcs = Vec::new();
            
            // Try array format: [{"ipAddress":"...", "portNumber":...}, ...]
            if let Some(arr) = json.as_array() {
                for item in arr {
                    if let (Some(ip), Some(port)) = (
                        item.get("ipAddress").and_then(|v| v.as_str()),
                        item.get("portNumber").and_then(|v| v.as_u64())
                    ) {
                        plcs.push(format!("{}:{}", ip, port));
                    }
                }
            }
            
            // Try dataMeta format: {"dataMeta":[{"ipAddress":"...", ...}]}
            if plcs.is_empty() {
                if let Some(data_meta) = json.get("dataMeta").and_then(|v| v.as_array()) {
                    for item in data_meta {
                        if let (Some(ip), Some(port)) = (
                            item.get("ipAddress").and_then(|v| v.as_str()),
                            item.get("portNumber").and_then(|v| v.as_u64())
                        ) {
                            plcs.push(format!("{}:{}", ip, port));
                        }
                    }
                }
            }
            
            if !plcs.is_empty() {
                return plcs.join(", ");
            }
        }
        
        // Fallback: return first 50 chars if can't parse
        if detail.len() > 50 {
            format!("{}...", &detail[..50])
        } else {
            detail.to_string()
        }
    }

    /// Handle a server command
    async fn handle_command(&self, cmd: &pb::ServerCommand) -> bool {

        
        match cmd.r#type.as_str() {
            "plcInfo" => {
                // Java PlcInfo.handle: always process and send plcInfo responses every heartbeat.
                // Server marks PLC online with 60s TTL, so we must respond each time.
                // Internal state (S7Manager, monitors, report cache) only updates when config changes.
                let config_changed = {
                    let last = self.last_plc_detail.lock().unwrap();
                    *last != cmd.detail
                };
                
                // Parse PLC list from detail (always needed for TCP test + plcInfo response)
                let plcs = self.parse_plc_list(&cmd.detail).await;
                
                if plcs.is_empty() {
                    self.log_udp("PLC_INFO", "No PLCs in config").await;
                    return false;
                }
                
                // Update internal state only when config changes
                if config_changed {
                    self.log_udp("PLC_INFO", "Config changed, updating internal state...").await;
                    {
                        let mut last = self.last_plc_detail.lock().unwrap();
                        *last = cmd.detail.clone();
                    }
                    
                    // Update S7Manager with new PLC list
                    self.s7_manager.update_plc_list(plcs.clone()).await;
                    
                    // Parse fill stations and update report cache (like Java ReportCache.saveReportLocation)
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&cmd.detail) {
                        if let Some(arr) = json.as_array() {
                            let stations = FillStation::parse_from_plc_info(arr);
                            if !stations.is_empty() {
                                // Clone stations for update_fill_stations since it consumes the vec
                                let stations_clone = stations.clone();
                                self.report_cache.update_fill_stations(stations_clone).await;
                            }
                            // Parse monitors and update monitor processor (like Java MonitorProcessor.saveMonitors)
                            let monitors = Monitor::parse_from_plc_info(arr);
                            if !monitors.is_empty() {
                                self.monitor_processor.update_monitors(monitors).await;
                            }
                            // Save DataBlockDefinition metadata (like Java PLCSerializeMetaFactory.saveMeta)
                            self.monitor_processor.save_meta(arr).await;
                            // Update fill stations map for deploy callbacks
                            let fs_list: Vec<(String, String, u16)> = stations.iter()
                                .map(|s| (s.id.clone(), s.ip.clone(), s.port))
                                .collect();
                            self.monitor_processor.update_fill_stations(fs_list).await;
                        }
                    }
                }
                
                // Test each PLC with TCP socket (like Java PlcInfo.handle) — EVERY heartbeat
                let mut count = 0usize;
                for plc in &plcs {
                    let result = self.test_tcp_connection(&plc.ip, plc.port).await;
                    let msg_type = if result { "plcInfo" } else { "plcInfoFail" };
                    let message = if result { "ok" } else { "failed" };
                    if result { count += 1; }
                    
                    // Send response for each PLC: sendPlcResponse(id, "plcInfo", "ok")
                    // Java: String id = (String) plc.get("id"); session.sendPlcResponse(id, ...)
                    self.send_plc_response(&plc.id, msg_type, message).await;
                    
                    // Sleep 50ms between each PLC (like Java)
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
                
                // Send final response: sendPlcResponse("plcInfo", "ok")
                self.send_plc_response("plcInfo", "plcInfo", "ok").await;
                
                self.log_udp("PLC_INFO", &format!("Tested {} PLCs, {} online{}", plcs.len(), count, if config_changed { " (config changed)" } else { "" })).await;
            }
            "reConnect" | "reconnect" => {
                // Java Reconnect.handle: update randomNumber then re-register
                // Keep client alive so heartbeat continues working during re-registration
                self.log_udp("RECONNECT", &format!("Received reconnect, updating random from {} to {}", 
                    self.random_number.load(Ordering::Relaxed), cmd.detail)).await;
                
                // Parse new random number from detail
                if let Ok(new_random) = cmd.detail.parse::<i64>() {
                    self.random_number.store(new_random, Ordering::Relaxed);
                } else {
                    // If detail is not a number, generate a new one
                    let new_random = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(self.random_number.load(Ordering::Relaxed) + 1);
                    self.random_number.store(new_random, Ordering::Relaxed);
                }
                
                // Break out of register stream to trigger re-register
                // (the caller will re-invoke register() in a loop)
                self.log_udp("RECONNECT", &format!("Re-registering with random={}", self.random_number.load(Ordering::Relaxed))).await;
                return true;  // Signal caller to re-register
            }
            "upgrade" => {
                self.log_udp("CMD", "Received upgrade command (not implemented yet)").await;
            }
            "restart" => {
                self.log_udp("CMD", "Received restart command (not implemented yet)").await;
            }
            "read" => {
                // Java Read.handle: read from PLC, send response
                self.log_udp("READ", &format!("detail_len={}", cmd.detail.len())).await;
                match serde_json::from_str::<crate::s7::ReadRequest>(&cmd.detail) {
                    Ok(read_req) => {
                        let plc_id = read_req.plc_id.clone();
                        match self.s7_manager.read_bytes(&read_req.ip, read_req.port, read_req.db_index, read_req.offset, read_req.size).await {
                            Ok(data) => {
                                let resp = crate::s7::ReadResponse {
                                    request_id: read_req.request_id,
                                    plc_id: read_req.plc_id.clone(),
                                    ip: read_req.ip,
                                    port: read_req.port,
                                    db_index: read_req.db_index,
                                    offset: read_req.offset,
                                    success: true,
                                    message: "OK".to_string(),
                                    hex_content: Some(hex::encode(&data)),
                                };
                                let msg = serde_json::to_string(&resp).unwrap_or_default();
                                self.send_plc_response(&plc_id, "read", &msg).await;
                            }
                            Err(e) => {
                                let resp = crate::s7::ReadResponse {
                                    request_id: read_req.request_id,
                                    plc_id: read_req.plc_id.clone(),
                                    ip: read_req.ip,
                                    port: read_req.port,
                                    db_index: read_req.db_index,
                                    offset: read_req.offset,
                                    success: false,
                                    message: e.to_string(),
                                    hex_content: None,
                                };
                                let msg = serde_json::to_string(&resp).unwrap_or_default();
                                self.send_plc_response(&plc_id, "read", &msg).await;
                            }
                        }
                    }
                    Err(e) => {
                        self.log_udp("READ", &format!("Failed to parse read request: {}", e)).await;
                    }
                }
            }
            "write" => {
                // Java Write.handle: write to PLC, send response
                self.log_udp("WRITE", &format!("detail_len={}", cmd.detail.len())).await;
                match serde_json::from_str::<crate::s7::WriteRequest>(&cmd.detail) {
                    Ok(write_req) => {
                        let plc_id = write_req.plc_id.clone();
                        let hex_content = write_req.hex_content.clone();
                        match hex::decode(&hex_content) {
                            Ok(data) => {
                                match self.s7_manager.write_bytes(&write_req.ip, write_req.port, write_req.db_index, write_req.offset, &data).await {
                                    Ok(()) => {
                                        let resp = crate::s7::WriteResponse {
                                            request_id: write_req.request_id,
                                            plc_id: write_req.plc_id.clone(),
                                            ip: write_req.ip,
                                            port: write_req.port,
                                            db_index: write_req.db_index,
                                            offset: write_req.offset,
                                            success: true,
                                            message: "OK".to_string(),
                                        };
                                        let msg = serde_json::to_string(&resp).unwrap_or_default();
                                        self.send_plc_response(&plc_id, "write", &msg).await;
                                    }
                                    Err(e) => {
                                        let resp = crate::s7::WriteResponse {
                                            request_id: write_req.request_id,
                                            plc_id: write_req.plc_id.clone(),
                                            ip: write_req.ip,
                                            port: write_req.port,
                                            db_index: write_req.db_index,
                                            offset: write_req.offset,
                                            success: false,
                                            message: e.to_string(),
                                        };
                                        let msg = serde_json::to_string(&resp).unwrap_or_default();
                                        self.send_plc_response(&plc_id, "write", &msg).await;
                                    }
                                }
                            }
                            Err(e) => {
                                self.log_udp("WRITE", &format!("Invalid hex content: {}", e)).await;
                            }
                        }
                    }
                    Err(e) => {
                        self.log_udp("WRITE", &format!("Failed to parse write request: {}", e)).await;
                    }
                }
            }
            "status" => {
                // Java Status.handle: return all cached status
                let statuses = self.report_cache.get_all_statuses().await;
                let msg = serde_json::to_string(&statuses).unwrap_or_default();
                self.send_plc_response("status", "status", &msg).await;
            }
            "ReportSubmitted" | "reportSubmitted" => {
                // Java ReportSubmitted.handle: mark report as sent
                if let Ok(report_id) = cmd.detail.parse::<i64>() {
                    self.report_cache.delete_report(report_id).await;
                    self.log_udp("REPORT_SUBMITTED", &format!("Deleted report {}", report_id)).await;
                }
            }
            "clientInfo" => {
                // Java VersionReport.handle: send version info
                self.send_plc_response("", "clientInfo", &format!("clientVersion: {}", self.id)).await;
            }
            "config" => {
                self.log_udp("CMD", "Received config command").await;
                // Try to parse detail as ServerConf JSON
                if !cmd.detail.is_empty() {
                    if let Ok(server_conf) = serde_json::from_str::<crate::context::ServerConf>(&cmd.detail) {
                        self.log_udp("CONFIG", &format!(
                            "heartBeat={}, reportInterval={}, statusInterval={}",
                            server_conf.heart_beat, server_conf.report_interval, server_conf.status_interval
                        )).await;
                        // Update heartbeat interval dynamically
                        if server_conf.heart_beat > 0 {
                            self.update_heartbeat(server_conf.heart_beat);
                        }
                        // Update report/status intervals dynamically
                        if server_conf.report_interval > 0 {
                            self.report_interval_ms.store(server_conf.report_interval, Ordering::Relaxed);
                        }
                        if server_conf.status_interval > 0 {
                            self.status_interval_ms.store(server_conf.status_interval, Ordering::Relaxed);
                        }
                    } else {
                        self.log_udp("CONFIG", &format!("Failed to parse config detail: {}", &cmd.detail[..cmd.detail.len().min(100)])).await;
                    }
                }
            }
            _ if cmd.r#type.is_empty() => {
                // Heartbeat response with no command
            }
            _ => {
                warn!("[CMD] Unknown command type: {}", cmd.r#type);
                self.log_udp("CMD", &format!("Unknown command type: {}", cmd.r#type)).await;
            }
        }
        false
    }

    /// Parse PLC list from JSON detail
    async fn parse_plc_list(&self, detail: &str) -> Vec<PlcConfig> {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(detail) {
            if let Some(arr) = json.as_array() {
                let mut plcs = Vec::new();
                for item in arr {
                    if let (Some(id), Some(ip), Some(port)) = (
                        item.get("id").and_then(|v| v.as_str()),
                        item.get("ipAddress").and_then(|v| v.as_str()),
                        item.get("portNumber").and_then(|v| v.as_u64())
                    ) {
                        plcs.push(PlcConfig {
                            id: id.to_string(),
                            ip: ip.to_string(),
                            port: port as u16,
                        });
                    }
                }
                return plcs;
            }
        }
        Vec::new()
    }

    /// Test TCP connection to PLC (like Java Socket.connect with 3s timeout)
    async fn test_tcp_connection(&self, host: &str, port: u16) -> bool {
        let addr = format!("{}:{}", host, port);
        let timeout = tokio::time::Duration::from_secs(3);
        
        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
            Ok(Ok(_stream)) => {

                true
            }
            Ok(Err(e)) => {
                warn!("[TCP_TEST] {}:{} failed: {}", host, port, e);
                self.log_udp("TCP_TEST", &format!("connect to {}:{} failed: {}", host, port, e)).await;
                false
            }
            Err(_) => {
                warn!("[TCP_TEST] {}:{} timeout (3s)", host, port);
                self.log_udp("TCP_TEST", &format!("connect to {}:{} timeout", host, port)).await;
                false
            }
        }
    }

    /// Send PLC response to cloud server via gRPC send() method
    /// 
    /// This is called when PLC status changes or data needs to be reported.
    /// Uses bidirectional streaming to send PlcResponse messages.
    pub async fn send_plc_response(&self, plc_id: &str, msg_type: &str, message: &str) {
        let random_number = self.random_number.load(Ordering::Relaxed);
        let sign = self.sign(random_number);

        let plc_response = pb::PlcResponse {
            id: self.id.clone(),
            plc_id: plc_id.to_string(),
            r#type: msg_type.to_string(),
            random_number,
            sign: sign.clone(),
            message: message.to_string(),
        };

        // Log full PlcResponse content before sending
        self.log_udp("SEND_DETAIL", &format!(
            "PlcResponse {{ id: {}, plc_id: {}, type: {}, random_number: {}, sign: {}, message: {} }}",
            self.id, plc_id, msg_type, random_number, sign, message
        )).await;

        let mut guard = self.client.write().await;
        let client = match guard.as_mut() {
            Some(c) => c,
            None => {
                warn!("[SEND] Not connected to cloud server");
                return;
            }
        };

        // Create a stream with single message
        let stream = tokio_stream::once(plc_response);
        let req = tonic::Request::new(stream);

        match client.send(req).await {
            Ok(_) => {
                self.log_udp("SEND", &format!("sent {} for {}", msg_type, plc_id)).await;
            }
            Err(e) => {
                warn!("[SEND] Failed to send: {}", e);
                self.log_udp("SEND", &format!("FAILED: {}", e)).await;
            }
        }
    }

    /// Send PLC connection status to cloud server
    pub async fn send_plc_status(&self, status: &PlcConnectionStatus) {
        let msg_type = if status.connected { "plcInfo" } else { "plcInfoFail" };
        let message = serde_json::to_string(status).unwrap_or_default();
        self.send_plc_response(&format!("{}:{}", status.host, status.port), msg_type, &message).await;
    }

    /// Fire-and-forget UDP log via the stored logger
    async fn log_udp(&self, name: &str, msg: &str) {
        // Local log (always)
        info!("[{}] {}", name, msg);
        // UDP log (async, non-blocking)
        let l = Arc::clone(&self.udp_logger);
        let name = name.to_string();
        let msg = msg.to_string();
        tokio::spawn(async move { l.log(&name, &msg).await; });
    }

    /// Get monitor processor (for main.rs to start persistence loop)
    pub fn get_monitor_processor(&self) -> Arc<MonitorProcessor> {
        Arc::clone(&self.monitor_processor)
    }

    // ===== Three background loops (matching Java Session.start()) =====

    /// Report loop - reads PLC reports and sends to cloud (like Java keepReadReport)
    /// Runs every reportInterval ms (default 5000ms)
    pub async fn start_report_loop(self: Arc<Self>) {
        let mut interval_ms = self.report_interval_ms.load(Ordering::Relaxed);
        self.log_udp("REPORT", &format!("Starting report loop (interval={}ms)", interval_ms)).await;
        let mut tick = interval(Duration::from_millis(interval_ms));

        loop {
            tick.tick().await;

            // Check for interval update
            let current_ms = self.report_interval_ms.load(Ordering::Relaxed);
            if current_ms != interval_ms {
                interval_ms = current_ms;
                tick = interval(Duration::from_millis(interval_ms));
                self.log_udp("REPORT", &format!("Interval updated to {}ms", interval_ms)).await;
            }

            // Read and send reports
            let unsent = {
                // Read from PLCs first
                self.report_cache.read_and_send_reports_direct().await
            };
            
            // Send unsent reports to cloud
            for (msg_type, msg) in unsent {
                self.send_plc_response("", &msg_type, &msg).await;
            }
        }
    }

    /// Status loop - reads PLC status DBs and sends to cloud (like Java keepReadStatus)
    /// Runs every statusInterval ms (default 1000ms)
    pub async fn start_status_loop(self: Arc<Self>) {
        let mut interval_ms = self.status_interval_ms.load(Ordering::Relaxed);
        self.log_udp("STATUS", &format!("Starting status loop (interval={}ms)", interval_ms)).await;
        let mut tick = interval(Duration::from_millis(interval_ms));

        loop {
            tick.tick().await;

            // Check for interval update
            let current_ms = self.status_interval_ms.load(Ordering::Relaxed);
            if current_ms != interval_ms {
                interval_ms = current_ms;
                tick = interval(Duration::from_millis(interval_ms));
                self.log_udp("STATUS", &format!("Interval updated to {}ms", interval_ms)).await;
            }

            // Read status and send
            let status_messages = self.report_cache.read_and_send_status_direct().await;
            for (msg_type, msg) in status_messages {
                self.send_plc_response("", &msg_type, &msg).await;
            }
        }
    }

    /// Monitor loop - triggers monitor reads (like Java keepTriggerMonitor)
    /// Runs every 10ms (matches Java Thread.sleep(10))
    pub async fn start_monitor_loop(self: Arc<Self>) {
        self.log_udp("MONITOR", "Starting monitor loop (interval=10ms)").await;
        let mut tick = interval(Duration::from_millis(10));

        loop {
            tick.tick().await;

            // Trigger monitors and collect messages to send
            let monitor_messages = self.monitor_processor.trigger_monitors_direct().await;
            for (msg_type, msg) in monitor_messages {
                self.send_plc_response("", &msg_type, &msg).await;
            }
        }
    }
}