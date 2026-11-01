//! Идущие прогоны: один на проект (`RUN_BUSY`), токен остановки. Образец — `scan::registry`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::sync::MutexExt;

struct Running {
    cancel: CancellationToken,
    /// Известен после события `Started` контроллера.
    run_id: Option<String>,
}

/// Идущие прогоны: ключ — канонический корень проекта (`ProjectDir::root`).
#[derive(Default)]
pub struct RunRegistry {
    running: Mutex<HashMap<String, Running>>,
}

pub fn busy() -> AppError {
    app_err!(
        ErrorCode::RunBusy,
        "errors.run.busy",
        "A run is already in progress in this project"
    )
}

impl RunRegistry {
    /// Регистрирует прогон; в проекте уже идёт прогон — `RUN_BUSY`.
    pub fn start(&self, project: &str) -> AppResult<CancellationToken> {
        let mut r = self.running.lock_safe();
        if r.contains_key(project) {
            return Err(busy());
        }
        let cancel = CancellationToken::new();
        r.insert(
            project.to_string(),
            Running {
                cancel: cancel.clone(),
                run_id: None,
            },
        );
        Ok(cancel)
    }

    pub fn finish(&self, project: &str) {
        self.running.lock_safe().remove(project);
    }

    pub fn set_run_id(&self, project: &str, run_id: &str) {
        if let Some(r) = self.running.lock_safe().get_mut(project) {
            r.run_id = Some(run_id.to_string());
        }
    }

    /// `None` — прогона нет; `Some(None)` — идёт, но `run_id` ещё не известен.
    pub fn current(&self, project: &str) -> Option<Option<String>> {
        self.running
            .lock_safe()
            .get(project)
            .map(|r| r.run_id.clone())
    }

    /// Запрашивает остановку; `false` — прогона нет.
    pub fn stop(&self, project: &str) -> bool {
        self.running
            .lock_safe()
            .get(project)
            .map(|r| r.cancel.cancel())
            .is_some()
    }

    /// Останавливает все прогоны; возвращает их число.
    pub fn stop_all(&self) -> usize {
        let r = self.running.lock_safe();
        for v in r.values() {
            v.cancel.cancel();
        }
        r.len()
    }

    pub fn is_busy(&self) -> bool {
        !self.running.lock_safe().is_empty()
    }

    /// Регистрирует прогон и снимает регистрацию при выходе из области видимости — в том числе
    /// при ошибке или панике между `start` и `finish`.
    pub fn guard(self: &Arc<Self>, project: String) -> AppResult<RunGuard> {
        let cancel = self.start(&project)?;
        Ok(RunGuard {
            registry: self.clone(),
            project,
            cancel,
        })
    }
}

/// Зарегистрированный прогон; `Drop` вызывает `finish`.
pub struct RunGuard {
    registry: Arc<RunRegistry>,
    project: String,
    cancel: CancellationToken,
}

impl RunGuard {
    pub fn cancel(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn project(&self) -> &str {
        &self.project
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.registry.finish(&self.project);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_run_per_project_and_stop() {
        let r = RunRegistry::default();
        let cancel = r.start("/p").unwrap();
        assert_eq!(r.start("/p").unwrap_err().code, ErrorCode::RunBusy);
        assert!(r.start("/q").is_ok(), "другой проект не мешает");
        assert_eq!(r.current("/p"), Some(None));
        r.set_run_id("/p", "r1");
        assert_eq!(r.current("/p"), Some(Some("r1".to_string())));
        assert!(r.stop("/p") && cancel.is_cancelled());
        assert!(!r.stop("/none"));
        assert_eq!(r.stop_all(), 2);
        r.finish("/p");
        assert_eq!(r.current("/p"), None);
        assert!(r.is_busy(), "/q ещё идёт");
    }

    #[test]
    fn guard_finishes_on_drop() {
        let r = Arc::new(RunRegistry::default());
        {
            let g = r.guard("/p".into()).unwrap();
            assert_eq!(g.project(), "/p");
            assert!(r.guard("/p".into()).is_err());
            assert!(r.stop("/p") && g.cancel().is_cancelled());
        }
        assert!(!r.is_busy(), "снят при выходе");
        assert!(r.guard("/p".into()).is_ok());
    }
}
