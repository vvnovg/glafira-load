//! Запись защищённого шаблона: A–E заблокированы, F–L открыты для заполнения.

use std::path::Path;

use glafira_load_model::prod_load::Figures;
use rust_xlsxwriter::{
    Color, DataValidation, DataValidationRule, Format, Note, Workbook, Worksheet,
};

use crate::{COLUMNS, MARKER_PREFIX, PARAMS, TEMPLATE_FORMAT, TemplateError, TemplateLabels};

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRowOut {
    pub id: String,
    pub service: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
    /// Текущие цифры (решение плана 4): шаблон можно выгрузить заполненным.
    pub figures: Option<Figures>,
}

const WIDTHS: [f64; COLUMNS] = [
    40.0, 14.0, 14.0, 28.0, 36.0, 14.0, 14.0, 14.0, 18.0, 18.0, 14.0, 40.0,
];

fn check_labels(labels: &TemplateLabels) -> Result<(), TemplateError> {
    if labels.columns.len() != COLUMNS
        || labels.notes.len() != COLUMNS
        || labels.params.len() != PARAMS
    {
        return Err(TemplateError::Format(format!(
            "labels: {COLUMNS} columns, {COLUMNS} notes, {PARAMS} params expected"
        )));
    }
    Ok(())
}

fn number(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    v: Option<f64>,
    f: &Format,
) -> Result<(), TemplateError> {
    match v {
        Some(v) => ws.write_number_with_format(row, col, v, f)?,
        None => ws.write_blank(row, col, f)?,
    };
    Ok(())
}

pub fn write_template(
    path: &Path,
    project: &str,
    rows: &[TemplateRowOut],
    labels: &TemplateLabels,
) -> Result<(), TemplateError> {
    check_labels(labels)?;
    let mut wb = Workbook::new();
    let header = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xDDE3EA))
        .set_locked();
    let locked = Format::new()
        .set_background_color(Color::RGB(0xF2F4F7))
        .set_locked();
    let unlocked = Format::new().set_unlocked();
    {
        let ws = wb.add_worksheet();
        ws.set_name(&labels.sheet_operations)?;
        for (c, title) in labels.columns.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, title, &header)?;
            ws.set_column_width(c as u16, WIDTHS[c])?;
            if !labels.notes[c].is_empty() {
                ws.insert_note(0, c as u16, &Note::new(&labels.notes[c]))?;
            }
        }
        for (i, r) in rows.iter().enumerate() {
            let row = (i + 1) as u32;
            for (c, v) in [&r.id, &r.service, &r.kind, &r.name, &r.summary]
                .into_iter()
                .enumerate()
            {
                ws.write_string_with_format(row, c as u16, v, &locked)?;
            }
            let f = r.figures.as_ref();
            // У kafka_consume последняя числовая колонка — допустимый лаг, а не p95.
            let consume = r.kind == "kafka_consume";
            number(ws, row, 5, f.map(|f| f.avg), &unlocked)?;
            number(ws, row, 6, f.map(|f| f.peak), &unlocked)?;
            number(ws, row, 7, f.and_then(|f| f.error_pct), &unlocked)?;
            number(
                ws,
                row,
                8,
                f.and_then(|f| f.req_bytes).map(|v| v as f64),
                &unlocked,
            )?;
            number(
                ws,
                row,
                9,
                f.and_then(|f| f.resp_bytes).map(|v| v as f64),
                &unlocked,
            )?;
            number(
                ws,
                row,
                10,
                f.and_then(|f| if consume { f.lag_max_s } else { f.p95_ms }),
                &unlocked,
            )?;
            ws.write_string_with_format(
                row,
                11,
                f.map(|f| f.comment.as_str()).unwrap_or(""),
                &unlocked,
            )?;
        }
        let last = rows.len() as u32;
        if last > 0 {
            let non_negative = DataValidation::new()
                .allow_decimal_number(DataValidationRule::GreaterThanOrEqualTo(0.0));
            for c in [5u16, 6, 8, 9, 10] {
                ws.add_data_validation(1, c, last, c, &non_negative)?;
            }
            let percent =
                DataValidation::new().allow_decimal_number(DataValidationRule::Between(0.0, 100.0));
            ws.add_data_validation(1, 7, last, 7, &percent)?;
            ws.autofilter(0, 0, last, (COLUMNS - 1) as u16)?;
        }
        ws.set_freeze_panes(1, 0)?;
        ws.protect();
    }
    {
        let ws = wb.add_worksheet();
        ws.set_name(&labels.sheet_params)?;
        ws.write_string(0, 0, format!("{MARKER_PREFIX}{TEMPLATE_FORMAT}:{project}"))?;
        ws.set_column_width(0, 24.0)?;
        ws.set_column_width(1, 40.0)?;
        for (i, label) in labels.params.iter().enumerate() {
            ws.write_string_with_format((i + 2) as u32, 0, label, &header)?;
            ws.write_blank((i + 2) as u32, 1, &unlocked)?;
        }
    }
    {
        let ws = wb.add_worksheet();
        ws.set_name(&labels.sheet_help)?;
        ws.set_column_width(0, 100.0)?;
        for (i, line) in labels.help_lines.iter().enumerate() {
            ws.write_string(i as u32, 0, line)?;
        }
    }
    wb.save(path)?;
    Ok(())
}
