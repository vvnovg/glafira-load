//! Идущие разборы и их флаги отмены. Одновременно в проекте идёт не больше одного разбора:
//! слияние читает и переписывает общий `registry/operations.yaml`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{AppResult, ErrorCode};
use crate::sync::MutexExt;

/// Идущие разборы: ключ `<проект>|<сервис>`, значение — флаг отмены.
#[derive(Default)]
pub struct ScanRegistry {
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// Проект из ключа: `id` сервиса не содержит `|`, поэтому разделитель — последний.
fn project_of(key: &str) -> &str {
    key.rsplit_once('|').map(|(p, _)| p).unwrap_or(key)
}

impl ScanRegistry {
    /// `project` — канонический корень проекта (`ProjectDir::root`), чтобы один проект,
    /// открытый по разным написаниям пути, давал один ключ.
    pub fn key(project: &str, service: &str) -> String {
        format!("{project}|{service}")
    }

    /// Регистрирует разбор; уже идёт разбор любого сервиса этого проекта — `SCAN_BUSY`.
    pub fn start(&self, key: &str) -> AppResult<Arc<AtomicBool>> {
        let mut r = self.running.lock_safe();
        let project = project_of(key);
        if r.keys().any(|k| project_of(k) == project) {
            return Err(app_err!(
                ErrorCode::ScanBusy,
                "errors.scan.busy",
                "This service is already being scanned"
            ));
        }
        let flag = Arc::new(AtomicBool::new(false));
        r.insert(key.into(), flag.clone());
        Ok(flag)
    }

    pub fn finish(&self, key: &str) {
        self.running.lock_safe().remove(key);
    }

    /// Ставит флаг отмены; `false` — такого разбора нет.
    pub fn cancel(&self, key: &str) -> bool {
        self.running
            .lock_safe()
            .get(key)
            .map(|f| f.store(true, Ordering::Relaxed))
            .is_some()
    }

    /// Отменяет все разборы проекта; возвращает их число.
    pub fn cancel_all(&self, project: &str) -> usize {
        self.running
            .lock_safe()
            .iter()
            .filter(|(k, _)| project_of(k) == project)
            .map(|(_, f)| f.store(true, Ordering::Relaxed))
            .count()
    }

    /// Регистрирует разбор и снимает регистрацию при выходе из области видимости — в том числе
    /// при ошибке или панике между `start` и `finish`.
    pub fn guard(self: &Arc<Self>, key: String) -> AppResult<Running> {
        let flag = self.start(&key)?;
        Ok(Running {
            registry: self.clone(),
            key,
            flag,
        })
    }
}

/// Зарегистрированный разбор; `Drop` вызывает `finish`.
pub struct Running {
    registry: Arc<ScanRegistry>,
    key: String,
    flag: Arc<AtomicBool>,
}

impl Running {
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.registry.finish(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_and_busy() {
        let r = ScanRegistry::default();
        let k = ScanRegistry::key("/p", "s");
        let flag = r.start(&k).unwrap();
        assert_eq!(r.start(&k).unwrap_err().code, ErrorCode::ScanBusy);
        assert!(r.cancel(&k) && flag.load(Ordering::Relaxed));
        r.finish(&k);
        assert!(!r.cancel(&k));
    }

    #[test]
    fn one_scan_per_project_and_cancel_all() {
        let r = ScanRegistry::default();
        let a = r.start(&ScanRegistry::key("/p", "a")).unwrap();
        assert_eq!(
            r.start(&ScanRegistry::key("/p", "b")).unwrap_err().code,
            ErrorCode::ScanBusy,
            "другой сервис того же проекта"
        );
        let other = r.start(&ScanRegistry::key("/p2", "a")).unwrap();
        assert_eq!(r.cancel_all("/p"), 1, "префикс /p не задевает /p2");
        assert!(a.load(Ordering::Relaxed) && !other.load(Ordering::Relaxed));
    }

    #[test]
    fn guard_finishes_on_drop() {
        let r = Arc::new(ScanRegistry::default());
        let k = ScanRegistry::key("/p", "s");
        {
            let g = r.guard(k.clone()).unwrap();
            assert!(r.cancel(&k) && g.flag().load(Ordering::Relaxed));
            assert!(r.guard(k.clone()).is_err());
        }
        assert!(!r.cancel(&k), "снят при выходе");
        assert!(r.guard(k).is_ok());
    }
}
