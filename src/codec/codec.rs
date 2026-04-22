//! Codec implementations for PLC data types

use byteorder::{BigEndian, ByteOrder};
use chrono::{Datelike, Timelike};

/// Codec trait for serializing/deserializing PLC data types
pub trait Codec: Send + Sync {
    fn name(&self) -> &'static str;
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String>;
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String>;
    fn byte_size(&self) -> Option<usize> { None }
}

pub struct BooleanCodec;
impl Codec for BooleanCodec {
    fn name(&self) -> &'static str { "boolean" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let b = match value {
            serde_json::Value::Bool(v) => *v as u8,
            serde_json::Value::Number(n) => if n.as_f64().unwrap_or(0.0) != 0.0 { 1 } else { 0 },
            serde_json::Value::String(s) => match s.to_lowercase().as_str() { "true" | "1" | "on" => 1, _ => 0 },
            _ => return Err("Invalid boolean".to_string()),
        };
        Ok(vec![b])
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.is_empty() { return Err("Empty".to_string()); }
        Ok(serde_json::Value::Bool(bytes[0] != 0))
    }
    fn byte_size(&self) -> Option<usize> { Some(1) }
}

pub struct ByteCodec;
impl Codec for ByteCodec {
    fn name(&self) -> &'static str { "byte" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let b: u8 = match value {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as u8,
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid byte".to_string()),
        };
        Ok(vec![b])
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.is_empty() { return Err("Empty".to_string()); }
        Ok(serde_json::Value::Number(bytes[0].into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(1) }
}

pub struct IntegerCodec;
impl Codec for IntegerCodec {
    fn name(&self) -> &'static str { "integer" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let v: i16 = match value {
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i16,
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid int".to_string()),
        };
        let mut buf = vec![0u8; 2];
        BigEndian::write_i16(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 2 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_i16(bytes);
        Ok(serde_json::Value::Number(v.into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(2) }
}

pub struct WordCodec;
impl Codec for WordCodec {
    fn name(&self) -> &'static str { "word" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let v: u16 = match value {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as u16,
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid word".to_string()),
        };
        let mut buf = vec![0u8; 2];
        BigEndian::write_u16(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 2 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_u16(bytes);
        Ok(serde_json::Value::Number(v.into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(2) }
}

pub struct DWordCodec;
impl Codec for DWordCodec {
    fn name(&self) -> &'static str { "dword" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let v: u32 = match value {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as u32,
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid dword".to_string()),
        };
        let mut buf = vec![0u8; 4];
        BigEndian::write_u32(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 4 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_u32(bytes);
        Ok(serde_json::Value::Number(v.into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(4) }
}

pub struct DIntCodec;
impl Codec for DIntCodec {
    fn name(&self) -> &'static str { "dint" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let v: i32 = match value {
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid dint".to_string()),
        };
        let mut buf = vec![0u8; 4];
        BigEndian::write_i32(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 4 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_i32(bytes);
        Ok(serde_json::Value::Number(v.into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(4) }
}

pub struct LIntCodec;
impl Codec for LIntCodec {
    fn name(&self) -> &'static str { "lint" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let v: i64 = match value {
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid lint".to_string()),
        };
        let mut buf = vec![0u8; 8];
        BigEndian::write_i64(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 8 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_i64(bytes);
        Ok(serde_json::Value::Number(v.into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(8) }
}

pub struct RealCodec;
impl Codec for RealCodec {
    fn name(&self) -> &'static str { "real" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let v: f32 = match value {
            serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
            serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
            _ => return Err("Invalid real".to_string()),
        };
        let mut buf = vec![0u8; 4];
        BigEndian::write_f32(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 4 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_f32(bytes);
        Ok(serde_json::Value::Number(serde_json::Number::from_f64(v as f64).unwrap_or(serde_json::Number::from(0))))
    }
    fn byte_size(&self) -> Option<usize> { Some(4) }
}

pub struct StringCodec;
impl Codec for StringCodec {
    fn name(&self) -> &'static str { "string" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let s = match value { serde_json::Value::String(s) => s.clone(), _ => value.to_string() };
        let bytes = s.into_bytes();
        if bytes.len() > 254 { return Err("Too long".to_string()); }
        let mut buf = vec![0u8; 2 + bytes.len()];
        buf[0] = 254; buf[1] = bytes.len() as u8;
        buf[2..].copy_from_slice(&bytes);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 2 { return Err("Header".to_string()); }
        let len = bytes[1] as usize;
        if bytes.len() < 2 + len { return Err("Truncated".to_string()); }
        let s = String::from_utf8(bytes[2..2+len].to_vec()).map_err(|e| e.to_string())?;
        Ok(serde_json::Value::String(s))
    }
}

pub struct DateTimeCodec;
impl Codec for DateTimeCodec {
    fn name(&self) -> &'static str { "datetime" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let s = value.as_str().ok_or("Invalid datetime")?;
        let dt = chrono::DateTime::parse_from_rfc3339(s).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8];
        buf[0] = ((dt.year() - 2000) % 100) as u8;
        buf[1] = dt.month() as u8;
        buf[2] = dt.day() as u8;
        buf[3] = dt.hour() as u8;
        buf[4] = dt.minute() as u8;
        buf[5] = dt.second() as u8;
        buf[6] = (dt.weekday().num_days_from_sunday() + 1) as u8;
        buf[7] = ((dt.timestamp_millis() % 1000) / 10) as u8;
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 8 { return Err("Not enough".to_string()); }
        let year: i64 = 2000 + bytes[0] as i64;
        let dt_str = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]);
        Ok(serde_json::Value::String(dt_str))
    }
}

