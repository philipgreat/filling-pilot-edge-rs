//! Restart processor

use tracing::info;

pub struct RestartProcessor;

impl RestartProcessor {
    pub fn new() -> Self { Self }
    
    pub fn handle(&self) -> Result<(), crate::error::Error> {
        info!("Restarting...");
        std::process::exit(0);
    }
}

impl Default for RestartProcessor {
    fn default() -> Self { Self::new() }
}
