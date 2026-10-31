//! Шаблон ПРОМ: строки из реестра, чтение заполненного файла, предпросмотр (спека L1 §7).

use std::collections::BTreeMap;
use std::path::Path;

use glafira_load_model::prod_load::{ImportPreview, preview_import};
use glafira_load_model::registry::{OpKind, Status};
use glafira_load_xlsx::{
    TemplateError, TemplateLabels, TemplateRowOut, read_template, write_template,
};
use serde::Serialize;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::scan::run::now_rfc3339;

/// Предпросмотр импорта плюс то, что видно только при чтении файла.
#[derive(Debug, Clone, Serialize)]
pub struct TemplatePreview {
    #[serde(flatten)]
    pub preview: ImportPreview,
    /// Строки с цифрами без ID: пропущены при чтении (предупреждение для пользователя).
    pub skipped_rows: usize,
}

/// Ошибки чтения xlsx: чужой или повреждённый маркер/формат и файл, который не разбирается
/// как .xlsx, — `TEMPLATE_INVALID`; ввод-вывод при разборе байтов невозможен — внутренняя ошибка.
fn map_err(e: TemplateError) -> AppError {
    match e {
        TemplateError::NoMarker => app_err!(
            ErrorCode::TemplateInvalid,
            "errors.template.notTemplate",
            "The file is not a Glafira production-load template"
        ),
        TemplateError::BadFormat(version) => app_err!(
            ErrorCode::TemplateInvalid,
            "errors.template.unsupportedFormat",
            "Unsupported template format {{version}}",
            version = version
        ),
        TemplateError::Format(m) => app_err!(
            ErrorCode::TemplateInvalid,
            "errors.template.unreadable",
            "The file cannot be read as .xlsx"
        )
        .with_details(m),
        TemplateError::Io(m) => AppError::internal_with(m),
    }
}

/// Ошибки выгрузки: неполный набор подписей — `INVALID_INPUT` (`badLabels`), запись в выбранный
/// пользователем путь не удалась — `INVALID_INPUT` (`writeFailed`). Остальное для записи невозможно.
fn map_write_err(e: TemplateError, out: &Path) -> AppError {
    match e {
        TemplateError::Format(m) => app_err!(
            ErrorCode::InvalidInput,
            "errors.template.badLabels",
            "Template labels are incomplete: {{reason}}",
            reason = m
        ),
        TemplateError::Io(m) => app_err!(
            ErrorCode::InvalidInput,
            "errors.template.writeFailed",
            "Cannot write the template to {{file}}: {{reason}}",
            file = out.display(),
            reason = m
        ),
        other @ (TemplateError::NoMarker | TemplateError::BadFormat(_)) => {
            AppError::internal_with(other.to_string())
        }
    }
}

/// Выгрузка: активные операции в порядке реестра; `prefill` — с текущими цифрами.
/// Файл `out` лежит там, куда его положил пользователь (диалог сохранения), — не в проекте.
pub fn export(
    d: &ProjectDir,
    out: &Path,
    labels: &TemplateLabels,
    prefill: bool,
) -> AppResult<usize> {
    let name = d.manifest()?.name;
    let registry = d.registry()?;
    let prod = if prefill {
        d.prod_load()?
    } else {
        Default::default()
    };
    let rows: Vec<TemplateRowOut> = registry
        .operations
        .iter()
        .filter(|o| o.status == Status::Active)
        .map(|o| TemplateRowOut {
            id: o.draft.id.clone(),
            service: o.draft.service.clone(),
            kind: o.draft.kind.as_str().into(),
            name: o.draft.name.clone(),
            summary: o.draft.summary.clone(),
            figures: prod.operations.get(&o.draft.id).cloned(),
        })
        .collect();
    write_template(out, &name, &rows, labels).map_err(|e| map_write_err(e, out))?;
    Ok(rows.len())
}

