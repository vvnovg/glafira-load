//! Разбор сервиса: контракты → черновики → слияние → снимок (спека L1 §5.3, §6.3).
//!
//! Разбор не удался (нет папки клона), отменён или ни один найденный контракт не разобран —
//! реестр, снимок и таблицы не пишутся: слияние с пустым списком черновиков пометило бы все
//! операции сервиса удалёнными. Сервис, у которого контрактов не нашлось вовсе, сливается как
//! обычно (с пустым списком черновиков). Файл
//! контракта, который не читается или не разбирается, — диагностика в отчёте, остальные
//! контракты разбираются; ошибкой разбор завершается только при сбое чтения или записи файлов
//! самого проекта.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use glafira_load_contracts::{
    ContractInput, ContractKind, Diagnostic, ParseOptions, ParseResult, Table, TopicBinding, parse,
    sql,
};
use glafira_load_model::diff::{SCAN_FORMAT, Scan};
use glafira_load_model::merge::{MergeSummary, merge};
use glafira_load_model::project::{ContractRef, Service};
use glafira_load_model::registry::{SCANS_DIR, Seen};
use glafira_load_model::to_yaml;
use serde::Serialize;

use crate::error::{AppResult, ErrorCode};
use crate::git::version::GitVersion;
use crate::i18n::LocalizedText;
use crate::project::manifest::ProjectDir;
use crate::project::paths::{canonical, inside, relative, resolve};
use crate::scan::discover::discover;
use crate::scan::migrations;
pub use crate::scan::registry::ScanRegistry;

/// Справочник таблиц из миграций: `registry/tables/<сервис>.yaml`.
pub const TABLES_DIR: &str = "registry/tables";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub service: String,
    /// Сколько файлов уже разобрано; `done == total` — разбор файлов закончен.
    pub done: usize,
    pub total: usize,
    /// Файл, который разбирается сейчас (относительно клона); пусто в последнем событии.
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub service: String,
    pub git: GitVersion,
    pub contracts: usize,
    pub operations_found: usize,
    pub tables_found: usize,
    pub summary: MergeSummary,
    /// Предупреждения импортёров, git недоступен, нет папки клона, чтение соседнего файла вне клона.
    pub warnings: Vec<Diagnostic>,
    /// Файлы контрактов, которые не прочитаны или не разобраны (`CONTRACT_PARSE`): разбор
    /// остальных продолжается.
    pub errors: Vec<Diagnostic>,
    /// `registry/scans/<метка>.yaml`; пусто, если реестр не менялся (разбор отменён, нет клона
    /// или ни один контракт не разобран).
    pub scan_file: String,
    pub cancelled: bool,
    /// Почему реестр не менялся (ни один контракт не разобран) — для заметной строки в отчёте.
    pub notice: Option<LocalizedText>,
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// `ГГГГ-ММ-ДД-ЧЧММСС` для имён снимков и копий шаблонов.
pub fn now_label() -> String {
    let f = time::macros::format_description!("[year]-[month]-[day]-[hour][minute][second]");
    time::OffsetDateTime::now_utc()
        .format(f)
        .unwrap_or_default()
}

/// Путь внутри клона в том виде, в каком его видят импортёры: относительно канонического корня,
/// разделитель `/`. Вне клона (в том числе через ссылку) — `PATH_OUTSIDE_PROJECT`.
fn clone_rel(clone: &Path, raw: &str) -> AppResult<String> {
    let raw = raw.replace('\\', "/");
    let abs = inside(clone, &resolve(clone, &raw))?;
    Ok(relative(clone, &abs))
}

/// Ключи `topics` — в той же форме, что `ContractInput::path`: `schema.rs` ищет привязку точным
/// совпадением пути (`./a.avsc` и `a.avsc` в манифесте означают один файл).
fn options(service: &Service, clone: &Path, contract: &ContractRef) -> ParseOptions {
    let topics = service
        .kafka
        .topics
        .iter()
        .map(|(topic, s)| {
            let key = clone_rel(clone, s.schema()).unwrap_or_else(|_| s.schema().to_string());
            let binding = TopicBinding {
                topic: topic.clone(),
                direction: s.direction(),
            };
            (key, binding)
        })
        .collect::<BTreeMap<_, _>>();
    ParseOptions {
        service: service.id.clone(),
        topics,
        include: contract.include.clone(),
    }
}