pub struct BigDecimalCodec;
impl Codec for BigDecimalCodec {
    fn name(&self) -> &'static str { "bigdecimal" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let s = value.as_str().ok_or("Need string")?;
        let v: f64 = s.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
        let mut buf = vec![0u8; 8];
        BigEndian::write_f64(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 8 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_f64(bytes);
        Ok(serde_json::Value::String(format!("{}", v)))
    }
    fn byte_size(&self) -> Option<usize> { Some(8) }
}

pub struct IpCodec;
impl Codec for IpCodec {
    fn name(&self) -> &'static str { "ip" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let s = value.as_str().ok_or("IP must be string")?;
        let parts: Result<Vec<u8>, _> = s.split('.').map(|p| p.parse().map_err(|_| "Bad IP")).collect();
        let parts = parts?;
        if parts.len() != 4 { return Err("Invalid IP".to_string()); }
        Ok(parts)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 4 { return Err("Not enough".to_string()); }
        Ok(serde_json::Value::String(format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])))
    }
    fn byte_size(&self) -> Option<usize> { Some(4) }
}

/// CharCodec - single byte as char (matches Java CharCodec)
/// Uses bit_offset from property definition for bit-level access
pub struct CharCodec;
impl Codec for CharCodec {
    fn name(&self) -> &'static str { "char" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let c = match value {
            serde_json::Value::String(s) => s.chars().next().unwrap_or(' ') as u8,
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as u8,
            _ => return Err("Invalid char".to_string()),
        };
        Ok(vec![c])
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.is_empty() { return Err("Empty".to_string()); }
        // Java: (char) b.intValue()
        let c = bytes[0] as char;
        Ok(serde_json::Value::String(c.to_string()))
    }
    fn byte_size(&self) -> Option<usize> { Some(1) }
}

/// WStringCodec - UTF-16BE encoded wide string (matches Java WStringCodec)
/// Format: [capacity(2 bytes LE)][real_len(2 bytes LE)][UTF-16BE data]
pub struct WStringCodec;
impl Codec for WStringCodec {
    fn name(&self) -> &'static str { "wstring" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        let s = value.as_str().ok_or("WString needs string")?;
        let utf16: Vec<u8> = s.encode_utf16()
            .flat_map(|cp| cp.to_be_bytes())
            .collect();
        let char_count = utf16.len() / 2;
        // Default capacity 254 chars (508 bytes) if not specified
        let capacity = 254;
        let mut buf = vec![0u8; 4 + utf16.len()];
        // capacity in first 2 bytes LE
        buf[0] = (capacity & 0xFF) as u8;
        buf[1] = ((capacity >> 8) & 0xFF) as u8;
        // real length in next 2 bytes LE
        buf[2] = (char_count & 0xFF) as u8;
        buf[3] = ((char_count >> 8) & 0xFF) as u8;
        // UTF-16BE data
        buf[4..].copy_from_slice(&utf16);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 4 { return Err("WString header too short".to_string()); }
        // First 2 bytes: real char count (LE)
        let len = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        if bytes.len() < 4 + len * 2 { return Err("WString truncated".to_string()); }
        let mut utf16_buf = vec![0u16; len];
        for i in 0..len {
            let offset = 2 + i * 2;
            utf16_buf[i] = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        }
        let s: String = String::from_utf16_lossy(&utf16_buf);
        Ok(serde_json::Value::String(s))
    }
}

