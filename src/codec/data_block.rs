//! Data block definition and management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::CodecRegistry;

/// Property type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PropertyType {
    Boolean,
    Byte,
    Integer,
    Word,
    DWord,
    DInt,
    LInt,
    Real,
    String,
    WString,
    DateTime,
    BigDecimal,
    IpAddress,
    Long,
    LongAsInt,
    LongAsDateTime,
    IntegerAsBigDecimal,
}

impl PropertyType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "boolean" | "bool" => PropertyType::Boolean,
            "byte" => PropertyType::Byte,
            "integer" | "int" => PropertyType::Integer,
            "word" => PropertyType::Word,
            "dword" => PropertyType::DWord,
            "dint" => PropertyType::DInt,
            "lint" => PropertyType::LInt,
            "real" | "float" => PropertyType::Real,
            "string" => PropertyType::String,
            "wstring" => PropertyType::WString,
            "datetime" | "date" => PropertyType::DateTime,
            "bigdecimal" => PropertyType::BigDecimal,
            "ip" | "ipaddress" => PropertyType::IpAddress,
            "long" => PropertyType::Long,
            "longasint" => PropertyType::LongAsInt,
            "longasdatetime" => PropertyType::LongAsDateTime,
            "integerasbigdecimal" => PropertyType::IntegerAsBigDecimal,
            _ => PropertyType::Byte, // default
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PropertyType::Boolean => "boolean",
            PropertyType::Byte => "byte",
            PropertyType::Integer => "integer",
            PropertyType::Word => "word",
            PropertyType::DWord => "dword",
            PropertyType::DInt => "dint",
            PropertyType::LInt => "lint",
            PropertyType::Real => "real",
            PropertyType::String => "string",
            PropertyType::WString => "wstring",
            PropertyType::DateTime => "datetime",
            PropertyType::BigDecimal => "bigdecimal",
            PropertyType::IpAddress => "ip",
            PropertyType::Long => "long",
            PropertyType::LongAsInt => "longasint",
            PropertyType::LongAsDateTime => "longasdatetime",
            PropertyType::IntegerAsBigDecimal => "integerasbigdecimal",
        }
    }
}

/// Property definition within a data block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBlockProperty {
    /// Property name
    pub name: String,

    /// Property type
    #[serde(rename = "type")]
    pub property_type: PropertyType,

    /// Byte offset in the data block
    pub offset: usize,

    /// Number of elements (for arrays)
    #[serde(default = "default_count")]
    pub count: usize,

    /// Byte size (for variable types)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<usize>,
}

fn default_count() -> usize {
    1
}

/// Data block definition (schema for PLC data block)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBlockDefinition {
    /// Data block number
    pub db_number: u16,

    /// Data block name/identifier
    pub name: String,

    /// PLC station/IP
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plc_ip: Option<String>,

    /// Properties (fields) in this data block
    #[serde(default)]
    pub properties: Vec<DataBlockProperty>,

    /// Total byte size
    #[serde(skip)]
    pub total_size: usize,
}

impl DataBlockDefinition {
    /// Calculate total byte size from properties
    pub fn calculate_size(&mut self) {
        self.total_size = self.properties.iter()
            .map(|p| p.byte_size.unwrap_or(Self::type_size(&p.property_type)) * p.count)
            .sum();
    }

    /// Get byte size for a property type
    fn type_size(property_type: &PropertyType) -> usize {
        match property_type {
            PropertyType::Boolean => 1,
            PropertyType::Byte => 1,
            PropertyType::Integer => 2,
            PropertyType::Word => 2,
            PropertyType::DWord => 4,
            PropertyType::DInt => 4,
            PropertyType::LInt => 8,
            PropertyType::Real => 4,
            PropertyType::String => 256, // max
            PropertyType::WString => 512, // max
            PropertyType::DateTime => 8,
            PropertyType::BigDecimal => 8,
            PropertyType::IpAddress => 4,
            PropertyType::Long => 8,
            PropertyType::LongAsInt => 4,
            PropertyType::LongAsDateTime => 8,
            PropertyType::IntegerAsBigDecimal => 8,
        }
    }
}

