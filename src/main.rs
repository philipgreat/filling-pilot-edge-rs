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

use context::Context;
use s7::S7Manager;
use processor::{StatusProcessor, VersionReportProcessor};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .with(tracing_subscriber::filter::EnvFilter::from_default_env()
            .add_directive(Level::INFO.into()))
        .init();

    info!("Starting Filling Pilot Edge v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let context = match Context::load() {
        Ok(ctx) => {
            info!("ECN: {}", ctx.id);
            ctx
        }
        Err(e) => {
            error!("Config error: {}", e);
            eprintln!("Error: Please ensure 'id' and 'serverConf' files exist");
            std::process::exit(1);
        }
    };

    // Initialize components
    let s7_manager = Arc::new(S7Manager::new());
    let status_cache = Arc::new(processor::status::StatusCache::new());
    let status_processor = StatusProcessor::new(Arc::clone(&status_cache));
    let version_processor = VersionReportProcessor::new(context.clone());

    let version_report = version_processor.get_report();
    info!("Version: {} | OS: {} | Arch: {}", 
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

    // Wait for shutdown
    tokio::signal::ctrl_c().await?;
    
    info!("Shutting down...");
    s7_manager.close_all().await;
    http_handle.abort();

    Ok(())
}
