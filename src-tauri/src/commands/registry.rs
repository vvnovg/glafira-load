//! Команды реестра операций: список с фильтром, карточка, ручные поля и операции, снимки, таблицы.

use std::collections::BTreeMap;

use glafira_load_contracts::Table;
use glafira_load_model::diff::{self, RegistryDiff, Scan};
use glafira_load_model::from_yaml;
use glafira_load_model::registry::{
    Manual, OpKind, Operation, OperationDraft, SCANS_DIR, Seen, Source, SourceKind, Status,
    address_label, operation_id,
};
use serde::{Deserialize, Serialize};

use super::blocking;
use crate::error::{AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::project::paths::inside;
use crate::scan::run::{TABLES_DIR, now_rfc3339};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryFilter {
    pub service: Option<String>,
    pub kind: Option<OpKind>,
    pub source: Option<SourceKind>,
    pub status: Option<Status>,
    /// Подстрока `id` или имени без учёта регистра.
    pub query: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRow {
    pub id: String,
    pub service: String,
    pub kind: OpKind,
    pub name: String,
    pub summary: String,
    pub source: SourceKind,
    pub status: Status,
    pub last_version: String,
    pub topic_guessed: bool,
}

fn row(o: &Operation) -> OperationRow {
    OperationRow {
        id: o.draft.id.clone(),
        service: o.draft.service.clone(),
        kind: o.draft.kind,
        name: o.draft.name.clone(),
        summary: o.draft.summary.clone(),
        source: o.draft.source.kind,
        status: o.status,
        last_version: o.last_seen.version.clone(),
        topic_guessed: o.draft.topic_guessed,
    }
}

fn matches(o: &Operation, f: &RegistryFilter) -> bool {
    f.service.as_ref().is_none_or(|s| &o.draft.service == s)
        && f.kind.is_none_or(|k| o.draft.kind == k)
        && f.source.is_none_or(|s| o.draft.source.kind == s)
        && f.status.is_none_or(|s| o.status == s)
        && f.query
            .as_ref()
            .filter(|q| !q.trim().is_empty())
            .is_none_or(|q| {
                let q = q.trim().to_lowercase();
                o.draft.id.to_lowercase().contains(&q) || o.draft.name.to_lowercase().contains(&q)
            })
}

fn not_found(id: &str) -> crate::error::AppError {
    app_err!(
        ErrorCode::NotFound,
        "errors.registry.notFound",
        "Operation {{id}} not found",
        id = id
    )
}

#[tauri::command]
pub async fn registry_list(path: String, filter: RegistryFilter) -> AppResult<Vec<OperationRow>> {
    blocking(move || {
        Ok(ProjectDir::open(&path)?
            .registry()?
            .operations
            .iter()
            .filter(|o| matches(o, &filter))
            .map(row)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn registry_get(path: String, id: String) -> AppResult<Operation> {
    blocking(move || {
        ProjectDir::open(&path)?
            .registry()?
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(&id))
    })
    .await
}

/// Ручные поля (теги, заметки, владелец) переживают переразбор: слияние меняет только черновик.
#[tauri::command]
pub async fn registry_update_manual(
    path: String,
    id: String,
    manual: Manual,
) -> AppResult<Operation> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        d.with_registry_lock(|| {
            let mut r = d.registry()?;
            let op = r.get_mut(&id).ok_or_else(|| not_found(&id))?;
            op.manual = manual;
            let out = op.clone();
            d.save_registry(&r)?;
            Ok(out)
        })
    })
    .await
}

/// Ручная операция: источник `manual`, `id` по правилам, версия — момент добавления.
#[tauri::command]
pub async fn registry_add_manual(path: String, draft: OperationDraft) -> AppResult<Operation> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let m = d.manifest()?;
        if m.service(&draft.service).is_none() {
            return Err(app_err!(
                ErrorCode::InvalidInput,
                "errors.project.noService",
                "Service {{id}} is not in the manifest",
                id = draft.service
            ));
        }
        let mut draft = draft;
        draft.id = operation_id(&draft.service, draft.kind, &draft.address);
        draft.source = Source::manual();
        if draft.name.trim().is_empty() {
            draft.name = address_label(&draft.address);
        }
        d.with_registry_lock(|| {
            let mut r = d.registry()?;
            if r.get(&draft.id).is_some() {
                return Err(app_err!(
                    ErrorCode::InvalidInput,
                    "errors.registry.duplicate",
                    "Operation {{id}} already exists",
                    id = draft.id
                ));
            }
            let seen = Seen {
                version: "manual".into(),
                commit: String::new(),
                at: now_rfc3339(),
            };
            let op = Operation {
                fingerprint: draft.fingerprint(),
                draft,
                status: Status::Active,
                first_seen: seen.clone(),
                last_seen: seen,
                manual: Manual::default(),
            };
            r.operations.push(op.clone());
            r.sort();
            d.save_registry(&r)?;
            Ok(op)
        })
    })
    .await
}

