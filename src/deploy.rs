//! PLC Serialization Metadata & Deploy Module
//! 
//! Matches Java PLCSerializeMetaFactory.java:
//! - DataBlockDefinition cache (loaded/saved to ~/.edge/object-meta.json)
//! - Hex data decoding using codec registry
//! - Deploy: write decoded data back to PLC

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

use crate::s7::S7Manager;
use crate::codec::{CodecRegistry, Codec};

/// Property definition within a data block (matches Java DataBlockPropertyDefinition)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataBlockPropertyDefinition {
    /// Field name
    pub name: String,
    /// Data type: int, string, real, bool, datetime, bigdecimal, lint, etc.
    #[serde(rename = "type")]
    pub property_type: String,
    /// Byte offset in the data block
    pub offset: usize,
    /// Number of elements (default 1)
    #[serde(default = "default_count")]
    pub count: usize,
    /// Byte size override (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<usize>,
}

fn default_count() -> usize { 1 }

/// Data block definition (matches Java DataBlockDefinition)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataBlockDefinition {
    /// Java type name (used as cache key)
    #[serde(rename = "javaType")]
    pub java_type: String,
    /// Total block size in bytes
    #[serde(rename = "blockSize")]
    pub block_size: usize,
    /// List of field definitions
    #[serde(rename = "dataBlockPropertyDefinitionList")]
    #[serde(default)]
    pub data_block_property_definition_list: Vec<DataBlockPropertyDefinition>,
}

/// Deployment request (write data to PLC)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    /// Fill station ID (optional — if null, use monitor's ip/port)
    #[serde(rename = "fillStationId", skip_serializing_if = "Option::is_none")]
    pub fill_station_id: Option<String>,
    /// PLC IP (optional)
    pub ip: Option<String>,
    /// PLC port (optional)
    pub port: Option<u16>,
    /// DB number to write to
    #[serde(rename = "dbNumber")]
    pub db_number: usize,
    /// Byte offset to start writing
    pub offset: usize,
    /// Number of bytes to write (0 = auto, write all from offset)
    pub length: usize,
    /// Content type (javaType, used to look up DataBlockDefinition)
    #[serde(rename = "contentType")]
    pub content_type: String,
    /// JSON content (field values to write)
    pub content: serde_json::Value,
}

/// Multiple deployments
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployments {
    pub deployments: Vec<Deployment>,
}

/// Pilot callback response from local HTTP endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct PilotCallbackResponse {
    #[serde(rename = "pilotCallback", skip_serializing_if = "Option::is_none")]
    pub pilot_callback: Option<String>,
}

/// Metadata cache - manages DataBlockDefinitions
/// Persists to ~/.edge/object-meta.json every 60 seconds
pub struct PLCSerializeMetaFactory {
    /// Cache: javaType -> DataBlockDefinition
    cache: Arc<RwLock<HashMap<String, DataBlockDefinition>>>,
    /// Persistence file path
    cache_file: PathBuf,
}

impl PLCSerializeMetaFactory {
    /// Create a new factory and start persistence loop
    pub fn new() -> Arc<Self> {
        let cache_file = Self::cache_file_path();
        
        let factory = Arc::new(Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_file: cache_file.clone(),
        });
        
        // Load from disk on startup
        let cache = Arc::clone(&factory.cache);
        let cache_file_load = cache_file.clone();
        std::thread::spawn(move || {
            Self::load_from_disk(&cache, &cache_file_load);
        });

