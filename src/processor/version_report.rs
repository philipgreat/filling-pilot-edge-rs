//! Version report processor

use crate::context::Context;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionReport {
    pub version: String,
    pub os: String,
    pub architecture: String,
}

impl VersionReport {
    pub fn new(context: &Context) -> Self {
        Self {
            version: context.version.clone(),
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
        }
    }
}

pub struct VersionReportProcessor {
    context: Context,
}

impl VersionReportProcessor {
    pub fn new(context: Context) -> Self { Self { context } }
    pub fn get_report(&self) -> VersionReport { VersionReport::new(&self.context) }
}