#[tauri::command]
pub async fn registry_delete(path: String, id: String) -> AppResult<()> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        d.with_registry_lock(|| {
            let mut r = d.registry()?;
            if !r.remove(&id) {
                return Err(not_found(&id));
            }
            d.save_registry(&r)
        })
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanMeta {
    pub file: String,
    pub at: String,
    pub service: String,
    pub version: String,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    pub restored: usize,
}

/// Снимок из `registry/scans/`; другой файл проекта или путь вне каталога снимков —
/// `PATH_OUTSIDE_PROJECT`.
fn read_scan(d: &ProjectDir, file: &str) -> AppResult<Scan> {
    let not_found = || {
        app_err!(
            ErrorCode::NotFound,
            "errors.registry.scanNotFound",
            "Scan {{file}} not found",
            file = file
        )
    };
    // Каталога снимков ещё нет (ни одного разбора) — снимка нет, а не сбой канонизации пути.
    if !d.file(SCANS_DIR).is_dir() {
        return Err(not_found());
    }
    inside(&d.file(SCANS_DIR), &d.file(file))?;
    let text = d.read_text(file)?.ok_or_else(not_found)?;
    from_yaml(&text).map_err(|e| {
        app_err!(
            ErrorCode::ProjectInvalid,
            "errors.project.invalidFile",
            "{{file}}: {{error}}",
            file = file,
            error = e.message
        )
    })
}

/// Снимки от новых к старым: по времени разбора, при равенстве — по имени файла.
#[tauri::command]
pub async fn registry_scans(path: String) -> AppResult<Vec<ScanMeta>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let files: Vec<String> = match std::fs::read_dir(d.file(SCANS_DIR)) {
            Ok(rd) => rd
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()
                        .filter(|n| n.ends_with(".yaml"))
                        .map(|n| format!("{SCANS_DIR}/{n}"))
                })
                .collect(),
            Err(_) => vec![],
        };
        let mut out = Vec::new();
        for file in files {
            match read_scan(&d, &file) {
                Ok(s) => out.push(ScanMeta {
                    file,
                    at: s.at,
                    service: s.service,
                    version: s.version,
                    added: s.summary.added.len(),
                    changed: s.summary.changed.len(),
                    removed: s.summary.removed.len(),
                    restored: s.summary.restored.len(),
                }),
                Err(e) => tracing::warn!("scan {file} skipped: {}", e.message),
            }
        }
        out.sort_by(|a, b| (&b.at, &b.file).cmp(&(&a.at, &a.file)));
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn registry_diff(path: String, from: String, to: String) -> AppResult<RegistryDiff> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        // Имя команды совпадает с функцией модели — вызываем её через модуль.
        Ok(diff::registry_diff(
            &read_scan(&d, &from)?,
            &read_scan(&d, &to)?,
        ))
    })
    .await
}