fn diag(path: &str, line: Option<u32>, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        path: path.into(),
        line,
        message: message.into(),
    }
}

/// Имя снимка по времени; два разбора в одну секунду (все сервисы подряд) не затирают друг друга.
fn snapshot_file(dir: &ProjectDir) -> String {
    let label = now_label();
    let mut file = format!("{SCANS_DIR}/{label}.yaml");
    let mut n = 1;
    while dir.file(&file).exists() {
        n += 1;
        file = format!("{SCANS_DIR}/{label}-{n}.yaml");
    }
    file
}

/// Блокирующий вызов: читает контракты, сливает реестр, пишет снимок и таблицы.
pub fn scan_service_blocking(
    dir: &ProjectDir,
    service_id: &str,
    git: GitVersion,
    progress: &(dyn Fn(ScanProgress) + Sync),
    cancel: &AtomicBool,
) -> AppResult<ScanReport> {
    let manifest = dir.manifest()?;
    let mut report = ScanReport {
        service: service_id.into(),
        git: git.clone(),
        contracts: 0,
        operations_found: 0,
        tables_found: 0,
        summary: MergeSummary::default(),
        warnings: Vec::new(),
        errors: Vec::new(),
        scan_file: String::new(),
        cancelled: false,
        notice: None,
    };
    let clone = match dir.service_root(&manifest, service_id) {
        Ok(p) => canonical(&p)?,
        // Нет папки клона — предупреждение; реестр не трогаем.
        Err(e) if e.code == ErrorCode::NotFound && manifest.service(service_id).is_some() => {
            let path = manifest.service(service_id).map(|s| s.path.clone());
            report
                .warnings
                .push(diag(&path.unwrap_or_default(), None, e.message));
            return Ok(report);
        }
        Err(e) => return Err(e),
    };
    let service = manifest
        .service(service_id)
        .expect("checked by service_root")
        .clone();
    // Файл таблиц проверяется до записи реестра: недопустимый `id` не оставит запись наполовину.
    let tables_file = format!("{TABLES_DIR}/{service_id}.yaml");
    inside(dir.root(), &dir.file(&tables_file))?;
    if !git.available {
        report.warnings.push(diag(
            "",
            None,
            format!(
                "git unavailable, version recorded as {}: {}",
                git.version,
                git.error.as_deref().unwrap_or("")
            ),
        ));
    }

    // Чтение соседних файлов (`$ref`, `import`): только внутри клона; попытка выйти — диагностика.
    let rejected = RefCell::new(Vec::<String>::new());
    let read = |rel: &str| -> Option<Vec<u8>> {
        match inside(&clone, &clone.join(rel)) {
            Ok(p) => std::fs::read(p).ok(),
            Err(e) => {
                rejected.borrow_mut().push(e.message);
                None
            }
        }
    };

    let listed: Vec<ContractRef> = if service.contracts.is_empty() {
        discover(&clone)
    } else {
        service
            .contracts
            .iter()
            .filter(|c| c.enabled)
            .cloned()
            .collect()
    };
    // Пути контрактов — в канонической форме относительно клона (как у автопоиска).
    let mut contracts = Vec::new();
    for mut c in listed {
        match clone_rel(&clone, &c.path) {
            Ok(rel) => {
                c.path = rel;
                contracts.push(c);
            }
            Err(e) => report.errors.push(diag(&c.path, None, e.message)),
        }
    }
    let outside = report.errors.len();
    // SQL — после остальных и в порядке применения миграций; откатные миграции не применяются.
    let (sql_list, mut contracts): (Vec<_>, Vec<_>) = contracts
        .into_iter()
        .partition(|c| c.kind == ContractKind::Sql);
    let (down, sql_list): (Vec<_>, Vec<_>) = sql_list
        .into_iter()
        .partition(|c| migrations::is_down(&c.path));
    for c in down {
        report.warnings.push(diag(
            &c.path,
            None,
            "down migration is not applied to the table list",
        ));
    }
    contracts.extend(migrations::order(sql_list, &read));
    rejected.borrow_mut().clear();

    let total = contracts.len() + outside;
    report.contracts = total;
    let (mut drafts, mut tables) = (Vec::new(), Vec::<Table>::new());
    let mut parsed_ok = 0usize;
    for (i, c) in contracts.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        progress(ScanProgress {
            service: service_id.into(),
            done: outside + i,
            total,
            file: c.path.clone(),
        });
        let bytes = match std::fs::read(clone.join(&c.path)) {
            Ok(b) => b,
            Err(e) => {
                report
                    .errors
                    .push(diag(&c.path, None, format!("file not readable: {e}")));
                continue;
            }
        };
        let input = ContractInput {
            path: &c.path,
            bytes: &bytes,
            read: &read,
        };
        let opts = options(&service, &clone, c);
        let parsed: Result<ParseResult, _> = if c.kind == ContractKind::Sql {
            // Миграция применяется к таблицам предыдущих.
            sql::parse_onto(&input, &opts, tables.clone())
        } else {
            parse(c.kind, &input, &opts)
        };
        match parsed {
            Ok(r) => {
                parsed_ok += 1;
                drafts.extend(r.operations);
                if c.kind == ContractKind::Sql {
                    tables = r.tables;
                } else {
                    tables.extend(r.tables);
                }
                report.warnings.extend(r.warnings);
            }
            Err(e) => report.errors.push(diag(&c.path, e.line, e.message)),
        }
        let mut seen_msgs = std::collections::BTreeSet::new();
        for m in rejected.borrow_mut().drain(..) {
            if seen_msgs.insert(m.clone()) {
                report.warnings.push(diag(&c.path, None, m));
            }
        }
    }
    // Отмена во время последнего файла тоже отменяет слияние.
    report.cancelled |= cancel.load(Ordering::Relaxed);
    report.operations_found = drafts.len();
    report.tables_found = tables.len();
    progress(ScanProgress {
        service: service_id.into(),
        done: total,
        total,
        file: String::new(),
    });
    if report.cancelled {
        return Ok(report);
    }
    // Контракты были, но ни один не разобран: слияние пометило бы все операции удалёнными.
    if total > 0 && parsed_ok == 0 {
        let notice = ltext!(
            "errors.scan.nothingParsed",
            "No contract could be parsed; registry left unchanged"
        );
        report.warnings.push(diag("", None, notice.text.clone()));
        report.notice = Some(notice);
        return Ok(report);
    }

    let seen = Seen {
        version: git.version.clone(),
        commit: git.commit.clone(),
        at: now_rfc3339(),
    };
    let mut registry = dir.registry()?;
    // Правило `last_seen` (повторный разбор той же версии не меняет файл) — внутри `merge`.
    report.summary = merge(&mut registry, service_id, drafts, &seen);
    dir.save_registry(&registry)?;
    let scan = Scan {
        format: SCAN_FORMAT,
        at: seen.at.clone(),
        service: service_id.into(),
        version: seen.version.clone(),
        commit: seen.commit.clone(),
        summary: report.summary.clone(),
        fingerprints: registry.fingerprints(),
    };
    let scan_file = snapshot_file(dir);
    dir.write_text(&scan_file, &to_yaml(&scan))?;
    dir.write_text(&tables_file, &to_yaml(&tables))?;
    report.scan_file = scan_file;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::project::manifest::create;
    use glafira_load_model::project::{ContractKind, ContractRef, Manifest, Service, TopicSchema};
    use glafira_load_model::registry::{Manual, Status};
    use std::path::Path;

    fn project_with(
        service_path: &str,
        contracts: Vec<ContractRef>,
    ) -> (tempfile::TempDir, ProjectDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "t").unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        let mut m = Manifest::new("t");
        m.services.push(Service {
            id: "orders".into(),
            name: "Orders".into(),
            path: service_path.into(),
            contracts,
            kafka: Default::default(),
            extra: Default::default(),
        });
        d.save_manifest(&m).unwrap();
        (dir, d)
    }

    fn fixtures() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dev/fixtures")
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn sql(path: &str) -> ContractRef {
        ContractRef {
            kind: ContractKind::Sql,
            path: path.into(),
            include: vec![],
            enabled: true,
        }
    }

    fn scan(d: &ProjectDir) -> ScanReport {
        scan_service_blocking(
            d,
            "orders",
            GitVersion::unversioned("test"),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn tables(d: &ProjectDir) -> Vec<(String, Vec<String>)> {
        let text = d.read_text("registry/tables/orders.yaml").unwrap().unwrap();
        let t: Vec<Table> = glafira_load_model::from_yaml(&text).unwrap();
        t.into_iter()
            .map(|t| (t.name, t.columns.into_iter().map(|c| c.name).collect()))
            .collect()
    }

    #[test]
    fn scan_merges_writes_snapshot_and_is_idempotent() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let seen = std::sync::Mutex::new(Vec::new());
        let progress = |p: ScanProgress| seen.lock().unwrap().push(p.done);
        let r = scan_service_blocking(
            &d,
            "orders",
            GitVersion::unversioned("test"),
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            (r.contracts, r.operations_found, r.summary.added.len()),
            (1, 2, 2)
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(d.file(&r.scan_file).is_file());
        assert!(d.file("registry/tables/orders.yaml").is_file());
        assert_eq!(*seen.lock().unwrap(), vec![0, 1]);
        let text1 = d.read_text("registry/operations.yaml").unwrap().unwrap();
        let r2 = scan(&d);
        assert!(r2.summary.is_empty());
        assert_ne!(
            r2.scan_file, r.scan_file,
            "второй снимок в ту же секунду не затирает первый"
        );
        assert!(d.file(&r.scan_file).is_file() && d.file(&r2.scan_file).is_file());
        assert_eq!(
            d.read_text("registry/operations.yaml").unwrap().unwrap(),
            text1
        );
        assert_eq!(
            d.registry()
                .unwrap()
                .get("http:orders:POST /v1/orders")
                .unwrap()
                .last_seen
                .version,
            "unversioned"
        );
    }

    #[test]
    fn explicit_contracts_outside_the_clone_and_broken_files_are_errors_not_panics() {
        let (_tmp, d) = project_with(
            &format!("{}/orders-openapi", fixtures()),
            vec![
                ContractRef {
                    kind: ContractKind::Openapi,
                    path: "../orders-swagger/swagger.json".into(),
                    include: vec![],
                    enabled: true,
                },
                ContractRef {
                    kind: ContractKind::Openapi,
                    path: "schemas/order.yaml".into(),
                    include: vec![],
                    enabled: true,
                },
                ContractRef {
                    kind: ContractKind::Sql,
                    path: "nope.sql".into(),
                    include: vec![],
                    enabled: false,
                },
            ],
        );
        let r = scan(&d);
        assert_eq!(r.contracts, 2, "выключенный контракт не считается");
        assert_eq!(r.errors.len(), 2, "{:?}", r.errors);
        assert!(r.errors[0].message.contains("outside"));
        assert_eq!(r.operations_found, 0);
        assert!(
            r.scan_file.is_empty(),
            "ни один контракт не разобран — реестр не трогаем"
        );
        assert_eq!(
            r.notice.as_ref().map(|n| n.key.as_str()),
            Some("errors.scan.nothingParsed")
        );
        assert!(
            r.warnings
                .iter()
                .any(|w| w.message.contains("No contract could be parsed")),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn scan_where_every_contract_fails_leaves_the_registry_byte_identical() {
        let tmp = tempfile::tempdir().unwrap();
        let clone = tmp.path().join("clone");
        std::fs::create_dir_all(clone.join("schemas")).unwrap();
        for f in ["openapi.yaml", "schemas/order.yaml"] {
            std::fs::copy(format!("{}/orders-openapi/{f}", fixtures()), clone.join(f)).unwrap();
        }
        let (_p, d) = project_with(clone.to_str().unwrap(), vec![]);
        assert_eq!(scan(&d).summary.added.len(), 2);
        let before = d.read_text("registry/operations.yaml").unwrap().unwrap();
        let tables_before = d.read_text("registry/tables/orders.yaml").unwrap();
        let snapshots = std::fs::read_dir(d.file(SCANS_DIR)).unwrap().count();
        // Контракт сломан: найден, но не разбирается.
        std::fs::write(clone.join("openapi.yaml"), "openapi: [: not yaml").unwrap();
        let r = scan(&d);
        assert_eq!(r.contracts, 1);
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert!(r.scan_file.is_empty() && !r.cancelled);
        assert!(r.summary.removed.is_empty());
        assert_eq!(
            d.read_text("registry/operations.yaml").unwrap().unwrap(),
            before,
            "операции не помечены удалёнными"
        );
        assert_eq!(
            d.read_text("registry/tables/orders.yaml").unwrap(),
            tables_before
        );
        assert_eq!(
            std::fs::read_dir(d.file(SCANS_DIR)).unwrap().count(),
            snapshots
        );
    }

    #[test]
    fn service_without_contracts_still_merges() {
        let tmp = tempfile::tempdir().unwrap();
        let (_p, d) = project_with(tmp.path().to_str().unwrap(), vec![]);
        let r = scan(&d);
        assert_eq!(r.contracts, 0);
        assert!(r.notice.is_none());
        assert!(!r.scan_file.is_empty());
    }

    #[test]
    fn cancel_flag_stops_before_merging() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let cancel = AtomicBool::new(true);
        let r = scan_service_blocking(
            &d,
            "orders",
            GitVersion::unversioned("test"),
            &|_| {},
            &cancel,
        )
        .unwrap();
        assert!(r.cancelled && r.scan_file.is_empty());
        assert_eq!(d.registry().unwrap().operations.len(), 0);
        assert!(!d.file("registry/tables/orders.yaml").exists());
    }

    #[test]
    fn cancel_during_the_last_file_still_skips_the_merge() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let cancel = AtomicBool::new(false);
        let progress = |p: ScanProgress| {
            if p.done == 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let r = scan_service_blocking(
            &d,
            "orders",
            GitVersion::unversioned("test"),
            &progress,
            &cancel,
        )
        .unwrap();
        assert!(r.cancelled && r.scan_file.is_empty());
        assert_eq!(d.registry().unwrap().operations.len(), 0);
    }

    #[test]
    fn missing_clone_is_a_warning_and_keeps_the_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let clone = tmp.path().join("clone");
        std::fs::create_dir(&clone).unwrap();
        std::fs::copy(
            format!("{}/orders-openapi/openapi.yaml", fixtures()),
            clone.join("openapi.yaml"),
        )
        .unwrap();
        std::fs::create_dir(clone.join("schemas")).unwrap();
        std::fs::copy(
            format!("{}/orders-openapi/schemas/order.yaml", fixtures()),
            clone.join("schemas/order.yaml"),
        )
        .unwrap();
        let (_p, d) = project_with(clone.to_str().unwrap(), vec![]);
        assert_eq!(scan(&d).summary.added.len(), 2);
        let before = d.read_text("registry/operations.yaml").unwrap().unwrap();
        let snapshots = std::fs::read_dir(d.file(SCANS_DIR)).unwrap().count();
        std::fs::rename(&clone, tmp.path().join("gone")).unwrap();
        let r = scan(&d);
        assert!(r.scan_file.is_empty() && !r.cancelled);
        assert!(
            r.warnings.iter().any(|w| w.message.contains("not found")),
            "{:?}",
            r.warnings
        );
        assert_eq!(
            d.read_text("registry/operations.yaml").unwrap().unwrap(),
            before,
            "операции не помечены удалёнными"
        );
        assert_eq!(
            std::fs::read_dir(d.file(SCANS_DIR)).unwrap().count(),
            snapshots
        );
    }

    #[test]
    fn unknown_service_fails() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let e = scan_service_blocking(
            &d,
            "absent",
            GitVersion::unversioned("test"),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
    }

    #[test]
    fn git_unavailable_is_a_warning() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let r = scan_service_blocking(
            &d,
            "orders",
            GitVersion::unversioned("no git here"),
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            r.warnings.iter().any(|w| w.message.contains("no git here")),
            "{:?}",
            r.warnings
        );
        let ok = GitVersion {
            version: "v1".into(),
            commit: "abc".into(),
            available: true,
            error: None,
        };
        let r = scan_service_blocking(&d, "orders", ok, &|_| {}, &AtomicBool::new(false)).unwrap();
        assert!(
            r.warnings.iter().all(|w| !w.message.contains("git")),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn flyway_migrations_are_applied_in_order_into_one_table_list() {
        let (_tmp, d) = project_with(&format!("{}/flyway", fixtures()), vec![]);
        let r = scan(&d);
        assert_eq!(r.contracts, 2);
        assert_eq!(
            tables(&d),
            [
                (
                    "orders".to_string(),
                    vec![
                        "id".to_string(),
                        "status".into(),
                        "created_at".into(),
                        "note".into()
                    ]
                ),
                (
                    "audit_log".to_string(),
                    vec!["id".to_string(), "payload".into()]
                ),
            ]
        );
        assert_eq!(r.tables_found, 2);
        // Порядок из манифеста не важен: V2 всё равно применяется после V1.
        let (_tmp2, d2) = project_with(
            &format!("{}/flyway", fixtures()),
            vec![
                sql("db/migration/V2__add_column.sql"),
                sql("db/migration/V1__init.sql"),
            ],
        );
        scan(&d2);
        assert_eq!(tables(&d2), tables(&d));
    }

    #[test]
    fn liquibase_changelog_and_its_sql_file() {
        let (_tmp, d) = project_with(&format!("{}/liquibase", fixtures()), vec![]);
        let r = scan(&d);
        assert_eq!(r.contracts, 2);
        let names: Vec<String> = tables(&d).into_iter().map(|t| t.0).collect();
        assert_eq!(names, ["invoices", "payments"]);
    }

    #[test]
    fn down_migrations_are_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        let m = tmp.path().join("migrations");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("1_init.up.sql"), "CREATE TABLE a (id int);").unwrap();
        std::fs::write(m.join("1_init.down.sql"), "DROP TABLE a;").unwrap();
        std::fs::write(m.join("2_b.up.sql"), "ALTER TABLE a ADD COLUMN b text;").unwrap();
        std::fs::write(m.join("2_b.down.sql"), "ALTER TABLE a DROP COLUMN b;").unwrap();
        let (_p, d) = project_with(tmp.path().to_str().unwrap(), vec![]);
        assert_eq!(scan(&d).contracts, 2);
        assert_eq!(
            tables(&d),
            [("a".to_string(), vec!["id".to_string(), "b".into()])]
        );
        // Явно перечисленная откатная миграция тоже не применяется — с предупреждением.
        let (_p2, d2) = project_with(
            tmp.path().to_str().unwrap(),
            vec![
                sql("migrations/1_init.up.sql"),
                sql("migrations/1_init.down.sql"),
            ],
        );
        let r = scan(&d2);
        assert_eq!(tables(&d2), [("a".to_string(), vec!["id".to_string()])]);
        assert!(
            r.warnings
                .iter()
                .any(|w| w.path == "migrations/1_init.down.sql"),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn kafka_topics_bind_schema_files_by_relative_path() {
        let (_tmp, d) = project_with(&format!("{}/schemas", fixtures()), vec![]);
        let mut m = d.manifest().unwrap();
        m.services[0].kafka.topics.insert(
            "orders.created.v1".into(),
            TopicSchema::Path("./orders-created.avsc".into()),
        );
        d.save_manifest(&m).unwrap();
        let r = scan(&d);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let reg = d.registry().unwrap();
        let op = reg
            .get("kafka_produce:orders.created.v1")
            .expect("топик из манифеста");
        assert!(!op.draft.topic_guessed);
        assert!(
            reg.operations.iter().any(|o| o.draft.topic_guessed),
            "payment — по имени файла"
        );
    }

    #[cfg(unix)]
    #[test]
    fn neighbour_reads_stay_inside_the_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(
            outside.path().join("secret.yaml"),
            "Secret: { type: object, properties: { pin: { type: string } } }\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.yaml"),
            tmp.path().join("ext.yaml"),
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("openapi.yaml"),
            "openapi: 3.0.3\ninfo: { title: x, version: '1' }\npaths:\n  /x:\n    post:\n      requestBody:\n        content:\n          application/json:\n            schema: { $ref: './ext.yaml#/Secret' }\n      responses:\n        '200': { description: ok }\n",
        )
        .unwrap();
        let (_p, d) = project_with(tmp.path().to_str().unwrap(), vec![]);
        let r = scan(&d);
        assert!(
            r.warnings
                .iter()
                .any(|w| w.path == "openapi.yaml" && w.message.contains("outside")),
            "{:?}",
            r.warnings
        );
        let reg = d.registry().unwrap();
        let text = glafira_load_model::to_yaml(&reg);
        assert!(!text.contains("pin"), "файл вне клона не прочитан");
    }

    #[test]
    fn manual_fields_survive_a_rescan() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        scan(&d);
        let mut reg = d.registry().unwrap();
        reg.get_mut("http:orders:POST /v1/orders").unwrap().manual = Manual {
            tags: vec!["critical".into()],
            notes: "n".into(),
            owner: "me".into(),
        };
        d.save_registry(&reg).unwrap();
        scan(&d);
        let op = d
            .registry()
            .unwrap()
            .get("http:orders:POST /v1/orders")
            .cloned()
            .unwrap();
        assert_eq!(op.manual.tags, ["critical"]);
        assert_eq!(op.status, Status::Active);
    }
}
