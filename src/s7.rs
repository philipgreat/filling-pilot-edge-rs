//! S7 PLC Communication Module
//! 
//! Uses s7-connector for S7 protocol communication.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::codec::{DataBlock, DataBlockDefinition};
use crate::error::{Error, Result};
use s7_connector::{S7Connection, S7Config};

/// Wrapped connection with Arc+Mutex for safe sharing across async tasks
type SharedConnection = Arc<RwLock<S7Connection>>;

/// S7 connection manager with per-PLC connection pool
pub struct S7Manager {
    /// Connection pool: (host, port) -> SharedConnection
    connections: Arc<RwLock<HashMap<String, SharedConnection>>>,
    /// Default config
    config: S7Config,
}

impl S7Manager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            config: S7Config::default(),
        }
    }

    /// Get or create a shared connection for the given PLC
    async fn get_connection(&self, host: &str, port: u16) -> Result<SharedConnection> {
        let key = format!("{}:{}", host, port);

        // Try existing connection
        {
            let pool = self.connections.read().await;
            if let Some(conn) = pool.get(&key) {
                return Ok(conn.clone());
            }
        }

        // Create new connection
        let mut config = self.config.clone();
        config.host = host.to_string();
        config.port = port;

        let connector = S7Connection::new(config);
        let shared: SharedConnection = Arc::new(RwLock::new(connector));

        // Store in pool
        let mut pool = self.connections.write().await;
        pool.insert(key, shared.clone());

        Ok(shared)
    }

    /// Connect to PLC
    async fn connect(&self, host: &str, port: u16) -> Result<SharedConnection> {
        let shared = self.get_connection(host, port).await?;
        let shared2 = shared.clone();
        {
            let mut conn = shared.write().await;
            conn.connect().await?;
        }
        Ok(shared2)
    }

    /// Read bytes from PLC data block
    pub async fn read_bytes(&self, host: &str, port: u16, db_number: u16, offset: u16, size: u16) -> Result<Vec<u8>> {
        let shared = self.connect(host, port).await?;
        let mut conn = shared.write().await;
        let data = conn.read_db(db_number, offset, size).await
            .map_err(|e| Error::s7(format!("Read failed: {}", e)))?;
        Ok(data.to_vec())
    }

    /// Write bytes to PLC data block
    pub async fn write_bytes(&self, host: &str, port: u16, db_number: u16, offset: u16, data: &[u8]) -> Result<()> {
        let shared = self.connect(host, port).await?;
        let mut conn = shared.write().await;
        conn.write_db(db_number, offset, data).await
            .map_err(|e| Error::s7(format!("Write failed: {}", e)))
    }

    /// Read data block with schema (decode to JSON)
    pub async fn read_data_block(&self, host: &str, port: u16, definition: &DataBlockDefinition) -> Result<DataBlock> {
        let bytes = self.read_bytes(host, port, definition.db_number, 0, definition.total_size as u16).await?;
        Ok(DataBlock::from_bytes(definition.clone(), bytes))
    }

    /// Write data block from JSON values
    pub async fn write_data_block(
        &self,
        host: &str,
        port: u16,
        definition: &DataBlockDefinition,
        values: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        let mut block = DataBlock {
            definition: definition.clone(),
            bytes: vec![],
            values: HashMap::new(),
        };

        let data = block.encode(values)
            .map_err(|e| Error::codec(format!("Encode failed: {}", e)))?;

        self.write_bytes(host, port, definition.db_number, 0, &data).await
    }

    /// Close all connections
    pub async fn close_all(&self) {
        let mut pool = self.connections.write().await;
        for (_, shared) in pool.drain() {
            let mut conn = shared.write().await;
            let _ = conn.disconnect().await;
        }
    }

    /// Test PLC connection status
    /// Returns (connected, latency_ms)
    pub async fn test_connection(&self, host: &str, port: u16) -> (bool, Option<u64>) {
        let start = std::time::Instant::now();
        
        // Try to connect and read a small amount of data
        match self.read_bytes(host, port, 1, 0, 1).await {
            Ok(_) => {
                let latency = start.elapsed().as_millis() as u64;
                (true, Some(latency))
            }
            Err(_) => {
                // Remove failed connection from pool
                let key = format!("{}:{}", host, port);
                let mut pool = self.connections.write().await;
                pool.remove(&key);
                (false, None)
            }
        }
    }

    /// Get all connection keys in the pool
    pub async fn get_connection_keys(&self) -> Vec<String> {
        let pool = self.connections.read().await;
        pool.keys().cloned().collect()
    }

    /// Check all connections and return status map
    pub async fn check_all_connections(&self) -> Vec<PlcConnectionStatus> {
        let keys = self.get_connection_keys().await;
        let mut results = Vec::new();
        
        for key in keys {
            let parts: Vec<&str> = key.split(':').collect();
            if parts.len() == 2 {
                let host = parts[0];
                let port: u16 = parts[1].parse().unwrap_or(102);
                let (connected, latency) = self.test_connection(host, port).await;
                results.push(PlcConnectionStatus {
                    host: host.to_string(),
                    port,
                    connected,
                    latency_ms: latency,
                });
            }
        }
        
        results
    }
}

/// PLC connection status
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlcConnectionStatus {
    pub host: String,
    pub port: u16,
    pub connected: bool,
    #[serde(rename = "latencyMs")]
    pub latency_ms: Option<u64>,
}

impl Default for S7Manager {
    fn default() -> Self {
        Self::new()
    }
}

/// Read request from cloud server
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadRequest {
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "plcId")]
    pub plc_id: String,
    pub ip: String,
    pub port: u16,
    #[serde(rename = "dbIndex")]
    pub db_index: u16,
    pub offset: u16,
    pub size: u16,
}

impl Default for ReadRequest {
    fn default() -> Self {
        Self {
            request_id: None,
            plc_id: String::new(),
            ip: String::new(),
            port: 102,
            db_index: 1,
            offset: 0,
            size: 10,
        }
    }
}

/// Read response to cloud server
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadResponse {
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "plcId")]
    pub plc_id: String,
    pub ip: String,
    pub port: u16,
    #[serde(rename = "dbIndex")]
    pub db_index: u16,
    pub offset: u16,
    pub success: bool,
    pub message: String,
    #[serde(rename = "hexContent")]
    pub hex_content: Option<String>,
}

/// Write request from cloud server
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WriteRequest {
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "plcId")]
    pub plc_id: String,
    pub ip: String,
    pub port: u16,
    #[serde(rename = "dbIndex")]
    pub db_index: u16,
    pub offset: u16,
    #[serde(rename = "hexContent")]
    pub hex_content: String,
}

/// Write response to cloud server
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WriteResponse {
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "plcId")]
    pub plc_id: String,
    pub ip: String,
    pub port: u16,
    #[serde(rename = "dbIndex")]
    pub db_index: u16,
    pub offset: u16,
    pub success: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_request_deserialize() {
        let json = r#"{"plcId":"plc-001","ip":"192.168.1.10","port":102,"dbIndex":1,"offset":0,"size":10}"#;
        let req: ReadRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.plc_id, "plc-001");
        assert_eq!(req.ip, "192.168.1.10");
        assert_eq!(req.db_index, 1);
    }
}
