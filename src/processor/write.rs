//! Write processor

use std::sync::Arc;
use tracing::info;

use crate::s7::{WriteRequest, WriteResponse, S7Manager};

pub struct WriteProcessor {
    s7_manager: Arc<S7Manager>,
}

impl WriteProcessor {
    pub fn new(s7_manager: Arc<S7Manager>) -> Self {
        Self { s7_manager }
    }

    pub async fn handle(&self, request: WriteRequest) -> Result<WriteResponse, crate::error::Error> {
        info!("Write: PLC={}, IP={}:{}, DB={}", request.plc_id, request.ip, request.port, request.db_index);

        let mut response = WriteResponse {
            request_id: request.request_id,
            plc_id: request.plc_id.clone(),
            ip: request.ip.clone(),
            port: request.port,
            db_index: request.db_index,
            offset: request.offset,
            success: false,
            message: String::new(),
        };

        let data = match hex::decode(&request.hex_content) {
            Ok(d) => d,
            Err(e) => {
                response.message = format!("Invalid hex: {}", e);
                return Ok(response);
            }
        };

        match self.s7_manager.write_bytes(&request.ip, request.port, request.db_index, request.offset, &data).await {
            Ok(()) => {
                response.success = true;
                response.message = "OK".to_string();
            }
            Err(e) => {
                response.message = e.to_string();
            }
        }

        Ok(response)
    }
}
