//! Upgrade processor

pub struct UpgradeProcessor {
    upgrade_url: Option<String>,
}

impl UpgradeProcessor {
    pub fn new(upgrade_url: Option<String>) -> Self { Self { upgrade_url } }
    pub fn handle(&self, _url: Option<String>) -> Result<(), crate::error::Error> {
        Ok(())
    }
}
