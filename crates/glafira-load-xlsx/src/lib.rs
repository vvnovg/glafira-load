//! Excel-шаблон нагрузки ПРОМ (спека L1 §7.1–§7.2): запись и чтение. Что в прочитанном
//! валидно — решает `glafira_load_model::prod_load::preview_import`.

pub mod export;
pub mod import;

use serde::{Deserialize, Serialize};

pub use export::{TemplateRowOut, write_template};
pub use import::{TemplateRead, read_template};

/// A1 листа «Параметры»: `glafira-load:prod-load:<формат>:<проект>`.
pub const MARKER_PREFIX: &str = "glafira-load:prod-load:";
pub const TEMPLATE_FORMAT: u32 = 1;
/// Колонки A–L листа «Операции».
pub const COLUMNS: usize = 12;
/// Строки B3..B7 листа «Параметры»: стенд, период с, период по, система, кто заполнил.
pub const PARAMS: usize = 5;

/// Подписи листов и колонок на языке интерфейса (решение плана 5: приходят из фронтенда).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateLabels {
    pub sheet_operations: String,
    pub sheet_params: String,
    pub sheet_help: String,
    /// Ровно `COLUMNS` заголовков A–L.
    pub columns: Vec<String>,
    /// Примечания к заголовкам (пустая строка — без примечания), по одному на колонку.
    pub notes: Vec<String>,
    /// Ровно `PARAMS` подписей листа «Параметры».
    pub params: Vec<String>,
    pub help_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Format(String),
    #[error("the file has no Glafira marker in the parameters sheet")]
    NoMarker,
    #[error("unsupported template format {0}")]
    BadFormat(u32),
}

impl From<rust_xlsxwriter::XlsxError> for TemplateError {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        TemplateError::Io(e.to_string())
    }
}

impl From<calamine::Error> for TemplateError {
    fn from(e: calamine::Error) -> Self {
        TemplateError::Format(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::prod_load::{Cell, Figures, SourceInfo, preview_import};
    use glafira_load_model::registry::OpKind;
    use std::collections::BTreeMap;

    pub(crate) fn labels() -> TemplateLabels {
        TemplateLabels {
            sheet_operations: "Операции".into(),
            sheet_params: "Параметры".into(),
            sheet_help: "Справка".into(),
            columns: [
                "ID",
                "Сервис",
                "Тип",
                "Операция",
                "Описание",
                "Среднее RPS",
                "Пиковое RPS",
                "Доля ошибок, %",
                "Размер запроса, байт",
                "Размер ответа, байт",
                "p95 ПРОМ, мс",
                "Комментарий",
            ]
            .map(String::from)
            .to_vec(),
            notes: vec![String::new(); COLUMNS],
            params: [
                "Стенд ПРОМ",
                "Период с",
                "Период по",
                "Источник цифр",
                "Заполнил",
            ]
            .map(String::from)
            .to_vec(),
            help_lines: vec!["Заполните колонки F–K.".into(), "ID менять нельзя.".into()],
        }
    }

    #[test]
    fn export_then_import_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.xlsx");
        let rows = vec![
            TemplateRowOut {
                id: "http:orders:POST /v1/orders".into(),
                service: "orders".into(),
                kind: "http".into(),
                name: "createOrder".into(),
                summary: "Create".into(),
                figures: Some(Figures {
                    avg: 120.0,
                    peak: 340.0,
                    error_pct: Some(0.2),
                    req_bytes: Some(850),
                    resp_bytes: Some(420),
                    p95_ms: Some(180.0),
                    lag_max_s: None,
                    comment: "ok".into(),
                }),
            },
            TemplateRowOut {
                id: "kafka_consume:orders.created".into(),
                service: "orders".into(),
                kind: "kafka_consume".into(),
                name: "orders.created".into(),
                summary: String::new(),
                figures: Some(Figures {
                    avg: 10.0,
                    peak: 20.0,
                    error_pct: None,
                    req_bytes: Some(1200),
                    resp_bytes: None,
                    p95_ms: None,
                    lag_max_s: Some(5.0),
                    comment: String::new(),
                }),
            },
            TemplateRowOut {
                id: "http:orders:GET /v1/orders/{id}".into(),
                service: "orders".into(),
                kind: "http".into(),
                name: "getOrder".into(),
                summary: String::new(),
                figures: None,
            },
        ];
        write_template(&path, "orders-platform", &rows, &labels()).unwrap();
        let read = read_template(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            (read.format, read.project.as_str()),
            (TEMPLATE_FORMAT, "orders-platform")
        );
        assert_eq!(read.rows.len(), 3);
        assert_eq!(read.rows[0].row, 2);
        assert_eq!(read.rows[0].avg, Cell::Number { value: 120.0 });
        assert_eq!(read.rows[0].comment, "ok");
        assert_eq!(
            read.rows[1].p95_ms,
            Cell::Number { value: 5.0 },
            "лаг consume лежит в последней числовой колонке"
        );
        assert_eq!(read.rows[2].avg, Cell::Empty);
        let known: BTreeMap<String, OpKind> = [
            ("http:orders:POST /v1/orders", OpKind::Http),
            ("kafka_consume:orders.created", OpKind::KafkaConsume),
            ("http:orders:GET /v1/orders/{id}", OpKind::Http),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let p = preview_import(&read.rows, SourceInfo::default(), &known, None);
        assert!(p.invalid.is_empty(), "{:?}", p.invalid);
        let r = p.result.unwrap();
        assert_eq!(
            r.operations["http:orders:POST /v1/orders"],
            rows[0].figures.clone().unwrap()
        );
        assert_eq!(
            r.operations["kafka_consume:orders.created"].lag_max_s,
            Some(5.0)
        );
        assert_eq!(p.empty, vec!["http:orders:GET /v1/orders/{id}"]);
    }

    #[test]
    fn files_without_marker_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.xlsx");
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet();
        ws.write_string(0, 0, "ID").unwrap();
        wb.add_worksheet();
        wb.save(&path).unwrap();
        assert_eq!(
            read_template(&std::fs::read(&path).unwrap()).unwrap_err(),
            TemplateError::NoMarker
        );
        assert!(matches!(
            read_template(b"not an xlsx").unwrap_err(),
            TemplateError::Format(_)
        ));
    }

    /// Книга «как после ручного заполнения»: пробелы, запятая, текст вместо числа.
    fn write_plain(path: &std::path::Path, marker: &str) {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet();
        ws.write_string(0, 0, "ID").unwrap();
        ws.write_string(1, 0, "http:o:GET /a").unwrap();
        ws.write_string(1, 5, "  ").unwrap();
        ws.write_string(1, 6, " 12,5 ").unwrap();
        ws.write_string(1, 7, "lots").unwrap();
        ws.write_number(1, 8, 7.0).unwrap();
        ws.write_string(1, 11, "  note  ").unwrap();
        // Строка без ID пропускается.
        ws.write_number(2, 5, 1.0).unwrap();
        let ws = wb.add_worksheet();
        ws.write_string(0, 0, marker).unwrap();
        ws.write_string(2, 1, " prod ").unwrap();
        ws.write_string(6, 1, "Ivanov").unwrap();
        wb.save(path).unwrap();
    }

    #[test]
    fn reader_trims_text_and_maps_blank_cells_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.xlsx");
        write_plain(&path, "glafira-load:prod-load:1:demo");
        let read = read_template(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read.rows.len(), 1, "строка без ID пропущена");
        let r = &read.rows[0];
        assert_eq!(r.row, 2);
        assert_eq!(r.avg, Cell::Empty, "пробелы — пустая ячейка");
        assert_eq!(
            r.peak,
            Cell::Number { value: 12.5 },
            "запятая как разделитель"
        );
        assert_eq!(
            r.error_pct,
            Cell::Text {
                value: "lots".into()
            }
        );
        assert_eq!(r.req_bytes, Cell::Number { value: 7.0 });
        assert_eq!(r.resp_bytes, Cell::Empty);
        assert_eq!(r.comment, "note");
        assert_eq!(read.source.stand, "prod");
        assert_eq!(read.source.filled_by, "Ivanov");
        assert_eq!(read.source.system, "");
    }

