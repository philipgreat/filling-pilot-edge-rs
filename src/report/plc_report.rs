//! PLCReport - matches Java PLCReport.java

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlcReport {
    pub report_id: i64,
    pub fill_station: String,
    pub task_id: i64,
    /// Start time as epoch millis
    pub start_time: i64,
    /// End time as epoch millis
    pub end_time: i64,
    pub ip: String,
    pub port: u16,
    pub db_index: i32,
    pub hex_content: String,
    #[serde(skip_serializing)]
    pub sent: bool,
}

impl PlcReport {
    pub fn new() -> Self {
        Self {
            report_id: 0,
            fill_station: String::new(),
            task_id: 0,
            start_time: 0,
            end_time: 0,
            ip: String::new(),
            port: 0,
            db_index: 0,
            hex_content: String::new(),
            sent: false,
        }
    }
}

impl Default for PlcReport {
    fn default() -> Self { Self::new() }
}