/// Содержимое файла шаблона; путь выбрал пользователь, поэтому границу проекта не проверяем.
/// Нет файла — `NOT_FOUND`; файл есть, но не читается (права, ввод-вывод) — `TEMPLATE_INVALID`.
pub fn read_file(file: &Path) -> AppResult<Vec<u8>> {
    std::fs::read(file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            app_err!(
                ErrorCode::NotFound,
                "errors.template.fileNotFound",
                "Template file not found: {{file}}",
                file = file.display()
            )
        } else {
            app_err!(
                ErrorCode::TemplateInvalid,
                "errors.template.readFailed",
                "Template file cannot be read: {{file}}: {{reason}}",
                file = file.display(),
                reason = e
            )
        }
    })
}

/// Предпросмотр: маркер, проект, строки против реестра и текущих цифр.
pub fn preview_bytes(d: &ProjectDir, bytes: &[u8]) -> AppResult<TemplatePreview> {
    let name = d.manifest()?.name;
    let read = read_template(bytes).map_err(map_err)?;
    if read.project != name {
        return Err(app_err!(
            ErrorCode::TemplateMismatch,
            "errors.template.otherProject",
            "The template belongs to project {{found}}, expected {{expected}}",
            found = read.project,
            expected = name
        ));
    }
    let registry = d.registry()?;
    let known: BTreeMap<String, OpKind> = registry
        .operations
        .iter()
        .filter(|o| o.status == Status::Active)
        .map(|o| (o.draft.id.clone(), o.draft.kind))
        .collect();
    let current = d.prod_load()?;
    let mut source = read.source;
    source.imported_at = now_rfc3339();
    Ok(TemplatePreview {
        preview: preview_import(&read.rows, source, &known, Some(&current)),
        skipped_rows: read.skipped_rows,
    })
}

