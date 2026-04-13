//! Filling Pilot Edge - Rust Implementation
//! 
//! Industrial IoT edge computing for Siemens PLC data collection.

use std::sync::Arc;
use tracing::{info, error, Level};
use tracing_subscriber::prelude::*;

mod context;
mod codec;
mod processor;
mod grpc;
mod s7;
mod http;
mod error;
mod logger;

use context::Context;
use s7::S7Manager;
use processor::{StatusProcessor, VersionReportProcessor};
use logger::{Logger, start_memory_reporter};
use grpc::CloudSession;
use tokio::time::{interval, Duration};

/// Global logger instance (initialized after context is loaded)
static LOGGER: std::sync::OnceLock<Arc<Logger>> = std::sync::OnceLock::new();

/// Log a message to both stdout and remote UDP server
/// 
/// Format: "[{id}] [{date}] {name}: {msg}"
macro_rules! log_udp {
    ($name:expr, $($arg:tt)*) => {{
        if let Some(l) = LOGGER.get() {
            let msg = format!($($arg)*);
            let l = Arc::clone(l);
            tokio::spawn(async move { l.log($name, &msg).await; });
        }
    }};
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .with(tracing_subscriber::filter::EnvFilter::from_default_env()
            .add_directive(Level::INFO.into()))
        .init();

    info!("Starting Filling Pilot Edge v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let (context, cwd, id_path, conf_path, id_content, conf_content) = match Context::load_with_paths() {
        Ok(result) => {
            let (ctx, cwd, id_path, conf_path, id_content, conf_content) = result;
            info!("Config directory: {}", cwd);
            info!("Loaded id file: {}", id_path);
            info!("{}", id_content.trim());
            info!("Loaded serverConf file: {}", conf_path);
            info!("{}", conf_content.trim());
            info!("ECN: {}", ctx.id);
            (ctx, cwd, id_path, conf_path, id_content, conf_content)
        }
        Err(e) => {
            error!("{}", e);
            std::process::exit(1);
        }
    };

    // Initialize UDP logger
    let logger = Arc::new(Logger::new(context.id.clone()));
    let _ = LOGGER.set(logger.clone());

    // Start periodic memory reporter (every 20 seconds)
    start_memory_reporter(Arc::clone(&logger), 20);

    // Log startup
    log_udp!("STARTUP", "Filling Pilot Edge v{} started", env!("CARGO_PKG_VERSION"));
    log_udp!("STARTUP", "ECN: {}", context.id);

    // Initialize components
    let s7_manager = Arc::new(S7Manager::new());
    let status_cache = Arc::new(processor::status::StatusCache::new());
    let status_processor = StatusProcessor::new(Arc::clone(&status_cache));
    let version_processor = VersionReportProcessor::new(context.clone());

    let version_report = version_processor.get_report();
    info!("Version: {} | OS: {} | Arch: {}", 
        version_report.version, version_report.os, version_report.architecture);
    log_udp!("STARTUP", "Version: {} | OS: {} | Arch: {}", 
        version_report.version, version_report.os, version_report.architecture);

    // Start HTTP server
    let http_ctx = context.clone();
    let http_s7 = Arc::clone(&s7_manager);
    let http_handle = tokio::spawn(async move {
        if let Err(e) = http::start_server(http_ctx, http_s7, context.local_port).await {
            error!("HTTP error: {}", e);
        }
    });

    info!("Filling Pilot Edge started");
    info!("HTTP API: http://0.0.0.0:{}", context.local_port);
    info!("Cloud Server: {}:{}", context.server_address, context.port);
    log_udp!("STARTUP", "HTTP API: http://0.0.0.0:{}", context.local_port);
    log_udp!("STARTUP", "Cloud Server: {}:{}", context.server_address, context.port);

    // Create cloud session and connect
    let cloud_session = Arc::new(CloudSession::new(&context, Arc::clone(&logger)));
    
    // Connect to cloud server
    match cloud_session.connect().await {
        Ok(()) => {
            log_udp!("STARTUP", "Cloud connection established");
        }
        Err(e) => {
            log_udp!("STARTUP", "Cloud connection failed: {}", e);
        }
    }

    // Spawn heartbeat loop
    let heartbeat_session = Arc::clone(&cloud_session);
    let heartbeat_handle = tokio::spawn(async move {
        heartbeat_session.start_heartbeat_loop().await;
    });

    // Spawn register (will block on command stream, so run in background)
    let register_session = Arc::clone(&cloud_session);
    let register_handle = tokio::spawn(async move {
        register_session.register().await;
    });

    // Spawn PLC connection status checker and reporter
    let plc_s7 = Arc::clone(&s7_manager);
    let plc_cloud = Arc::clone(&cloud_session);
    let plc_logger = Arc::clone(&logger);
    let plc_status_handle = tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(10)); // Check every 10 seconds
        loop {
            tick.tick().await;
            
            let statuses = plc_s7.check_all_connections().await;
            if statuses.is_empty() {
                // No PLCs configured yet, skip
                continue;
            }
            
            for status in statuses {
                let status_str = if status.connected {
                    format!("PLC {}:{} CONNECTED (latency: {:?}ms)", 
                        status.host, status.port, status.latency_ms)
                } else {
                    format!("PLC {}:{} DISCONNECTED", status.host, status.port)
                };
                info!("[PLC_STATUS] {}", status_str);
                
                // Log locally
                let l = Arc::clone(&plc_logger);
                let msg = status_str.clone();
                tokio::spawn(async move { l.log("PLC_STATUS", &msg).await; });
                
                // Report to cloud server
                let cloud = Arc::clone(&plc_cloud);
                let status_clone = status.clone();
                tokio::spawn(async move {
                    cloud.send_plc_status(&status_clone).await;
                });
            }
        }
    });

    // Wait for shutdown
    tokio::signal::ctrl_c().await?;
    
    log_udp!("SHUTDOWN", "Filling Pilot Edge shutting down");
    info!("Shutting down...");
    s7_manager.close_all().await;
    http_handle.abort();
    heartbeat_handle.abort();
    register_handle.abort();
    plc_status_handle.abort();

    Ok(())
}
