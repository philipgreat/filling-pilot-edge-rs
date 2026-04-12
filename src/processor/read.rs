//! Read processor

use std::sync::Arc;
use tracing::info;

use crate::s7::{ReadRequest, ReadResponse, S7Manager};

pub struct ReadProcessor {
    s7_manager: Arc<S7Manager>,
}

impl ReadProcessor {
    pub fn new(s7_manager: Arc<S7Manager>) -> Self {
        Self { s7_manager }
    }

    pub async fn handle(&self, request: ReadRequest) -> Result<ReadResponse, crate::error::Error> {
        info!("Read: PLC={}, IP={}:{}, DB={}, Offset={}, Size={}",
            request.plc_id, request.ip, request.port, request.db_index, request.offset, request.size);

        let mut response = ReadResponse {
            request_id: request.request_id,
            plc_id: request.plc_id.clone(),
            ip: request.ip.clone(),
            port: request.port,
            db_index: request.db_index,
            offset: request.offset,
            success: false,
            message: String::new(),
            hex_content: None,
        };

        match self.s7_manager.read_bytes(&request.ip, request.port, request.db_index, request.offset, request.size).await {
            Ok(data) => {
                response.success = true;
                response.message = "OK".to_string();
                response.hex_content = Some(hex::encode(&data));
                info!("Read {} bytes", data.len());
            }
            Err(e) => {
                response.message = e.to_string();
            }
        }

        Ok(response)
    }
}