/// IntegerAsBigDecimalCodec - decodes i16 as BigDecimal string (matches Java IntegerAsBigDecimalCodec)
/// Extends IntegerCodec: decode as i16, then wrap in BigDecimal string
pub struct IntegerAsBigDecimalCodec;
impl Codec for IntegerAsBigDecimalCodec {
    fn name(&self) -> &'static str { "intAsBigDecimal" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        // Same as IntegerCodec: encode as i16 big-endian
        let v: i16 = match value {
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i16,
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid intAsBigDecimal".to_string()),
        };
        let mut buf = vec![0u8; 2];
        BigEndian::write_i16(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 2 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_i16(bytes);
        // Wrap as BigDecimal string (matches Java: new BigDecimal(v))
        Ok(serde_json::Value::String(format!("{}", v)))
    }
    fn byte_size(&self) -> Option<usize> { Some(2) }
}

/// LongAsIntCodec - decodes i64 as i16 (matches Java LongAsIntCodec)
/// Extends LongCodec: decode as i64, then narrow to i16
pub struct LongAsIntCodec;
impl Codec for LongAsIntCodec {
    fn name(&self) -> &'static str { "longAsInt" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        // Encode as i64 (8 bytes big-endian)
        let v: i64 = match value {
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
            serde_json::Value::String(s) => s.parse().unwrap_or(0),
            _ => return Err("Invalid longAsInt".to_string()),
        };
        let mut buf = vec![0u8; 8];
        BigEndian::write_i64(&mut buf, v);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 8 { return Err("Not enough".to_string()); }
        let v = BigEndian::read_i64(bytes);
        // Narrow to i16 (matches Java: v.intValue())
        let v16 = v as i16;
        Ok(serde_json::Value::Number(v16.into()))
    }
    fn byte_size(&self) -> Option<usize> { Some(8) }
}

/// LongAsDateTimeCodec - decodes 8-byte i64 (epoch millis) as DateTime string (matches Java LongAsDateTimeCodec)
/// The i64 is milliseconds since 1970-01-01 00:00:00 UTC
pub struct LongAsDateTimeCodec;
impl Codec for LongAsDateTimeCodec {
    fn name(&self) -> &'static str { "longAsDateTime" }
    fn encode(&self, value: &serde_json::Value) -> Result<Vec<u8>, String> {
        // Parse datetime string to epoch millis, encode as i64 big-endian
        let s = value.as_str().ok_or("Need datetime string")?;
        let millis = chrono::DateTime::parse_from_rfc3339(s)
            .map_err(|e| e.to_string())?
            .timestamp_millis();
        let mut buf = vec![0u8; 8];
        BigEndian::write_i64(&mut buf, millis);
        Ok(buf)
    }
    fn decode(&self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        if bytes.len() < 8 { return Err("Not enough".to_string()); }
        let millis = BigEndian::read_i64(bytes);
        // Convert epoch millis to datetime string
        let secs = millis / 1000;
        let remaining = (millis % 1000) as u32;
        let datetime = chrono::DateTime::from_timestamp(secs, remaining * 1_000_000)
            .ok_or_else(|| format!("Invalid timestamp: {}", millis))?;
        Ok(serde_json::Value::String(datetime.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
    }
    fn byte_size(&self) -> Option<usize> { Some(8) }
}

/// Codec registry
pub struct CodecRegistry {
    codecs: std::collections::HashMap<String, Box<dyn Codec>>,
}

impl Default for CodecRegistry {
    fn default() -> Self {
        let mut r = Self { codecs: std::collections::HashMap::new() };
        r.codecs.insert("boolean".into(), Box::new(BooleanCodec));
        r.codecs.insert("byte".into(), Box::new(ByteCodec));
        r.codecs.insert("integer".into(), Box::new(IntegerCodec));
        r.codecs.insert("word".into(), Box::new(WordCodec));
        r.codecs.insert("dword".into(), Box::new(DWordCodec));
        r.codecs.insert("dint".into(), Box::new(DIntCodec));
        r.codecs.insert("lint".into(), Box::new(LIntCodec));
        r.codecs.insert("real".into(), Box::new(RealCodec));
        r.codecs.insert("string".into(), Box::new(StringCodec));
        r.codecs.insert("datetime".into(), Box::new(DateTimeCodec));
        r.codecs.insert("bigdecimal".into(), Box::new(BigDecimalCodec));
        r.codecs.insert("ip".into(), Box::new(IpCodec));
        r.codecs.insert("char".into(), Box::new(CharCodec));
        r.codecs.insert("wstring".into(), Box::new(WStringCodec));
        r.codecs.insert("intAsBigDecimal".into(), Box::new(IntegerAsBigDecimalCodec));
        r.codecs.insert("longAsInt".into(), Box::new(LongAsIntCodec));
        r.codecs.insert("longAsDateTime".into(), Box::new(LongAsDateTimeCodec));
        r
    }
}

impl CodecRegistry {
    pub fn new() -> Self { Self::default() }
    pub fn get(&self, name: &str) -> Option<&dyn Codec> {
        self.codecs.get(name).map(|b| b.as_ref())
    }
}
