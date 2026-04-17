//! Filling Pilot Edge - Rust Implementation
//! 
//! Industrial IoT edge computing for Siemens PLC data collection.
//! Communicates with cloud server via gRPC and provides local HTTP config API.

pub mod context;
pub mod codec;
pub mod processor;
pub mod grpc;
pub mod s7;
pub mod http;
pub mod error;
pub mod logger;
pub mod report;

pub use context::Context;
pub use error::{Error, Result};