/// Таблицы из миграций по сервисам (подсказки для ручных `sql`-операций).
#[tauri::command]
pub async fn registry_tables(path: String) -> AppResult<BTreeMap<String, Vec<Table>>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let mut out = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(d.file(TABLES_DIR)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(service) = name.strip_suffix(".yaml") else {
                    continue;
                };
                if let Some(text) = d.read_text(&format!("{TABLES_DIR}/{name}"))?
                    && let Ok(t) = from_yaml::<Vec<Table>>(&text)
                {
                    out.insert(service.to_string(), t);
                }
            }
        }
        Ok(out)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::version::GitVersion;
    use crate::project::manifest::create;
    use crate::scan::run::scan_service_blocking;
    use glafira_load_model::project::{Manifest, Service};
    use glafira_load_model::registry::Address;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    async fn project_at(service_path: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").to_string_lossy().into_owned();
        create(&path, "t").unwrap();
        let d = ProjectDir::open(&path).unwrap();
        let mut m = Manifest::new("t");
        m.services.push(Service {
            id: "billing".into(),
            name: String::new(),
            path: service_path.into(),
            contracts: vec![],
            kafka: Default::default(),
            extra: Default::default(),
        });
        d.save_manifest(&m).unwrap();
        (dir, path)
    }

    async fn project() -> (tempfile::TempDir, String) {
        project_at(".").await
    }

    fn fixture(name: &str) -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dev/fixtures")
            .join(name)
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn scan(path: &str) {
        let d = ProjectDir::open(path).unwrap();
        let r = scan_service_blocking(
            &d,
            "billing",
            GitVersion::unversioned("test"),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!r.scan_file.is_empty());
    }

    #[tokio::test]
    async fn manual_operations_are_added_listed_updated_and_deleted() {
        let (_tmp, path) = project().await;
        let mut draft = OperationDraft::new(
            "billing",
            OpKind::Sql,
            Address::Sql {
                name: "invoices.insert".into(),
            },
            Source::manual(),
        );
        draft.id = "ignored".into();
        let op = registry_add_manual(path.clone(), draft.clone())
            .await
            .unwrap();
        assert_eq!(op.draft.id, "sql:billing:invoices.insert");
        assert_eq!(
            registry_add_manual(path.clone(), draft)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput,
            "дубликат"
        );
        let rows = registry_list(
            path.clone(),
            RegistryFilter {
                query: Some("INVOICES".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            registry_list(
                path.clone(),
                RegistryFilter {
                    kind: Some(OpKind::Http),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .is_empty()
        );
        let updated = registry_update_manual(
            path.clone(),
            op.draft.id.clone(),
            Manual {
                tags: vec!["critical".into()],
                notes: "n".into(),
                owner: String::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.manual.tags, ["critical"]);
        assert_eq!(
            registry_get(path.clone(), op.draft.id.clone())
                .await
                .unwrap()
                .manual
                .notes,
            "n"
        );
        registry_delete(path.clone(), op.draft.id.clone())
            .await
            .unwrap();
        assert_eq!(
            registry_get(path.clone(), op.draft.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert!(registry_scans(path.clone()).await.unwrap().is_empty());
        assert!(registry_tables(path).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_service_and_missing_operation() {
        let (_tmp, path) = project().await;
        let draft = OperationDraft::new(
            "absent",
            OpKind::Sql,
            Address::Sql { name: "x".into() },
            Source::manual(),
        );
        assert_eq!(
            registry_add_manual(path.clone(), draft)
                .await
                .unwrap_err()
                .key,
            "errors.project.noService"
        );
        assert_eq!(
            registry_delete(path.clone(), "nope".into())
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            registry_update_manual(path, "nope".into(), Manual::default())
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn manual_edits_and_manual_operations_survive_a_rescan() {
        let (_tmp, path) = project_at(&fixture("orders-openapi")).await;
        scan(&path);
        let id = "http:billing:POST /v1/orders".to_string();
        registry_update_manual(
            path.clone(),
            id.clone(),
            Manual {
                tags: vec!["critical".into()],
                notes: "keep".into(),
                owner: "team".into(),
            },
        )
        .await
        .unwrap();
        let manual_op = registry_add_manual(
            path.clone(),
            OperationDraft::new(
                "billing",
                OpKind::Sql,
                Address::Sql {
                    name: "orders.select".into(),
                },
                Source::manual(),
            ),
        )
        .await
        .unwrap();
        scan(&path);
        let op = registry_get(path.clone(), id).await.unwrap();
        assert_eq!(
            (op.manual.notes.as_str(), op.manual.owner.as_str()),
            ("keep", "team")
        );
        let manual = registry_get(path.clone(), manual_op.draft.id)
            .await
            .unwrap();
        assert_eq!(
            manual.status,
            Status::Active,
            "ручная не помечается удалённой"
        );
        let rows = registry_list(
            path.clone(),
            RegistryFilter {
                source: Some(SourceKind::Openapi),
                status: Some(Status::Active),
                service: Some("billing".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].last_version, "unversioned");
    }

    #[tokio::test]
    async fn scans_are_listed_newest_first_and_diffed() {
        let (_tmp, path) = project_at(&fixture("orders-openapi")).await;
        scan(&path);
        scan(&path);
        let scans = registry_scans(path.clone()).await.unwrap();
        assert_eq!(scans.len(), 2);
        assert!(scans[0].at >= scans[1].at);
        assert_eq!((scans[1].added, scans[0].added), (2, 0));
        let diff = registry_diff(path.clone(), scans[1].file.clone(), scans[0].file.clone())
            .await
            .unwrap();
        assert!(diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty());
        assert_eq!(
            registry_diff(
                path.clone(),
                "glafira-load.yaml".into(),
                scans[0].file.clone()
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::PathOutsideProject,
            "только файлы снимков"
        );
        assert_eq!(
            registry_diff(
                path.clone(),
                "registry/scans/none.yaml".into(),
                scans[0].file.clone()
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn diff_before_the_first_scan_is_not_found() {
        let (_tmp, path) = project().await;
        let d = ProjectDir::open(&path).unwrap();
        let _ = std::fs::remove_dir_all(d.file(SCANS_DIR));
        let e = registry_diff(
            path,
            "registry/scans/a.yaml".into(),
            "registry/scans/b.yaml".into(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::NotFound, "errors.registry.scanNotFound")
        );
    }

    /// Держит блокировку реестра проекта в другом потоке ~150 мс; возвращает флаг «отпущена».
    fn hold_registry_lock(path: &str) -> Arc<AtomicBool> {
        let released = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let (p, flag) = (path.to_string(), released.clone());
        std::thread::spawn(move || {
            let d = ProjectDir::open(&p).unwrap();
            d.with_registry_lock(|| {
                tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(150));
                flag.store(true, Ordering::SeqCst);
            });
        });
        rx.recv().unwrap();
        released
    }

    #[tokio::test]
    async fn manual_writes_wait_for_the_registry_lock() {
        let (_tmp, path) = project_at(&fixture("orders-openapi")).await;
        scan(&path);
        let id = "http:billing:POST /v1/orders".to_string();
        let released = hold_registry_lock(&path);
        let manual = Manual {
            notes: "during scan".into(),
            ..Default::default()
        };
        registry_update_manual(path.clone(), id.clone(), manual)
            .await
            .unwrap();
        assert!(released.load(Ordering::SeqCst), "правка ждала блокировку");
        // Разбор тоже берёт блокировку, и правка, сделанная до него, переживает слияние.
        let released = hold_registry_lock(&path);
        scan(&path);
        assert!(released.load(Ordering::SeqCst), "разбор ждал блокировку");
        assert_eq!(
            registry_get(path.clone(), id).await.unwrap().manual.notes,
            "during scan"
        );
        let released = hold_registry_lock(&path);
        let draft = OperationDraft::new(
            "billing",
            OpKind::Sql,
            Address::Sql { name: "x".into() },
            Source::manual(),
        );
        let op = registry_add_manual(path.clone(), draft).await.unwrap();
        assert!(
            released.load(Ordering::SeqCst),
            "добавление ждало блокировку"
        );
        let released = hold_registry_lock(&path);
        registry_delete(path.clone(), op.draft.id).await.unwrap();
        assert!(released.load(Ordering::SeqCst), "удаление ждало блокировку");
    }

    #[tokio::test]
    async fn tables_by_service() {
        let (_tmp, path) = project_at(&fixture("flyway")).await;
        scan(&path);
        let t = registry_tables(path).await.unwrap();
        let names: Vec<&str> = t["billing"].iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["orders", "audit_log"]);
    }
}