    #[test]
    fn other_marker_versions_and_malformed_markers_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.xlsx");
        for (marker, expected) in [
            ("glafira-load:prod-load:2:demo", TemplateError::BadFormat(2)),
            ("glafira-load:prod-load:x:demo", TemplateError::NoMarker),
            ("glafira-load:prod-load:1", TemplateError::NoMarker),
            ("something else", TemplateError::NoMarker),
        ] {
            write_plain(&path, marker);
            assert_eq!(
                read_template(&std::fs::read(&path).unwrap()).unwrap_err(),
                expected,
                "{marker}"
            );
        }
    }

    #[test]
    fn single_sheet_workbook_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("one.xlsx");
        let mut wb = rust_xlsxwriter::Workbook::new();
        wb.add_worksheet().write_string(0, 0, "ID").unwrap();
        wb.save(&path).unwrap();
        assert_eq!(
            read_template(&std::fs::read(&path).unwrap()).unwrap_err(),
            TemplateError::NoMarker
        );
    }

    #[test]
    fn project_name_with_colon_survives_and_empty_export_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.xlsx");
        write_template(&path, "a:b", &[], &labels()).unwrap();
        let read = read_template(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read.project, "a:b");
        assert!(read.rows.is_empty());
    }

    #[test]
    fn wrong_label_counts_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut bad = labels();
        bad.columns.pop();
        let err = write_template(&dir.path().join("x.xlsx"), "p", &[], &bad).unwrap_err();
        assert!(matches!(err, TemplateError::Format(_)));
    }

    #[test]
    fn template_is_protected_with_validation_freeze_and_filter() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.xlsx");
        let rows = vec![TemplateRowOut {
            id: "http:o:GET /a".into(),
            service: "o".into(),
            kind: "http".into(),
            name: "a".into(),
            summary: String::new(),
            figures: None,
        }];
        write_template(&path, "demo", &rows, &labels()).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut read = |name: &str| {
            let mut s = String::new();
            zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        let sheet = read("xl/worksheets/sheet1.xml");
        assert!(sheet.contains("<sheetProtection"), "лист защищён");
        assert!(sheet.contains("<pane"), "заголовок закреплён");
        assert!(sheet.contains("<autoFilter"), "автофильтр");
        assert!(sheet.contains("<dataValidations"), "валидация данных");
        assert!(
            read("xl/styles.xml").contains("locked=\"0\""),
            "есть разблокированный формат"
        );
    }
}
