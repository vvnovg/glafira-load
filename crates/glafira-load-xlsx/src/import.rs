//! Чтение заполненного шаблона: только преобразование ячеек в `Cell`, без проверки валидности.

use std::io::Cursor;

use calamine::{Data, Range, Reader, open_workbook_auto_from_rs};
use glafira_load_model::prod_load::{Cell, SourceInfo, TemplateRow};

use crate::{MARKER_PREFIX, PARAMS, TEMPLATE_FORMAT, TemplateError};

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRead {
    pub format: u32,
    pub project: String,
    pub rows: Vec<TemplateRow>,
    /// Строки с цифрами/комментарием (F–L), но без ID: пропущены, приложение может показать счётчик.
    pub skipped_rows: usize,
    /// Лист «Параметры»; `imported_at` и `file` заполняет приложение.
    pub source: SourceInfo,
}

fn text(d: Option<&Data>) -> String {
    match d {
        Some(Data::String(s)) => s.trim().to_string(),
        Some(Data::Float(f)) => f.to_string(),
        Some(Data::Int(i)) => i.to_string(),
        Some(Data::Bool(b)) => b.to_string(),
        // Даты (поля периода) — в виде `ГГГГ-ММ-ДД`; длительность — числом дней.
        Some(Data::DateTime(dt)) if dt.is_datetime() => {
            let (y, m, d, ..) = dt.to_ymd_hms_milli();
            format!("{y:04}-{m:02}-{d:02}")
        }
        Some(Data::DateTime(dt)) => dt.as_f64().to_string(),
        Some(Data::DateTimeIso(s)) | Some(Data::DurationIso(s)) => s.clone(),
        Some(Data::Error(e)) => format!("#{e:?}"),
        Some(Data::Empty) | None => String::new(),
    }
}

/// Конечное число — `Cell::Number`; `NaN`/`inf` не бывают валидными цифрами.
fn number(v: f64, original: &str) -> Cell {
    if v.is_finite() {
        Cell::Number { value: v }
    } else {
        Cell::Text {
            value: original.to_string(),
        }
    }
}

/// Пустая и состоящая из пробелов ячейка — `Cell::Empty`; текст, похожий на число
/// (в том числе с запятой), — число. Нечисловые значения (`nan`, `inf`, `1e999`) — текст.
fn cell(d: Option<&Data>) -> Cell {
    match d {
        Some(Data::Float(f)) => number(*f, &f.to_string()),
        Some(Data::Int(i)) => Cell::Number { value: *i as f64 },
        Some(Data::Empty) | None => Cell::Empty,
        Some(Data::String(s)) if s.trim().is_empty() => Cell::Empty,
        Some(Data::String(s)) => match s.trim().replace(',', ".").parse::<f64>() {
            Ok(v) => number(v, s.trim()),
            Err(_) => Cell::Text {
                value: s.trim().to_string(),
            },
        },
        other => Cell::Text { value: text(other) },
    }
}

/// Ячейка по абсолютным координатам листа (0-based), независимо от начала диапазона.
fn at(range: &Range<Data>, row: u32, col: u32) -> Option<&Data> {
    range.get_value((row, col))
}

/// A1 листа — маркер Glafira (проверяется только префикс; разбор — в `marker`).
fn marker_text(range: &Range<Data>) -> Option<String> {
    let a1 = text(at(range, 0, 0));
    a1.starts_with(MARKER_PREFIX).then_some(a1)
}

fn marker(a1: &str) -> Result<(u32, String), TemplateError> {
    let Some(rest) = a1.strip_prefix(MARKER_PREFIX) else {
        return Err(TemplateError::NoMarker);
    };
    let (format, project) = rest.split_once(':').ok_or(TemplateError::NoMarker)?;
    let format: u32 = format.parse().map_err(|_| TemplateError::NoMarker)?;
    if format != TEMPLATE_FORMAT {
        return Err(TemplateError::BadFormat(format));
    }
    Ok((format, project.to_string()))
}

pub fn read_template(bytes: &[u8]) -> Result<TemplateRead, TemplateError> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes))?;
    let names = wb.sheet_names().to_vec();
    // Лист параметров — тот, в A1 которого маркер; порядок листов не важен.
    let mut found = None;
    for (i, name) in names.iter().enumerate() {
        let range = wb.worksheet_range(name)?;
        if let Some(a1) = marker_text(&range) {
            found = Some((i, range, a1));
            break;
        }
    }
    let (params_idx, params, a1) = found.ok_or(TemplateError::NoMarker)?;
    let (format, project) = marker(&a1)?;
    // Операции — первый лист, который не «Параметры».
    let ops_name = names
        .iter()
        .enumerate()
        .find_map(|(i, n)| (i != params_idx).then_some(n))
        .ok_or_else(|| TemplateError::Format("no operations sheet".into()))?;
    let ops = wb.worksheet_range(ops_name)?;
    let mut rows = Vec::new();
    let mut skipped_rows = 0;
    // Абсолютные строки листа: заголовок — строка 0, данные — со строки 1 (Excel-строка 2).
    if let (Some((first, _)), Some((last, _))) = (ops.start(), ops.end()) {
        for r in first.max(1)..=last {
            let id = text(at(&ops, r, 0));
            let figures = [
                cell(at(&ops, r, 5)),
                cell(at(&ops, r, 6)),
                cell(at(&ops, r, 7)),
                cell(at(&ops, r, 8)),
                cell(at(&ops, r, 9)),
                cell(at(&ops, r, 10)),
            ];
            let comment = text(at(&ops, r, 11));
            if id.is_empty() {
                // Цифры без ID привязать не к чему: считаем и пропускаем.
                if figures.iter().any(|c| *c != Cell::Empty) || !comment.is_empty() {
                    skipped_rows += 1;
                }
                continue;
            }
            let [avg, peak, error_pct, req_bytes, resp_bytes, p95_ms] = figures;
            rows.push(TemplateRow {
                row: r + 1,
                id,
                avg,
                peak,
                error_pct,
                req_bytes,
                resp_bytes,
                p95_ms,
                comment,
            });
        }
    }
    let p = |i: usize| text(at(&params, (i + 2) as u32, 1));
    let mut values: Vec<String> = (0..PARAMS).map(p).collect();
    let filled_by = values.pop().unwrap_or_default();
    let system = values.pop().unwrap_or_default();
    let period_to = values.pop().unwrap_or_default();
    let period_from = values.pop().unwrap_or_default();
    let stand = values.pop().unwrap_or_default();
    Ok(TemplateRead {
        format,
        project,
        rows,
        skipped_rows,
        source: SourceInfo {
            stand,
            period_from,
            period_to,
            system,
            filled_by,
            imported_at: String::new(),
            file: String::new(),
        },
    })
}
