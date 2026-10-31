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

/// Пустая и состоящая из пробелов ячейка — `Cell::Empty`; текст, похожий на число
/// (в том числе с запятой), — число.
fn cell(d: Option<&Data>) -> Cell {
    match d {
        Some(Data::Float(f)) => Cell::Number { value: *f },
        Some(Data::Int(i)) => Cell::Number { value: *i as f64 },
        Some(Data::Empty) | None => Cell::Empty,
        Some(Data::String(s)) if s.trim().is_empty() => Cell::Empty,
        Some(Data::String(s)) => match s.trim().replace(',', ".").parse::<f64>() {
            Ok(v) => Cell::Number { value: v },
            Err(_) => Cell::Text {
                value: s.trim().to_string(),
            },
        },
        other => Cell::Text { value: text(other) },
    }
}

fn marker(params: &Range<Data>) -> Result<(u32, String), TemplateError> {
    let a1 = text(params.get((0, 0)));
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
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))?;
    let names = wb.sheet_names().to_vec();
    if names.len() < 2 {
        return Err(TemplateError::NoMarker);
    }
    let params = wb.worksheet_range(&names[1])?;
    let (format, project) = marker(&params)?;
    let ops = wb.worksheet_range(&names[0])?;
    let mut rows = Vec::new();
    for (i, r) in ops.rows().enumerate().skip(1) {
        let id = text(r.first());
        if id.is_empty() {
            continue;
        }
        rows.push(TemplateRow {
            row: (i + 1) as u32,
            id,
            avg: cell(r.get(5)),
            peak: cell(r.get(6)),
            error_pct: cell(r.get(7)),
            req_bytes: cell(r.get(8)),
            resp_bytes: cell(r.get(9)),
            p95_ms: cell(r.get(10)),
            comment: text(r.get(11)),
        });
    }
    let p = |i: usize| text(params.get((i + 2, 1)));
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
