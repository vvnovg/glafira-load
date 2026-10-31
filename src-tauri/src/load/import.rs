//! Применение импорта: копия заполненного шаблона в проекте и запись `load/prod-load.yaml`.

use std::path::Path;

use glafira_load_model::prod_load::{IMPORTS_DIR, ProdLoad};

use super::template::{preview_bytes, read_file};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::scan::run::now_label;

/// Сколько суффиксов пробуем, прежде чем сдаться (защита от бесконечного цикла).
const MAX_COPY_ATTEMPTS: u32 = 1000;

/// Свободное имя копии: `load/imports/<метка>.xlsx`, при совпадении метки — `<метка>-2.xlsx` и далее
/// (не больше `MAX_COPY_ATTEMPTS` вариантов).
fn copy_name(d: &ProjectDir) -> AppResult<String> {
    let label = now_label();
    let name = format!("{IMPORTS_DIR}/{label}.xlsx");
    if !d.file(&name).exists() {
        return Ok(name);
    }
    for n in 2..=MAX_COPY_ATTEMPTS {
        let name = format!("{IMPORTS_DIR}/{label}-{n}.xlsx");
        if !d.file(&name).exists() {
            return Ok(name);
        }
    }
    Err(AppError::internal_with(format!(
        "no free name for the import copy in {IMPORTS_DIR} after {MAX_COPY_ATTEMPTS} attempts (label {label})"
    )))
}

/// Файл читается один раз: проверяется и копируется ровно то, что прочитано. Есть ошибочные
/// ячейки — `TEMPLATE_INVALID`, ничего не пишется. Запись в проект проходит границу проекта.
pub fn apply(d: &ProjectDir, src: &Path) -> AppResult<ProdLoad> {
    let bytes = read_file(src)?;
    let p = preview_bytes(d, &bytes)?.preview;
    let Some(mut result) = p.result else {
        return Err(app_err!(
            ErrorCode::TemplateInvalid,
            "errors.template.invalidRows",
            "The template has {{count}} invalid cell(s)",
            count = p.invalid.len()
        ));
    };
    let copy = copy_name(d)?;
    d.write_bytes(&copy, &bytes)?;
    result.source.file = copy;
    d.save_prod_load(&result)?;
    Ok(result)
}
