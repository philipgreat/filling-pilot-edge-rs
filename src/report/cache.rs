//! ReportCache - matches Java ReportCache.java
//! 
//! Manages:
//! - Report reading from PLC (by FillStation config)
//! - Status reading from PLC (by FillStation config)
//! - Sending reports and status to cloud server
//! - Deleting submitted reports

use std::collections::{HashMap, LinkedList, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::s7::S7Manager;
use super::fill_station::FillStation;
use super::plc_report::PlcReport;

/// Current time in milliseconds
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Report cache - manages fill stations, reports, and status
pub struct ReportCache {
    /// FillStation -> List of reports
    caches: Arc<RwLock<HashMap<FillStation, LinkedList<PlcReport>>>>,
    /// Status cache: fillStationId -> hex data
    status_cache: Arc<RwLock<HashMap<String, String>>>,
    /// Last status sent time: fillStationId -> timestamp_ms
    last_status_sent_time: Arc<RwLock<HashMap<String, u64>>>,
    /// S7 manager for PLC reads
    s7_manager: Arc<S7Manager>,
}

impl ReportCache {
    pub fn new(s7_manager: Arc<S7Manager>) -> Self {
        Self {
            caches: Arc::new(RwLock::new(HashMap::new())),
            status_cache: Arc::new(RwLock::new(HashMap::new())),
            last_status_sent_time: Arc::new(RwLock::new(HashMap::new())),
            s7_manager,
        }
    }

    /// Update fill stations from plcInfo (like Java ReportCache.saveReportLocation)
    pub async fn update_fill_stations(&self, stations: Vec<FillStation>) {
        let mut caches = self.caches.write().await;
        let new_set: HashSet<FillStation> = stations.into_iter().collect();
        let existing: HashSet<FillStation> = caches.keys().cloned().collect();
        
        // Remove stations no longer in config
        let to_remove: Vec<FillStation> = existing.difference(&new_set).cloned().collect();
        for s in to_remove {
            caches.remove(&s);
        }
        
        // Add new stations
        let to_add: Vec<FillStation> = new_set.difference(&existing).cloned().collect();
        for s in to_add {
            info!("[REPORT] Added FillStation: {} ({}:{}, reportDb={}, statusDb={})",
                s.id, s.ip, s.port, s.report_db, s.status_db);
            caches.insert(s, LinkedList::new());
        }
    }

    /// Read reports from all stations and return unsent ones as (type, message) pairs
    /// (like Java ReportCache.saveAndTrySendReport, but returns messages instead of sending)
    pub async fn read_and_send_reports_direct(&self) -> Vec<(String, String)> {
        let caches = self.caches.read().await;
        let stations: Vec<FillStation> = caches.keys().cloned().collect();
        drop(caches);
        
        // Read reports for each station
        for station in &stations {
            self.read_report(station).await;
        }
        
        // Collect unsent reports
        let mut messages = Vec::new();
        let caches = self.caches.read().await;
        for (_station, reports) in caches.iter() {
            for report in reports {
                if !report.sent {
                    let msg = serde_json::to_string(report).unwrap_or_default();
                    messages.push(("report".to_string(), msg));
                }
            }
        }
        messages
    }

    /// Read report from a single fill station (like Java ReportCache.readReport)
    async fn read_report(&self, station: &FillStation) {
        // db index = 0 means don't read
        if station.report_db <= 0 {
            return;
        }

        let report_data = match self.s7_manager.read_bytes(
            &station.ip, station.port, station.report_db as u16, 0, station.report_db_size as u16
        ).await {
            Ok(data) => data,
            Err(e) => {
                warn!("[REPORT] Read failed for {} ({}:{} DB{}): {}",
                    station.id, station.ip, station.port, station.report_db, e);
                return;
            }
        };

        let mut caches = self.caches.write().await;
        let reports = caches.entry(station.clone()).or_insert_with(LinkedList::new);
        
        // Get last report
        let last = reports.back().cloned();
        
        // Process new report
        let new_report = self.handle_new_report_read(station, last.as_ref(), &report_data);
        
        match new_report {
            Some(report) => {
                // Remove sent reports when adding new ones (like Java)
                if reports.len() >= station.report_count {
                    reports.pop_front();
                }
                // Remove already-sent reports when adding
                let mut to_keep = LinkedList::new();
                while let Some(r) = reports.pop_front() {
                    if !r.sent { to_keep.push_back(r); }
                }
                *reports = to_keep;
                reports.push_back(report);
            }
            None => {
                // Check if we need to update last report (same task, new content)
                if let Some(last) = reports.back_mut() {
                    let hex = hex::encode(&report_data);
                    if !last.sent && hex != last.hex_content {
                        // Same task but content updated
                        last.hex_content = hex;
                        last.sent = false;
                    }
                }
            }
        }
    }

    /// Process a new report read (like Java ReportCache.handleNewReportRead)
    fn handle_new_report_read(
        &self,
        station: &FillStation,
        last_report: Option<&PlcReport>,
        report_data: &[u8],
    ) -> Option<PlcReport> {
        let hex_content = hex::encode(report_data);
        let now = now_millis() as i64;

        // Try to read task ID (8 bytes at task offset, big-endian i64)
        let task_id = if report_data.len() >= station.report_db_task_offset + 8 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&report_data[station.report_db_task_offset..station.report_db_task_offset + 8]);
            i64::from_be_bytes(buf)
        } else {
            0
        };

        // Try to read start time (DateAndTime at start time offset, S7 format → epoch ms)
        let start_time = self.extract_s7_datetime(report_data, station.report_db_start_time_offset)?;
        // Reject if before 2000-01-01
        if start_time < 946684800000 {
            return None;
        }

        // Try to read end time
        let end_time = self.extract_s7_datetime(report_data, station.report_db_end_time_offset).unwrap_or(0);

        let new_report = PlcReport {
            report_id: now,
            fill_station: station.id.clone(),
            task_id,
            start_time,
            end_time,
            ip: station.ip.clone(),
            port: station.port,
            db_index: station.report_db,
            hex_content,
            sent: false,
        };

        // No previous report, this is the first one
        let last = last_report?;

        // Same hex content as last, ignore
        if new_report.hex_content == last.hex_content {
            return None;
        }

        // Check task ID
        if task_id > 0 {
            if last.task_id == task_id {
                // Same task, just update content (return None to signal update)
                return None;
            }
            // New task, save as new report
            info!("[REPORT] New report: station={}, task={}, start={}", 
                station.id, task_id, start_time);
            return Some(new_report);
        }

        // No task ID, check start/end times
        if new_report.start_time == last.start_time && new_report.end_time == last.end_time {
            // Same times, just update content
            return None;
        }
        
        info!("[REPORT] New report: station={}, start={}, end={}", 
            station.id, start_time, end_time);
        Some(new_report)
    }

    /// Extract S7 DateAndTime (6 bytes: year, month, day, hour, min, sec, ms_high, ms_low)
    /// Returns epoch millis, or None if out of bounds
    fn extract_s7_datetime(&self, data: &[u8], offset: usize) -> Option<i64> {
        // S7 DateAndTime is 8 bytes: year(1), month(1), day(1), hour(1), min(1), sec(1), ms_high(1), ms_low(1)
        if data.len() < offset + 8 {
            return None;
        }
        
        let year_bcd = data[offset];
        let month_bcd = data[offset + 1];
        let day_bcd = data[offset + 2];
        let hour_bcd = data[offset + 3];
        let min_bcd = data[offset + 4];
        let sec_bcd = data[offset + 5];
        
        let year = Self::bcd_to_u8(year_bcd) as i32 + 2000;
        let month = Self::bcd_to_u8(month_bcd) as u32;
        let day = Self::bcd_to_u8(day_bcd) as u32;
        let hour = Self::bcd_to_u8(hour_bcd) as u32;
        let min = Self::bcd_to_u8(min_bcd) as u32;
        let sec = Self::bcd_to_u8(sec_bcd) as u32;
        
        // Convert to epoch millis using chrono-like calculation
        // Simple approach: use UNIX epoch calculation
        let days = Self::days_since_epoch(year, month, day)?;
        let secs = (days as u64 * 86400) + (hour as u64 * 3600) + (min as u64 * 60) + sec as u64;
        Some((secs * 1000) as i64)
    }

    /// Convert BCD byte to u8
    fn bcd_to_u8(bcd: u8) -> u8 {
        (bcd >> 4) * 10 + (bcd & 0x0F)
    }

    /// Calculate days since 1970-01-01
    fn days_since_epoch(year: i32, month: u32, day: u32) -> Option<u64> {
        if month == 0 || month > 12 || day == 0 || day > 31 {
            return None;
        }
        // Simplified: use a lookup approach
        let mut total_days: u64 = 0;
        for y in 1970..year {
            total_days += if Self::is_leap_year(y) { 366 } else { 365 };
        }
        let days_in_months = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for m in 1..month {
            total_days += days_in_months[m as usize];
            if m == 2 && Self::is_leap_year(year) {
                total_days += 1;
            }
        }
        total_days += day as u64 - 1;
        Some(total_days)
    }

    fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    /// Read status and return messages to send (like Java ReportCache.saveAndTrySendStatus)
    pub async fn read_and_send_status_direct(&self) -> Vec<(String, String)> {
        let caches = self.caches.read().await;
        let stations: Vec<FillStation> = caches.keys().cloned().collect();
        drop(caches);
        
        let now = now_millis();
        let mut changed_list: Vec<serde_json::Value> = Vec::new();
        
        for station in &stations {
            if station.status_db <= 0 {
                continue;
            }
            
            // Read status DB
            let status_data = match self.s7_manager.read_bytes(
                &station.ip, station.port, station.status_db as u16, 0, station.status_db_size as u16
            ).await {
                Ok(data) => data,
                Err(e) => {
                    warn!("[STATUS] Read failed for {} ({}:{} DB{}): {}",
                        station.id, station.ip, station.port, station.status_db, e);
                    continue;
                }
            };
            
            let status = hex::encode(&status_data);
            
            // Update cache and check if changed
            let mut status_cache = self.status_cache.write().await;
            let previous = status_cache.insert(station.id.clone(), status.clone());
            drop(status_cache);
            
            // Should we send?
            let mut last_sent = self.last_status_sent_time.write().await;
            let last_sent_time = last_sent.get(&station.id).copied();
            
            let should_send = previous.is_none()
                || previous.as_ref() != Some(&status)
                || last_sent_time.is_none()
                || now - last_sent_time.unwrap() > 30_000;
            
            if should_send {
                last_sent.insert(station.id.clone(), now);
                drop(last_sent);
                
                changed_list.push(serde_json::json!({
                    "fillStation": station.id,
                    "status": status,
                }));
            }
        }
        
        if changed_list.is_empty() {
            Vec::new()
        } else {
            let msg = serde_json::to_string(&changed_list).unwrap_or_default();
            vec![("status".to_string(), msg)]
        }
    }

    /// Delete a report by reportId (like Java ReportCache.deleteReport)
    pub async fn delete_report(&self, report_id: i64) {
        let mut caches = self.caches.write().await;
        for (_station, reports) in caches.iter_mut() {
            for report in reports.iter_mut() {
                if report.report_id == report_id {
                    report.sent = true;
                }
            }
        }
    }

    /// Get all cached statuses (for "status" command)
    pub async fn get_all_statuses(&self) -> Vec<serde_json::Value> {
        let cache = self.status_cache.read().await;
        cache.iter()
            .map(|(k, v)| serde_json::json!({"fillStation": k, "status": v}))
            .collect()
    }
}