/// In-memory data block instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBlock {
    /// Definition reference
    pub definition: DataBlockDefinition,

    /// Raw bytes from PLC
    #[serde(skip)]
    pub bytes: Vec<u8>,

    /// Decoded values
    #[serde(skip)]
    pub values: HashMap<String, serde_json::Value>,
}

impl DataBlock {
    /// Create from definition and raw bytes
    pub fn from_bytes(definition: DataBlockDefinition, bytes: Vec<u8>) -> Self {
        let mut block = Self {
            definition,
            bytes,
            values: HashMap::new(),
        };
        block.decode();
        block
    }

    /// Decode raw bytes to JSON values
    pub fn decode(&mut self) {
        self.values.clear();
        let registry = CodecRegistry::new();

        for prop in &self.definition.properties {
            if let Some(codec) = registry.get(prop.property_type.as_str()) {
                let start = prop.offset;
                let end = start + prop.byte_size.unwrap_or(Self::type_size(&prop.property_type));

                if end <= self.bytes.len() {
                    let prop_bytes = &self.bytes[start..end];
                    if prop.count == 1 {
                        if let Ok(value) = codec.decode(prop_bytes) {
                            self.values.insert(prop.name.clone(), value);
                        }
                    } else {
                        // Array of values
                        let type_size = prop.byte_size.unwrap_or(Self::type_size(&prop.property_type));
                        let mut arr = Vec::new();
                        for i in 0..prop.count {
                            let item_start = i * type_size;
                            let item_end = item_start + type_size;
                            if item_end <= prop_bytes.len() {
                                if let Ok(value) = codec.decode(&prop_bytes[item_start..item_end]) {
                                    arr.push(value);
                                }
                            }
                        }
                        if !arr.is_empty() {
                            self.values.insert(prop.name.clone(), serde_json::Value::Array(arr));
                        }
                    }
                }
            }
        }
    }

    /// Encode JSON values to raw bytes
    pub fn encode(&mut self, values: &HashMap<String, serde_json::Value>) -> Result<Vec<u8>, String> {
        let registry = CodecRegistry::new();
        let mut result = vec![0u8; self.definition.total_size];

        for prop in &self.definition.properties {
            if let Some(value) = values.get(&prop.name) {
                if let Some(codec) = registry.get(prop.property_type.as_str()) {
                    let start = prop.offset;
                    let type_size = prop.byte_size.unwrap_or(Self::type_size(&prop.property_type));

                    if prop.count == 1 {
                        let end = start + type_size;
                        if end <= result.len() {
                            if let Ok(encoded) = codec.encode(value) {
                                result[start..end].copy_from_slice(&encoded);
                            }
                        }
                    } else {
                        // Array
                        if let serde_json::Value::Array(arr) = value {
                            for (i, item) in arr.iter().take(prop.count).enumerate() {
                                let item_start = start + i * type_size;
                                let item_end = item_start + type_size;
                                if item_end <= result.len() {
                                    if let Ok(encoded) = codec.encode(item) {
                                        result[item_start..item_end].copy_from_slice(&encoded);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    fn type_size(property_type: &PropertyType) -> usize {
        DataBlockDefinition::type_size(property_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_type_from_str() {
        assert_eq!(PropertyType::from_str("boolean"), PropertyType::Boolean);
        assert_eq!(PropertyType::from_str("INTEGER"), PropertyType::Integer);
        assert_eq!(PropertyType::from_str("real"), PropertyType::Real);
    }

    #[test]
    fn test_data_block_definition_size() {
        let mut def = DataBlockDefinition {
            db_number: 1,
            name: "test".to_string(),
            plc_ip: None,
            properties: vec![
                DataBlockProperty {
                    name: "status".to_string(),
                    property_type: PropertyType::Boolean,
                    offset: 0,
                    count: 1,
                    byte_size: Some(1),
                },
                DataBlockProperty {
                    name: "temperature".to_string(),
                    property_type: PropertyType::Real,
                    offset: 1,
                    count: 1,
                    byte_size: Some(4),
                },
            ],
            total_size: 0,
        };
        def.calculate_size();
        // Sequential: status at 0(1 byte) + temperature at 1(4 bytes) = 5 bytes total
        assert_eq!(def.total_size, 5);
    }
}
