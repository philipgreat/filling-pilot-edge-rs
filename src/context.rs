//! Application context configuration
//! 
//! Reads configuration from files: `id` and `serverConf`

use serde::{Deserialize, Serialize};
use std::fs;
use thiserror::Error;

/// Application context loaded from configuration files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    /// Server address (gRPC server)
    #[serde(default = "default_server_address")]
    pub server_address: String,

    /// Server gRPC port
    #[serde(default = "default_port")]
    pub port: u16,

    /// Upgrade URL for firmware updates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_url: Option<String>,

    /// Server configuration URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_conf_url: Option<String>,

    /// Application version
    #[serde(default = "default_version")]
    pub version: String,

    /// Edge node unique ID
    pub id: String,

    /// Private key for ECDSA signing (Base64 encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,

    /// Heartbeat interval in milliseconds
    #[serde(default = "default_heartbeat")]
    pub heart_beat: u64,

    /// Report interval in milliseconds
    #[serde(default = "default_report_interval")]
    pub report_interval: u64,

    /// Status interval in milliseconds
    #[serde(default = "default_status_interval")]
    pub status_interval: u64,

    /// Local HTTP server port
    #[serde(default = "default_local_port")]
    pub local_port: u16,
}

fn default_server_address() -> String {
    "192.168.0.1".to_string()
}

fn default_port() -> u16 {
    9999
}

fn default_version() -> String {
    "1.0".to_string()
}

fn default_heartbeat() -> u64 {
    5000
}

fn default_report_interval() -> u64 {
    5000
}

fn default_status_interval() -> u64 {
    1000
}

fn default_local_port() -> u16 {
    22222
}

impl Default for Context {
    fn default() -> Self {
        Self {
            server_address: default_server_address(),
            port: default_port(),
            upgrade_url: None,
            server_conf_url: None,
            version: default_version(),
            id: String::new(),
            private_key: None,
            heart_beat: default_heartbeat(),
            report_interval: default_report_interval(),
            status_interval: default_status_interval(),
            local_port: default_local_port(),
        }
    }
}

/// Context loading errors with full path information
#[derive(Debug, Error)]
pub enum ContextError {
    #[error("Failed to read `id` file: {source}\n  Expected path: {id_path}\n  Working directory: {cwd}\n  Hint: Place the `id` file in the current working directory, or run the program from the directory containing `id`.")]
    IdFileNotFound {
        #[source]
        source: std::io::Error,
        id_path: String,
        cwd: String,
    },

    #[error("Failed to parse `id` file as JSON: {source}\n  Expected path: {id_path}\n  Working directory: {cwd}\n  Content preview: {preview}")]
    IdFileInvalidJson {
        #[source]
        source: serde_json::Error,
        id_path: String,
        cwd: String,
        preview: String,
    },

    #[error("Missing required field `{field}` in `id` file.\n  Expected path: {id_path}\n  Working directory: {cwd}\n  Hint: The `id` file must contain: {{\"id\": \"your-node-id\"}}")]
    MissingField {
        field: &'static str,
        id_path: String,
        cwd: String,
    },

    #[error("Failed to read `serverConf` file: {source}\n  Expected path: {conf_path}\n  Working directory: {cwd}")]
    ServerConfFileInvalid {
        #[source]
        source: serde_json::Error,
        conf_path: String,
        cwd: String,
    },
}

impl Context {
    /// Load context from configuration files
    /// 
    /// Reads `id` file for id and private_key
    /// Reads `serverConf` file for server configuration
    /// 
    /// Files are read from the current working directory (std::env::current_dir).
    /// Expected file paths:
    ///   - `./id` (required) - must contain JSON with `id` field
    ///   - `./serverConf` (optional) - additional server configuration
    pub fn load() -> Result<Self, ContextError> {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());

        let id_path = format!("{}/id", cwd);
        let server_conf_path = format!("{}/serverConf", cwd);

        // Read id file
        let id_content = fs::read_to_string("id").map_err(|source| ContextError::IdFileNotFound {
            source,
            id_path: id_path.clone(),
            cwd: cwd.clone(),
        })?;

        // Parse as generic JSON value first
        let json_value: serde_json::Value = serde_json::from_str(&id_content)
            .map_err(|source| ContextError::IdFileInvalidJson {
                source,
                id_path: id_path.clone(),
                cwd: cwd.clone(),
                preview: id_content.chars().take(200).collect(),
            })?;

        // Deserialize into Context
        let mut ctx: Context = serde_json::from_value(json_value).map_err(|_source| {
            ContextError::MissingField {
                field: "id",
                id_path: id_path.clone(),
                cwd: cwd.clone(),
            }
        })?;

        // Validate required id field
        if ctx.id.is_empty() {
            return Err(ContextError::MissingField {
                field: "id",
                id_path: id_path.clone(),
                cwd: cwd.clone(),
            });
        }

        // Read serverConf file if exists (optional, ignore errors)
        if let Ok(server_conf_content) = fs::read_to_string("serverConf") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&server_conf_content) {
                if let Ok(server_conf) = serde_json::from_value::<Context>(v) {
                    ctx.merge_server_conf(&server_conf);
                } else {
                    // Non-fatal: just skip serverConf on parse error
                }
            }
        }

        Ok(ctx)
    }

    /// Merge server configuration
    fn merge_server_conf(&mut self, server_conf: &Context) {
        if !server_conf.server_address.is_empty() {
            self.server_address = server_conf.server_address.clone();
        }
        if server_conf.port != default_port() {
            self.port = server_conf.port;
        }
        if server_conf.upgrade_url.is_some() {
            self.upgrade_url = server_conf.upgrade_url.clone();
        }
        if server_conf.server_conf_url.is_some() {
            self.server_conf_url = server_conf.server_conf_url.clone();
        }
        if server_conf.heart_beat != default_heartbeat() {
            self.heart_beat = server_conf.heart_beat;
        }
        if server_conf.report_interval != default_report_interval() {
            self.report_interval = server_conf.report_interval;
        }
        if server_conf.status_interval != default_status_interval() {
            self.status_interval = server_conf.status_interval;
        }
        if server_conf.local_port != default_local_port() {
            self.local_port = server_conf.local_port;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_default() {
        let ctx = Context::default();
        assert_eq!(ctx.server_address, "192.168.0.1");
        assert_eq!(ctx.port, 9999);
        assert_eq!(ctx.version, "1.0");
    }

    #[test]
    fn test_context_serde() {
        let json = r#"{
            "id": "test-123",
            "server_address": "192.168.1.100",
            "port": 8888
        }"#;
        let ctx: Context = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.id, "test-123");
        assert_eq!(ctx.server_address, "192.168.1.100");
        assert_eq!(ctx.port, 8888);
    }
}
