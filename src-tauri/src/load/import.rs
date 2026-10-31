//! Применение импорта: копия заполненного шаблона в проекте и запись `load/prod-load.yaml`.

use std::path::Path;

use glafira_load_model::prod_load::{IMPORTS_DIR, ProdLoad};

use super::template::{preview_bytes, read_file};
use crate::error::{AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::scan::run::now_label;

/// Свободное имя копии: `load/imports/<метка>.xlsx`, при совпадении метки — `<метка>-2.xlsx` и далее.
fn copy_name(d: &ProjectDir) -> String {
    let label = now_label();
    let mut name = format!("{IMPORTS_DIR}/{label}.xlsx");
    let mut n = 2;
    while d.file(&name).exists() {
        name = format!("{IMPORTS_DIR}/{label}-{n}.xlsx");
        n += 1;
    }
    name
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
    let copy = copy_name(d);
    d.write_bytes(&copy, &bytes)?;
    result.source.file = copy;
    d.save_prod_load(&result)?;
    Ok(result)
}
