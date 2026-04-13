//! Logger with UDP remote logging capability
//! 
//! Sends log messages to iotlog.doublechaintech.com:54321 via UDP.
//! Each byte is encoded as (255 - byte[i]) before transmission.

use std::net::ToSocketAddrs;
use chrono::Local;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use std::sync::Arc;

/// Remote log destination
const LOG_HOST: &str = "iotlog.doublechaintech.com";
const LOG_PORT: u16 = 54321;

/// Shared logger state
pub struct Logger {
    /// Edge node ID for log prefix
    id: String,
    /// Cached resolved socket address
    addr: Arc<RwLock<Option<std::net::SocketAddr>>>,
}

impl Logger {
    /// Create a new logger with the given edge node ID
    pub fn new(id: String) -> Self {
        Self {
            id,
            addr: Arc::new(RwLock::new(None)),
        }
    }

    /// Resolve the remote log host (cached)
    async fn resolve_addr(&self) -> Option<std::net::SocketAddr> {
        // Fast path: already resolved
        if let Some(addr) = self.addr.read().await.as_ref() {
            return Some(*addr);
        }

        // Slow path: resolve DNS
        let resolved: Option<std::net::SocketAddr> = format!("{}:{}", LOG_HOST, LOG_PORT)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next());

        if let Some(addr) = resolved {
            *self.addr.write().await = Some(addr);
        }

        resolved
    }

    /// Send a log message to the remote UDP server
    /// 
    /// Format: "[{id}] [{date}] {name}: {msg}"
    /// Encoding: each byte = 255 - original_byte
    pub async fn log(&self, name: &str, msg: &str) {
        let date = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_line = format!("[{}] [{}] {}: {}", self.id, date, name, msg);

        // Also print to stdout
        println!("{}", log_line);

        // Resolve address
        let addr = match self.resolve_addr().await {
            Some(a) => a,
            None => return,
        };

        // Encode: 255 - byte[i]
        let bytes: Vec<u8> = log_line.bytes().map(|b| 255u8.wrapping_sub(b)).collect();

        // Send via UDP
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(_) => return,
        };

        if let Err(_) = socket.send_to(&bytes, addr).await {
            // Silently ignore send errors (log should not crash the app)
        }
    }
}

/// Start the periodic memory stat reporter
/// 
/// Runs in a background tokio task, reporting memory usage every `interval_secs` seconds.
/// 
/// Format: "used/total(k): {used}/{total}"
pub fn start_memory_reporter(logger: Arc<Logger>, interval_secs: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            let mem = sys_info::mem_info().ok();
            if let Some(mem) = mem {
                let used_kb = mem.total - mem.free;
                let total_kb = mem.total;
                logger.log(
                    "MEM",
                    &format!("used/total(k): {}/{}", used_kb, total_kb)
                ).await;
            }
        }
    });
}
