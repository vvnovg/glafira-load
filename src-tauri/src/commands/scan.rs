//! Команды разбора: автопоиск контрактов, разбор сервисов с прогрессом, отмена.

use glafira_load_model::project::ContractRef;
use tauri::State;
use tauri::ipc::Channel;

use super::blocking;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::git::version::GitVersion;
use crate::project::manifest::ProjectDir;
use crate::scan::discover::discover;
use crate::scan::run::{ScanProgress, ScanRegistry, ScanReport, scan_service_blocking};
use crate::state::AppState;

#[tauri::command]
pub async fn service_discover_contracts(
    path: String,
    service_id: String,
) -> AppResult<Vec<ContractRef>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let m = d.manifest()?;
        Ok(discover(&d.service_root(&m, &service_id)?))
    })
    .await
}

/// Канонический корень проекта — ключ реестра разборов (один проект, разные написания пути).
fn project_key(path: &str) -> AppResult<String> {
    Ok(ProjectDir::open(path)?
        .root()
        .to_string_lossy()
        .into_owned())
}

/// Разбор одного сервиса: регистрация (`SCAN_BUSY`), версия git, разбор и слияние.
async fn scan_one(
    st: &AppState,
    root: &str,
    id: String,
    on_progress: &Channel<ScanProgress>,
) -> AppResult<ScanReport> {
    // Снимается при выходе, в том числе по ошибке.
    let running = st.scans.guard(ScanRegistry::key(root, &id))?;
    let (r, sid) = (root.to_string(), id.clone());
    let clone = blocking(move || {
        let d = ProjectDir::open(&r)?;
        let m = d.manifest()?;
        Ok(d.service_root(&m, &sid).ok())
    })
    .await?;
    // Нет папки клона — разбор сам вернёт предупреждение и не тронет реестр.
    let git = match clone {
        Some(c) => crate::git::version::describe(&c).await,
        None => GitVersion::unversioned("service folder not found"),
    };
    let (r, ch, cancel) = (root.to_string(), on_progress.clone(), running.flag());
    blocking(move || {
        let d = ProjectDir::open(&r)?;
        let progress = move |pr: ScanProgress| {
            let _ = ch.send(pr);
        };
        scan_service_blocking(&d, &id, git, &progress, &cancel)
    })
    .await
}

/// Отчёт о сервисе, разбор которого завершился ошибкой (при «Разобрать все»): ошибка — в
/// `errors` и заметной строкой; реестр этого сервиса не менялся.
pub fn failed_report(service: &str, e: &AppError) -> ScanReport {
    ScanReport {
        service: service.into(),
        git: GitVersion::unversioned(""),
        contracts: 0,
        operations_found: 0,
        tables_found: 0,
        summary: Default::default(),
        warnings: Vec::new(),
        errors: vec![glafira_load_contracts::Diagnostic {
            path: String::new(),
            line: None,
            message: e.message.clone(),
        }],
        scan_file: String::new(),
        cancelled: false,
        notice: Some(e.text()),
    }
}

/// Разбор одного сервиса или всех (`service_id = None`), по очереди; прогресс — в канал.
/// Отмена останавливает и текущий сервис, и следующие. При разборе всех ошибка одного сервиса
/// не теряет отчёты уже разобранных: она становится отчётом этого сервиса, разбор идёт дальше
/// (`SCAN_BUSY` — другой разбор проекта — останавливает очередь).
#[tauri::command]
pub async fn service_scan(
    state: State<'_, AppState>,
    path: String,
    service_id: Option<String>,
    on_progress: Channel<ScanProgress>,
) -> AppResult<Vec<ScanReport>> {
    let st = state.inner().clone();
    let all = service_id.is_none();
    let (root, ids) = blocking(move || {
        let d = ProjectDir::open(&path)?;
        let ids = match service_id {
            Some(id) => vec![id],
            None => d
                .manifest()?
                .services
                .iter()
                .map(|s| s.id.clone())
                .collect(),
        };
        Ok((d.root().to_string_lossy().into_owned(), ids))
    })
    .await?;
    collect_reports(all, ids, |id| scan_one(&st, &root, id, &on_progress)).await
}

/// Очередь разборов: один сервис — ошибка как есть; все — см. `service_scan`.
async fn collect_reports<F, Fut>(
    all: bool,
    ids: Vec<String>,
    mut scan: F,
) -> AppResult<Vec<ScanReport>>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = AppResult<ScanReport>>,
{
    let mut reports = Vec::new();
    for id in ids {
        match scan(id.clone()).await {
            Ok(report) => {
                let stop = report.cancelled;
                reports.push(report);
                if stop {
                    break;
                }
            }
            Err(e) if !all || (reports.is_empty() && e.code == ErrorCode::ScanBusy) => {
                return Err(e);
            }
            Err(e) => {
                let busy = e.code == ErrorCode::ScanBusy;
                reports.push(failed_report(&id, &e));
                if busy {
                    break;
                }
            }
        }
    }
    Ok(reports)
}

/// Ставит флаг отмены; разбора нет — ничего не делает.
#[tauri::command]
pub async fn scan_cancel(
    state: State<'_, AppState>,
    path: String,
    service_id: Option<String>,
) -> AppResult<()> {
    let st = state.inner().clone();
    blocking(move || {
        // Проекта уже нет на диске — ключ по пути как есть (так он и не найдётся).
        let root = project_key(&path).unwrap_or(path);
        match service_id {
            Some(id) => {
                st.scans.cancel(&ScanRegistry::key(&root, &id));
            }
            None => {
                st.scans.cancel_all(&root);
            }
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(id: &str) -> ScanReport {
        let mut r = failed_report(id, &AppError::internal_with("x"));
        r.errors.clear();
        r.notice = None;
        r
    }

    fn broken() -> AppError {
        app_err!(
            ErrorCode::PathOutsideProject,
            "errors.project.outside",
            "Path {{path}} is outside {{base}}",
            path = "x",
            base = "/p"
        )
    }

    #[tokio::test]
    async fn scan_all_keeps_earlier_reports_and_reports_the_failed_service() {
        let ids = vec!["a".to_string(), "b".into(), "c".into()];
        let reports = collect_reports(true, ids, |id| async move {
            if id == "b" {
                Err(broken())
            } else {
                Ok(ok(&id))
            }
        })
        .await
        .unwrap();
        let services: Vec<&str> = reports.iter().map(|r| r.service.as_str()).collect();
        assert_eq!(services, ["a", "b", "c"]);
        assert_eq!(reports[1].errors.len(), 1);
        assert!(reports[1].errors[0].message.contains("outside"));
        assert_eq!(
            reports[1].notice.as_ref().unwrap().key,
            "errors.project.outside"
        );
        assert!(reports[0].errors.is_empty() && reports[2].errors.is_empty());
    }

    #[tokio::test]
    async fn single_service_error_and_busy_project_are_returned_as_errors() {
        let e = collect_reports(false, vec!["b".into()], |_| async { Err(broken()) })
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
        let busy = || {
            app_err!(
                ErrorCode::ScanBusy,
                "errors.scan.busy",
                "A scan is already running in this project"
            )
        };
        let e = collect_reports(true, vec!["a".into(), "b".into()], |_| async move {
            Err(busy())
        })
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::ScanBusy);
        // Занято посреди очереди — уже разобранные отчёты остаются, очередь останавливается.
        let reports = collect_reports(
            true,
            vec!["a".into(), "b".into(), "c".into()],
            |id| async move { if id == "a" { Ok(ok(&id)) } else { Err(busy()) } },
        )
        .await
        .unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[1].notice.as_ref().unwrap().key, "errors.scan.busy");
    }
}
