//! gRPC types and cloud session for Filling Pilot

pub mod cloud;

// Re-export the generated protobuf types
pub use cloud::pb::{EcnInfo, PlcResponse, ServerCommand};
pub use cloud::CloudSession;

// Legacy serde-based types (kept for HTTP API compatibility)
use serde::{Deserialize, Serialize};

/// ECN Info - Edge Node registration info (JSON for HTTP API)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EcnInfoJson {
    pub id: String,
    pub random_number: i64,
    pub sign: String,
}

/// PLC Response - Response from PLC read/write operations (JSON for HTTP API)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlcResponseJson {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(rename = "plcId")]
    pub plc_id: String,
    pub random_number: i64,
    pub sign: String,
    pub message: String,
}

/// Server Command - Command from cloud server (JSON for HTTP API)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCommandJson {
    #[serde(rename = "type")]
    pub cmd_type: String,
    pub detail: String,
}

impl PlcResponseJson {
    /// Create success response
    pub fn success(plc_id: &str, msg_type: &str, message: &str) -> Self {
        Self {
            id: String::new(),
            msg_type: msg_type.to_string(),
            plc_id: plc_id.to_string(),
            random_number: 0,
            sign: String::new(),
            message: message.to_string(),
        }
    }

    /// Create error response
    pub fn error(plc_id: &str, msg_type: &str, error: &str) -> Self {
        Self {
            id: String::new(),
            msg_type: msg_type.to_string(),
            plc_id: plc_id.to_string(),
            random_number: 0,
            sign: String::new(),
            message: error.to_string(),
        }
    }

    /// Convert to JSON string
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl ServerCommandJson {
    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}
