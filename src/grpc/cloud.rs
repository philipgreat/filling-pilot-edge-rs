//! Cloud session - gRPC connection to cloud server
//! 
//! Implements:
//! - ECDSA signing (ECDSA over P-256 + SHA-256, matches Java SHA256withECDSA)
//! - register() on startup (streaming ServerCommand responses)
//! - heartBeat() every N ms
//! - handle ServerCommand messages

pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/pilot.rs"));
}

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tracing::{info, warn};
use k256::ecdsa::{SigningKey, Signature, signature::Signer};
use k256::pkcs8::DecodePrivateKey;
use sha2::{Sha256, Digest};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::context::Context;
use crate::logger::Logger;
use crate::s7::PlcConnectionStatus;

/// Cloud session - manages gRPC connection to cloud server
pub struct CloudSession {
    /// gRPC client (interior mutability for async calls)
    client: RwLock<Option<pb::filling_pilot_client::FillingPilotClient<Channel>>>,
    /// Edge node ID
    id: String,
    /// Random number generated once at startup
    random_number: i64,
    /// ECDSA signing key (Base64 PEM encoded)
    private_key_pem: String,
    /// Heartbeat interval in milliseconds
    heart_beat_ms: u64,
    /// Cloud server address
    server_address: String,
    /// Cloud server port
    port: u16,
    /// UDP logger
    udp_logger: Arc<Logger>,
}

impl CloudSession {
    /// Create a new cloud session
    pub fn new(ctx: &Context, udp_logger: Arc<Logger>) -> Self {
        let random_number = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        Self {
            client: RwLock::new(None),
            id: ctx.id.clone(),
            random_number,
            private_key_pem: ctx.private_key.clone().unwrap_or_default(),
            heart_beat_ms: ctx.heart_beat,
            server_address: ctx.server_address.clone(),
            port: ctx.port,
            udp_logger,
        }
    }