        // Start persistence loop (every 60 seconds, matches Java)
        let cache_persist = Arc::clone(&factory.cache);
        let cache_file_persist = cache_file;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                Self::persist_to_disk(&cache_persist, &cache_file_persist);
            }
        });

        factory
    }

    /// Get home directory cache path: ~/.edge/
    fn cache_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".edge")
    }

    /// Cache file path: ~/.edge/object-meta.json
    fn cache_file_path() -> PathBuf {
        Self::cache_dir().join("object-meta.json")
    }

    /// Monitor meta file path: ~/.edge/monitor-meta.json
    pub fn monitor_meta_file_path() -> PathBuf {
        Self::cache_dir().join("monitor-meta.json")
    }

    /// Load cache from disk (blocking, called on startup)
    fn load_from_disk(cache: &Arc<RwLock<HashMap<String, DataBlockDefinition>>>, path: &PathBuf) {
        if !path.exists() {
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => {
                match serde_json::from_str::<HashMap<String, DataBlockDefinition>>(&content) {
                    Ok(loaded) => {
                        let count = loaded.len();
                        // Use a blocking block to do this safely
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .unwrap();
                        rt.block_on(async {
                            let mut c = cache.write().await;
                            *c = loaded;
                        });
                        info!("[META] Loaded {} DataBlockDefinitions from {}", count, path.display());
                    }
                    Err(e) => {
                        warn!("[META] Failed to parse {}: {}", path.display(), e);
                    }
                }
            }
            Err(e) => {
                warn!("[META] Failed to read {}: {}", path.display(), e);
            }
        }
    }

    /// Persist cache to disk (called every 60s)
    fn persist_to_disk(cache: &Arc<RwLock<HashMap<String, DataBlockDefinition>>>, path: &PathBuf) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cache_snapshot = rt.block_on(async {
            cache.read().await.clone()
        });
        
        if cache_snapshot.is_empty() {
            return;
        }
        
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        
        match serde_json::to_string_pretty(&cache_snapshot) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    warn!("[META] Failed to write {}: {}", path.display(), e);
                } else {
                    info!("[META] Persisted {} DataBlockDefinitions to {}", cache_snapshot.len(), path.display());
                }
            }
            Err(e) => {
                warn!("[META] Failed to serialize metadata: {}", e);
            }
        }
    }

    /// Get a clone of the cache Arc (for passing to decode/encode functions)
    pub fn cache(&self) -> Arc<RwLock<HashMap<String, DataBlockDefinition>>> {
        Arc::clone(&self.cache)
    }

    /// Get a DataBlockDefinition by type name (throws if not found)
    pub async fn get_object_definition(&self, java_type: &str) -> Option<DataBlockDefinition> {
        let cache = self.cache.read().await;
        cache.get(java_type).cloned()
    }

    /// Save metadata from plcInfo (called from PlcInfo handler, matches Java saveMeta)
    pub async fn save_meta(&self, plc_info: &[serde_json::Value]) {
        let mut all_definitions: Vec<DataBlockDefinition> = Vec::new();
        
        for plc in plc_info {
            if let Some(data_meta) = plc.get("dataMeta").and_then(|v| v.as_array()) {
                for item in data_meta {
                    if let Ok(def) = serde_json::from_value::<DataBlockDefinition>(item.clone()) {
                        all_definitions.push(def);
                    }
                }
            }
        }
        
        if all_definitions.is_empty() {
            return;
        }
        
        let mut cache = self.cache.write().await;
        for def in &all_definitions {
            let old = cache.insert(def.java_type.clone(), def.clone());
            if old.is_none() {
                info!("[META] Registered DataBlockDefinition: {} ({} bytes, {} fields)", 
                    def.java_type, def.block_size, def.data_block_property_definition_list.len());
            }
        }
    }

    /// Decode raw bytes as a Map using a DataBlockDefinition (matches Java PLCSerializeMetaFactory.decodeAsMap)
    pub fn decode_as_map(data: &[u8], java_type: &str, cache: &Arc<RwLock<HashMap<String, DataBlockDefinition>>>) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        // Use blocking access since this is called from async context
        let rt = tokio::runtime::Handle::current();
        let def = rt.block_on(async {
            cache.read().await.get(java_type).cloned()
        });
        
        let def = match def {
            Some(d) => d,
            None => return Err(format!("Missing DataBlockDefinition for type: {}", java_type)),
        };

        let registry = CodecRegistry::new();
        let mut map = serde_json::Map::new();

        for prop in &def.data_block_property_definition_list {
            let type_size = prop.byte_size.unwrap_or_else(|| Self::type_size(&prop.property_type));
            let start = prop.offset;
            let end = (start + type_size * prop.count).min(data.len());

            if start >= data.len() {
                continue;
            }

            if prop.count == 1 {
                let prop_bytes = &data[start..end];
                if let Some(codec) = registry.get(&prop.property_type) {
                    if let Ok(value) = codec.decode(prop_bytes) {
                        map.insert(prop.name.clone(), value);
                    }
                }
            } else {
                let mut arr = Vec::new();
                for i in 0..prop.count {
                    let item_start = start + i * type_size;
                    let item_end = (item_start + type_size).min(data.len());
                    if item_start >= data.len() { break; }
                    let prop_bytes = &data[item_start..item_end];
                    if let Some(codec) = registry.get(&prop.property_type) {
                        if let Ok(value) = codec.decode(prop_bytes) {
                            arr.push(value);
                        }
                    }
                }
                if !arr.is_empty() {
                    map.insert(prop.name.clone(), serde_json::Value::Array(arr));
                }
            }
        }

        Ok(map)
    }

    /// Encode JSON values to bytes using a DataBlockDefinition (matches Java commonDataBlockCodec.encode)
    pub fn encode(content: &serde_json::Value, java_type: &str, cache: &Arc<RwLock<HashMap<String, DataBlockDefinition>>>) -> Result<Vec<u8>, String> {
        let content_map = match content {
            serde_json::Value::Object(m) => m.clone(),
            _ => return Err("Content must be a JSON object".to_string()),
        };

        let def = {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                cache.read().await.get(java_type).cloned()
            })
        };

        let def = match def {
            Some(d) => d,
            None => return Err(format!("Missing DataBlockDefinition for type: {}", java_type)),
        };

        let registry = CodecRegistry::new();
        let mut bytes = vec![0u8; def.block_size];

        for prop in &def.data_block_property_definition_list {
            if let Some(value) = content_map.get(&prop.name) {
                let type_size = prop.byte_size.unwrap_or_else(|| Self::type_size(&prop.property_type));
                let start = prop.offset;

                if prop.count == 1 {
                    let end = (start + type_size).min(bytes.len());
                    if let Some(codec) = registry.get(&prop.property_type) {
                        if let Ok(encoded) = codec.encode(value) {
                            bytes[start..end].copy_from_slice(&encoded[..(end - start)]);
                        }
                    }
                } else {
                    if let serde_json::Value::Array(arr) = value {
                        for (i, item) in arr.iter().take(prop.count).enumerate() {
                            let item_start = start + i * type_size;
                            let item_end = (item_start + type_size).min(bytes.len());
                            if let Some(codec) = registry.get(&prop.property_type) {
                                if let Ok(encoded) = codec.encode(item) {
                                    bytes[item_start..item_end].copy_from_slice(&encoded[..(item_end - item_start)]);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(bytes)
    }

    /// Get byte size for a type name
    fn type_size(type_name: &str) -> usize {
        match type_name.to_lowercase().as_str() {
            "boolean" | "bool" => 1,
            "byte" => 1,
            "char" => 1,
            "integer" | "int" | "word" => 2,
            "intAsBigDecimal" => 2,
            "dint" | "dword" | "real" | "float" => 4,
            "longAsInt" => 8,
            "lint" | "long" | "datetime" | "bigdecimal" | "longAsDateTime" | "integerAsBigDecimal" => 8,
            "string" => 256,
            "wstring" => 512,
            "ip" | "ipaddress" => 4,
            _ => 1,
        }
    }
}

impl Default for PLCSerializeMetaFactory {
    fn default() -> Self {
        Self::new();
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_file: Self::cache_file_path(),
        }
    }
}

/// Deploy data to a PLC (matches Java PLCSerializeMetaFactory.deploy)
/// 
/// Takes a Deployment and writes the encoded data to the specified PLC.
/// If fillStationId is provided, looks up the IP/port from fill_stations.
/// Otherwise uses the monitor's ip/port.
pub async fn deploy(
    deployment: &Deployment,
    s7_manager: &Arc<S7Manager>,
    meta_factory: &Arc<PLCSerializeMetaFactory>,
    fill_stations: &Arc<RwLock<HashMap<String, (String, u16)>>>,
    monitor_ip: Option<&str>,
    monitor_port: Option<u16>,
) -> Result<(), String> {
    // Resolve IP and port
    let (ip, port) = if let Some(ref fs_id) = deployment.fill_station_id {
        let stations = fill_stations.read().await;
        match stations.get(fs_id) {
            Some((ip, port)) => (ip.clone(), *port),
            None => return Err(format!("FillStation {} not found", fs_id)),
        }
    } else {
        match (monitor_ip, monitor_port) {
            (Some(ip), Some(port)) => (ip.to_string(), port),
            _ => return Err("Cannot resolve PLC IP/port for deploy".to_string()),
        }
    };

    // Encode the content
    let encoded = PLCSerializeMetaFactory::encode(
        &deployment.content,
        &deployment.content_type,
        &meta_factory.cache(),
    )?;

    // Calculate the slice to write
    let offset = deployment.offset;
    let end = if deployment.length > 0 {
        (offset + deployment.length).min(encoded.len())
    } else {
        encoded.len()
    };

    let to_write = &encoded[offset..end];

    info!("[DEPLOY] Writing {} bytes to {}:{} DB{}, offset={}", 
        to_write.len(), ip, port, deployment.db_number, offset);

    s7_manager.write_bytes(&ip, port, deployment.db_number as u16, offset as u16, to_write)
        .await
        .map_err(|e| format!("S7 write failed: {}", e))?;

    info!("[DEPLOY] Done writing to {}:{} DB{}, offset={}", ip, port, deployment.db_number, offset);
    Ok(())
}

/// Deploy all deployments (matches Java PLCSerializeMetaFactory.deployAll)
pub async fn deploy_all(
    deployments: &Deployments,
    s7_manager: &Arc<S7Manager>,
    meta_factory: &Arc<PLCSerializeMetaFactory>,
    fill_stations: &Arc<RwLock<HashMap<String, (String, u16)>>>,
    monitor_ip: Option<&str>,
    monitor_port: Option<u16>,
) {
    for dep in &deployments.deployments {
        if let Err(e) = deploy(dep, s7_manager, meta_factory, fill_stations, monitor_ip, monitor_port).await {
            error!("[DEPLOY] Failed: {}", e);
        }
    }
}
