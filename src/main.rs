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

    // Wait for shutdown
    tokio::signal::ctrl_c().await?;
    
    log_udp!("SHUTDOWN", "Filling Pilot Edge shutting down");
    info!("Shutting down...");
    s7_manager.close_all().await;
    http_handle.abort();

    Ok(())
}
