//! Токены агентов в хранилище ОС (решение плана 10; образец — Glafira for Apache Kafka).
//! `KeyringStore` — Keychain macOS, Credential Manager Windows, Secret Service Linux;
//! `MemoryStore` — для тестов. Ключ записи — `agents::token_key`.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::sync::MutexExt;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn delete(&self, key: &str) -> AppResult<()>;
}

/// Хранилище ОС; `service` — имя сервиса записей (`product::KEYCHAIN_SERVICE`).
pub struct KeyringStore {
    pub service: &'static str,
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self {
            service: crate::product::KEYCHAIN_SERVICE,
        }
    }
}

fn keyring_err(e: keyring::Error) -> AppError {
    app_err!(
        ErrorCode::Internal,
        "errors.secrets.keychain",
        "Cannot access the system secret store"
    )
    .with_details(e.to_string())
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        let entry = keyring::Entry::new(self.service, key).map_err(keyring_err)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        keyring::Entry::new(self.service, key)
            .and_then(|e| e.set_password(value))
            .map_err(keyring_err)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        let entry = keyring::Entry::new(self.service, key).map_err(keyring_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
    }
}

/// Хранилище в памяти для тестов.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self.values.lock_safe().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.values
            .lock_safe()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        self.values.lock_safe().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryStore::default();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap(), Some("1".to_string()));
        s.delete("a").unwrap();
        s.delete("a").unwrap(); // повторное удаление — не ошибка
        assert_eq!(s.get("a").unwrap(), None);
    }

    #[test]
    fn keyring_store_uses_the_product_service() {
        assert_eq!(
            KeyringStore::default().service,
            crate::product::KEYCHAIN_SERVICE
        );
        let boxed: std::sync::Arc<dyn SecretStore> = std::sync::Arc::new(MemoryStore::default());
        boxed.set("k", "v").unwrap();
        assert_eq!(boxed.get("k").unwrap().as_deref(), Some("v"));
    }
}
