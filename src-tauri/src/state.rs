use std::path::Path;
use std::sync::Arc;

use crate::error::AppResult;

/// Общее состояние приложения: политика и настройки. Клонируется дёшево (всё за `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub policy: Arc<crate::policy::Policy>,
    pub settings: Arc<crate::settings::SettingsStore>,
    /// Идущие разборы сервисов (флаги отмены, `SCAN_BUSY`).
    pub scans: Arc<crate::scan::run::ScanRegistry>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> AppResult<Self> {
        Ok(Self {
            policy: Arc::new(crate::policy::Policy::default()),
            settings: Arc::new(crate::settings::SettingsStore::open(data_dir)),
            scans: Arc::default(),
        })
    }

    /// Политика администратора (читается при запуске в lib.rs).
    pub fn with_policy(mut self, policy: crate::policy::Policy) -> Self {
        self.policy = Arc::new(policy);
        self
    }
}
