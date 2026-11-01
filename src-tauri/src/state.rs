use std::path::Path;
use std::sync::Arc;

use crate::error::AppResult;
use crate::storage::secrets::{KeyringStore, SecretStore};

/// Общее состояние приложения: политика, настройки, фоновые задачи, хранилище токенов.
/// Клонируется дёшево (всё за `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub policy: Arc<crate::policy::Policy>,
    pub settings: Arc<crate::settings::SettingsStore>,
    /// Идущие разборы сервисов (флаги отмены, `SCAN_BUSY`).
    pub scans: Arc<crate::scan::run::ScanRegistry>,
    /// Токены агентов: хранилище ОС, в тестах — память (`with_secrets`).
    pub secrets: Arc<dyn SecretStore>,
    /// Идущие прогоны: один на проект (`RUN_BUSY`), токены остановки.
    pub runs: Arc<crate::runs::registry::RunRegistry>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> AppResult<Self> {
        Ok(Self {
            policy: Arc::new(crate::policy::Policy::default()),
            settings: Arc::new(crate::settings::SettingsStore::open(data_dir)),
            scans: Arc::default(),
            secrets: Arc::new(KeyringStore::default()),
            runs: Arc::default(),
        })
    }

    /// Политика администратора (читается при запуске в lib.rs).
    pub fn with_policy(mut self, policy: crate::policy::Policy) -> Self {
        self.policy = Arc::new(policy);
        self
    }

    /// Другое хранилище токенов (тесты — `MemoryStore`).
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretStore>) -> Self {
        self.secrets = secrets;
        self
    }
}
