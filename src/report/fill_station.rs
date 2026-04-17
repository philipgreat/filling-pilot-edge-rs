//! FillStation - matches Java FillStation.java
//! 
//! Configuration for a filling station (report + status DB layout)

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillStation {
    pub id: String,
    pub ip: String,
    pub port: u16,

    #[serde(default = "default_report_db")]
    pub report_db: i32,

    #[serde(default = "default_status_db")]
    pub status_db: i32,

    #[serde(default = "default_report_count")]
    pub report_count: usize,

    #[serde(default = "default_report_db_size")]
    pub report_db_size: usize,

    #[serde(default = "default_report_db_task_offset")]
    pub report_db_task_offset: usize,

    #[serde(default = "default_report_db_start_time_offset")]
    pub report_db_start_time_offset: usize,

    #[serde(default = "default_report_db_end_time_offset")]
    pub report_db_end_time_offset: usize,

    #[serde(default = "default_status_db_size")]
    pub status_db_size: usize,
}

fn default_report_db() -> i32 { 0 }
fn default_status_db() -> i32 { 0 }
fn default_report_count() -> usize { 50 }
fn default_report_db_size() -> usize { 808 }
fn default_report_db_task_offset() -> usize { 780 }
fn default_report_db_start_time_offset() -> usize { 472 }
fn default_report_db_end_time_offset() -> usize { 480 }
fn default_status_db_size() -> usize { 38 }

impl PartialEq for FillStation {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.ip == other.ip
            && self.port == other.port
            && self.report_db == other.report_db
            && self.status_db == other.status_db
            && self.report_count == other.report_count
            && self.report_db_size == other.report_db_size
            && self.report_db_task_offset == other.report_db_task_offset
            && self.report_db_start_time_offset == other.report_db_start_time_offset
            && self.report_db_end_time_offset == other.report_db_end_time_offset
            && self.status_db_size == other.status_db_size
    }
}

impl Eq for FillStation {}

impl Hash for FillStation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.ip.hash(state);
        self.port.hash(state);
        self.report_db.hash(state);
        self.status_db.hash(state);
        self.report_count.hash(state);
        self.report_db_size.hash(state);
        self.report_db_task_offset.hash(state);
        self.report_db_start_time_offset.hash(state);
        self.report_db_end_time_offset.hash(state);
        self.status_db_size.hash(state);
    }
}

impl FillStation {
    /// Parse fillStations from plcInfo JSON (array of PLC objects)
    pub fn parse_from_plc_info(plc_info: &[serde_json::Value]) -> Vec<Self> {
        let mut stations = Vec::new();
        for plc in plc_info {
            let ip = match plc.get("ipAddress").and_then(|v| v.as_str()) {
                Some(ip) => ip.to_string(),
                None => continue,
            };
            let port = match plc.get("portNumber").and_then(|v| v.as_u64()) {
                Some(p) => p as u16,
                None => continue,
            };

            if let Some(fill_stations) = plc.get("fillStations").and_then(|v| v.as_array()) {
                for fs in fill_stations {
                    let station: FillStation = match serde_json::from_value(
                        serde_json::json!({
                            "ip": ip,
                            "port": port,
                            "id": fs.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            "reportDb": fs.get("reportDb").and_then(|v| v.as_i64()).unwrap_or(0),
                            "statusDb": fs.get("statusDb").and_then(|v| v.as_i64()).unwrap_or(0),
                            "reportCount": fs.get("reportCount").and_then(|v| v.as_i64()).unwrap_or(50),
                            "reportDbSize": fs.get("reportDbSize").and_then(|v| v.as_i64()).unwrap_or(808),
                            "reportDbTaskOffset": fs.get("reportDbTaskOffset").and_then(|v| v.as_i64()).unwrap_or(780),
                            "reportDbStartTimeOffset": fs.get("reportDbStartTimeOffset").and_then(|v| v.as_i64()).unwrap_or(472),
                            "reportDbEndTimeOffset": fs.get("reportDbEndTimeOffset").and_then(|v| v.as_i64()).unwrap_or(480),
                            "statusDbSize": fs.get("statusDbSize").and_then(|v| v.as_i64()).unwrap_or(38),
                        })
                    ) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    stations.push(station);
                }
            }
        }
        stations
    }
}
