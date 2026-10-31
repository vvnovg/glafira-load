use std::path::Path;
use std::sync::Arc;

use crate::error::AppResult;

/// Общее состояние приложения: политика и настройки. Клонируется дёшево (всё за `Arc`).
/// Реестр идущих разборов добавляется в Task 13.
#[derive(Clone)]
pub struct AppState {
    pub policy: Arc<crate::policy::Policy>,
    pub settings: Arc<crate::settings::SettingsStore>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> AppResult<Self> {
        Ok(Self {
            policy: Arc::new(crate::policy::Policy::default()),
            settings: Arc::new(crate::settings::SettingsStore::open(data_dir)),
        })
    }

    /// Политика администратора (читается при запуске в lib.rs).
    pub fn with_policy(mut self, policy: crate::policy::Policy) -> Self {
        self.policy = Arc::new(policy);
        self
    }
}