pub fn preview(d: &ProjectDir, file: &Path) -> AppResult<TemplatePreview> {
    preview_bytes(d, &read_file(file)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::project::manifest::create;
    use glafira_load_model::project::{Manifest, Service};
    use glafira_load_model::registry::{
        Address, Manual, Operation, OperationDraft, Registry, Seen, Source,
    };

    pub(crate) fn labels() -> TemplateLabels {
        TemplateLabels {
            sheet_operations: "Operations".into(),
            sheet_params: "Parameters".into(),
            sheet_help: "Help".into(),
            columns: (0..12).map(|i| format!("c{i}")).collect(),
            notes: vec![String::new(); 12],
            params: (0..5).map(|i| format!("p{i}")).collect(),
            help_lines: vec![],
        }
    }

    pub(crate) fn project_with_ops() -> (tempfile::TempDir, ProjectDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").to_string_lossy().into_owned();
        create(&path, "orders-platform").unwrap();
        let d = ProjectDir::open(&path).unwrap();
        let mut m = Manifest::new("orders-platform");
        m.services.push(Service {
            id: "orders".into(),
            name: String::new(),
            path: ".".into(),
            contracts: vec![],
            kafka: Default::default(),
            extra: Default::default(),
        });
        d.save_manifest(&m).unwrap();
        let mut r = Registry::default();
        for (kind, addr) in [
            (
                OpKind::Http,
                Address::Http {
                    method: "POST".into(),
                    path: "/v1/orders".into(),
                },
            ),
            (
                OpKind::KafkaConsume,
                Address::Kafka {
                    topic: "orders.created".into(),
                },
            ),
        ] {
            let draft = OperationDraft::new("orders", kind, addr, Source::manual());
            r.operations.push(Operation {
                fingerprint: draft.fingerprint(),
                draft,
                status: Status::Active,
                first_seen: Seen::default(),
                last_seen: Seen::default(),
                manual: Manual::default(),
            });
        }
        d.save_registry(&r).unwrap();
        (dir, d)
    }

    #[test]
    fn export_then_preview_without_figures_is_all_empty() {
        let (tmp, d) = project_with_ops();
        let file = tmp.path().join("t.xlsx");
        assert_eq!(export(&d, &file, &labels(), false).unwrap(), 2);
        let p = preview(&d, &file).unwrap();
        assert_eq!(p.preview.empty.len(), 2);
        assert_eq!(p.skipped_rows, 0);
        assert!(p.preview.result.is_some());
        let json = serde_json::to_value(&p).unwrap();
        assert!(
            json["empty"].is_array() && json["skipped_rows"] == 0,
            "{json}"
        );

        // Шаблон другого проекта.
        let (_tmp2, other) = project_with_ops();
        let mut m = other.manifest().unwrap();
        m.name = "another".into();
        other.save_manifest(&m).unwrap();
        let e = preview(&other, &file).unwrap_err();
        assert_eq!(e.code, ErrorCode::TemplateMismatch);
        assert_eq!(e.params["found"], "orders-platform");
        assert_eq!(e.params["expected"], "another");
    }

    #[test]
    fn unreadable_files_get_their_own_error_classes() {
        let (tmp, d) = project_with_ops();
        // Не xlsx вообще.
        std::fs::write(tmp.path().join("x.xlsx"), b"junk").unwrap();
        let e = preview(&d, &tmp.path().join("x.xlsx")).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::TemplateInvalid, "errors.template.unreadable")
        );
        // Нет файла.
        let e = preview(&d, &tmp.path().join("none.xlsx")).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::NotFound, "errors.template.fileNotFound")
        );
        assert!(e.params["file"].ends_with("none.xlsx"));
        // Файл есть, но не читается (каталог вместо файла): не `Internal`/`errors.io`.
        let e = preview(&d, tmp.path()).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::TemplateInvalid, "errors.template.readFailed")
        );
        assert!(e.params.contains_key("reason"));
    }

    #[test]
    fn export_errors_are_mapped_separately_from_reading() {
        let (tmp, d) = project_with_ops();
        // Неполный набор подписей.
        let mut bad = labels();
        bad.columns.pop();
        let e = export(&d, &tmp.path().join("t.xlsx"), &bad, false).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::InvalidInput, "errors.template.badLabels")
        );
        assert!(e.params.contains_key("reason"));
        // Путь, в который нельзя записать.
        let e = export(&d, &tmp.path().join("no/such/dir/t.xlsx"), &labels(), false).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::InvalidInput, "errors.template.writeFailed")
        );
        assert!(e.params["file"].ends_with("t.xlsx"));
    }

    /// Шаблон с маркером проекта и двумя строками: цифра без ID и неизвестный ID.
    fn template_with_stray_rows(path: &Path) {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet();
        ws.write_string(0, 0, "glafira-load:prod-load:1:orders-platform")
            .unwrap();
        ws.write_string(2, 1, "prod").unwrap();
        let ws = wb.add_worksheet();
        ws.write_string(0, 0, "ID").unwrap();
        ws.write_number(1, 5, 3.0).unwrap();
        ws.write_string(2, 0, "http:orders:GET /gone").unwrap();
        ws.write_number(2, 5, 7.0).unwrap();
        wb.save(path).unwrap();
    }

    #[test]
    fn skipped_rows_surface_in_the_preview() {
        let (tmp, d) = project_with_ops();
        let file = tmp.path().join("stray.xlsx");
        template_with_stray_rows(&file);
        let p = preview(&d, &file).unwrap();
        assert_eq!(p.skipped_rows, 1);
        assert_eq!(serde_json::to_value(&p).unwrap()["skipped_rows"], 1);
    }

    #[test]
    fn unknown_ids_are_reported_and_not_imported() {
        let (tmp, d) = project_with_ops();
        let file = tmp.path().join("stray.xlsx");
        template_with_stray_rows(&file);
        let p = preview(&d, &file).unwrap().preview;
        assert_eq!(p.unknown_ids, vec!["http:orders:GET /gone"]);
        assert!(p.filled.is_empty());
        assert!(p.result.unwrap().operations.is_empty());
    }

    #[test]
    fn xlsx_without_marker_is_an_invalid_template() {
        let (tmp, d) = project_with_ops();
        let file = tmp.path().join("plain.xlsx");
        let mut wb = rust_xlsxwriter::Workbook::new();
        wb.add_worksheet().write_string(0, 0, "hello").unwrap();
        wb.save(&file).unwrap();
        let e = preview(&d, &file).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::TemplateInvalid, "errors.template.notTemplate")
        );
    }

    #[test]
    fn export_skips_deleted_operations() {
        let (tmp, d) = project_with_ops();
        let mut r = d.registry().unwrap();
        r.operations[0].status = Status::Removed;
        d.save_registry(&r).unwrap();
        assert_eq!(
            export(&d, &tmp.path().join("t.xlsx"), &labels(), true).unwrap(),
            1
        );
    }
}