    /// Build ECDSA signature (P-256 + SHA-256, matches Java's SHA256withECDSA)
    /// 
    /// Java: SecureUtil.sign(SignAlgorithm.SHA256withECDSA, privateKey, null)
    ///       signer.signHex(ByteUtil.longToBytes(randomNumber))
    /// Output: hex string of signature bytes
    fn sign(&self, data: i64) -> String {
        if self.private_key_pem.is_empty() {
            return String::new();
        }

        // Parse PEM-encoded private key
        let signing_key = match SigningKey::from_pkcs8_pem(&self.private_key_pem) {
            Ok(k) => k,
            Err(_) => {
                // Try raw Base64-decoded bytes
                let bytes = match BASE64.decode(&self.private_key_pem) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Failed to base64-decode private key: {}", e);
                        return String::new();
                    }
                };
                match SigningKey::from_slice(&bytes) {
                    Ok(k) => k,
                    Err(e) => {
                        warn!("Failed to parse private key bytes: {}", e);
                        return String::new();
                    }
                }
            }
        };

        // Sign: SHA256(data as big-endian i64 bytes) — matches Java ByteUtil.longToBytes
        let mut hasher = Sha256::new();
        hasher.update(&data.to_be_bytes());
        let hash = hasher.finalize();

        let signature: Signature = signing_key.sign(&hash);
        
        // Hex encode signature bytes — matches Java Hex.encodeHexString(sig.toByteArray())
        hex::encode(signature.to_bytes())
    }

    /// Build ECNInfo protobuf message
    fn build_ecn_info(&self) -> pb::EcnInfo {
        pb::EcnInfo {
            id: self.id.clone(),
            random_number: self.random_number,
            sign: self.sign(self.random_number),
        }
    }

    /// Connect to the cloud server
    pub async fn connect(&self) -> Result<(), String> {
        let addr = format!("http://{}:{}", self.server_address, self.port);
        
        info!("[CLOUD] Connecting to {}:{}", self.server_address, self.port);
        self.log_udp("CLOUD", &format!("Connecting to cloud server: {}:{}", self.server_address, self.port)).await;

        match tonic::transport::Endpoint::new(addr)
            .map_err(|e| format!("Invalid endpoint: {}", e))?
            .connect()
            .await
        {
            Ok(channel) => {
                let client = pb::filling_pilot_client::FillingPilotClient::new(channel);
                *self.client.write().await = Some(client);
                info!("[CLOUD] Connected to cloud server");
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

    /// Register with cloud server — runs on startup, keeps streaming for commands
    pub async fn register(&self) {
        let ecn_info = self.build_ecn_info();
        
        self.log_udp("REGISTER", &format!("Sending register (id={}, random={})", self.id, self.random_number)).await;

        // Try to get mutable client from read lock then upgrade... 
        // For simplicity, hold the write lock during register
        let mut stream_result = {
            let mut guard = self.client.write().await;
            if let Some(ref mut client) = *guard {
                let req = tonic::Request::new(ecn_info);
                client.register(req).await
            } else {
                self.log_udp("REGISTER", "Not connected, skipping register").await;
                return;
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
                            info!("[CMD] type={}{}", cmd.r#type, plc_summary);
                            self.log_udp("CMD", &format!("type={}{}", cmd.r#type, plc_summary)).await;
                            self.handle_command(&cmd).await;
                        }
                        Err(e) => {
                            warn!("[REGISTER] stream error: {}", e);
                            self.log_udp("REGISTER", &format!("Stream error: {}", e)).await;
                            break;
                        }
                    }
                }
                self.log_udp("REGISTER", "Command stream ended").await;
            }
            Err(e) => {
                warn!("[REGISTER] failed: {}", e);
                self.log_udp("REGISTER", &format!("Register failed: {}", e)).await;
            }
        }
    }

    /// Start heartbeat loop — runs forever until shutdown
    pub async fn start_heartbeat_loop(&self) {
        info!("[HEARTBEAT] Starting loop (interval={}ms)", self.heart_beat_ms);
        self.log_udp("HEARTBEAT", &format!("Starting heartbeat loop (interval={}ms)", self.heart_beat_ms)).await;

        let mut tick = interval(Duration::from_millis(self.heart_beat_ms));
        
        loop {
            tick.tick().await;
            self.send_heartbeat().await;
        }
    }

    /// Send one heartbeat and log the result
    async fn send_heartbeat(&self) {
        let ecn_info = self.build_ecn_info();

        let mut guard = self.client.write().await;
        let client = match guard.as_mut() {
            Some(c) => c,
            None => return,
        };

        let req = tonic::Request::new(ecn_info);
        
        match client.heart_beat(req).await {
            Ok(response) => {
                let cmd = response.into_inner();
                info!("[HEARTBEAT] OK, server response: type={}", cmd.r#type);
                self.log_udp("HEARTBEAT", &format!("OK (id={}, random={})", self.id, self.random_number)).await;
                if !cmd.detail.is_empty() {
                    // Parse detail and extract only PLC info (ipAddress, portNumber)
                    let plc_summary = self.extract_plc_summary(&cmd.detail);
                    self.log_udp("HEARTBEAT", &format!("plc={}", plc_summary)).await;
                }
            }
            Err(e) => {
                warn!("[HEARTBEAT] failed: {}", e);
                self.log_udp("HEARTBEAT", &format!("FAILED: {}", e)).await;
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
    async fn handle_command(&self, cmd: &pb::ServerCommand) {
        info!("[CMD] Handling: type={}, detail={}", cmd.r#type, cmd.detail);
        
        match cmd.r#type.as_str() {
            "upgrade" => {
                self.log_udp("CMD", "Received upgrade command (not implemented yet)").await;
            }
            "restart" => {
                self.log_udp("CMD", "Received restart command (not implemented yet)").await;
            }
            "config" => {
                self.log_udp("CMD", "Received config command (not implemented yet)").await;
            }
            _ if cmd.r#type.is_empty() => {
                // Heartbeat response with no command
            }
            _ => {
                warn!("[CMD] Unknown command type: {}", cmd.r#type);
                self.log_udp("CMD", &format!("Unknown command type: {}", cmd.r#type)).await;
            }
        }
    }

    /// Send PLC response to cloud server via gRPC send() method
    /// 
    /// This is called when PLC status changes or data needs to be reported.
    /// Uses bidirectional streaming to send PlcResponse messages.
    pub async fn send_plc_response(&self, plc_id: &str, msg_type: &str, message: &str) {
        let plc_response = pb::PlcResponse {
            id: self.id.clone(),
            plc_id: plc_id.to_string(),
            r#type: msg_type.to_string(),
            random_number: self.random_number,
            sign: self.sign(self.random_number),
            message: message.to_string(),
        };

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
                info!("[SEND] PLC response sent: type={}, plcId={}", msg_type, plc_id);
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
        let msg_type = if status.connected { "plc_connected" } else { "plc_disconnected" };
        let message = serde_json::to_string(status).unwrap_or_default();
        self.send_plc_response(&format!("{}:{}", status.host, status.port), msg_type, &message).await;
    }

    /// Fire-and-forget UDP log via the stored logger
    async fn log_udp(&self, name: &str, msg: &str) {
        let l = Arc::clone(&self.udp_logger);
        let name = name.to_string();
        let msg = msg.to_string();
        tokio::spawn(async move { l.log(&name, &msg).await; });
    }
}