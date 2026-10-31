//! Команды нагрузки ПРОМ и профилей: шаблон Excel, предпросмотр и применение импорта, расчёт.

use std::path::PathBuf;

use glafira_load_model::prod_load::ProdLoad;
use glafira_load_model::profile::{Computed, ProfileFile};
use glafira_load_xlsx::TemplateLabels;

use super::blocking;
use crate::error::AppResult;
use crate::load::template::{TemplatePreview, export, preview};
use crate::load::{import, profile};
use crate::project::manifest::ProjectDir;

/// Выгрузка шаблона в файл, выбранный пользователем; `prefill` — с текущими цифрами.
/// Возвращает число выгруженных операций.
#[tauri::command]
pub async fn prod_load_export_template(
    path: String,
    file: String,
    labels: TemplateLabels,
    prefill: bool,
) -> AppResult<usize> {
    blocking(move || {
        export(
            &ProjectDir::open(&path)?,
            &PathBuf::from(file),
            &labels,
            prefill,
        )
    })
    .await
}

#[tauri::command]
pub async fn prod_load_preview_import(path: String, file: String) -> AppResult<TemplatePreview> {
    blocking(move || preview(&ProjectDir::open(&path)?, &PathBuf::from(file))).await
}

/// Применяет импорт: повторный разбор (файл могли поменять), копия в `load/imports/`,
/// запись `prod-load.yaml`.
#[tauri::command]
pub async fn prod_load_apply_import(path: String, file: String) -> AppResult<ProdLoad> {
    blocking(move || import::apply(&ProjectDir::open(&path)?, &PathBuf::from(file))).await
}

#[tauri::command]
pub async fn prod_load_get(path: String) -> AppResult<ProdLoad> {
    blocking(move || ProjectDir::open(&path)?.prod_load()).await
}

#[tauri::command]
pub async fn profile_get(path: String) -> AppResult<ProfileFile> {
    blocking(move || ProjectDir::open(&path)?.profile()).await
}

#[tauri::command]
pub async fn profile_save(path: String, profile: ProfileFile) -> AppResult<ProfileFile> {
    blocking(move || {
        profile::save(&ProjectDir::open(&path)?, &profile)?;
        Ok(profile)
    })
    .await
}

#[tauri::command]
pub async fn profile_compute(path: String, profile_name: String) -> AppResult<Computed> {
    blocking(move || profile::calculate(&ProjectDir::open(&path)?, &profile_name)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::load::template::tests::{labels, project_with_ops};
    use glafira_load_model::prod_load::Figures;

    #[tokio::test]
    async fn apply_import_copies_the_file_and_computes_a_profile() {
        let (tmp, d) = project_with_ops();
        let path = d.root().to_string_lossy().into_owned();
        let mut prod = ProdLoad::default();
        prod.operations.insert(
            "http:orders:POST /v1/orders".into(),
            Figures {
                avg: 100.0,
                peak: 300.0,
                ..Default::default()
            },
        );
        prod.operations.insert(
            "kafka_consume:orders.created".into(),
            Figures {
                avg: 10.0,
                peak: 20.0,
                lag_max_s: Some(5.0),
                ..Default::default()
            },
        );
        d.save_prod_load(&prod).unwrap();
        let file = tmp.path().join("filled.xlsx");
        let file_s = file.to_string_lossy().into_owned();
        assert_eq!(
            prod_load_export_template(path.clone(), file_s.clone(), labels(), true)
                .await
                .unwrap(),
            2
        );
        d.save_prod_load(&ProdLoad::default()).unwrap();
        let preview = prod_load_preview_import(path.clone(), file_s.clone())
            .await
            .unwrap();
        assert_eq!(preview.preview.filled.len(), 2);
        let applied = prod_load_apply_import(path.clone(), file_s.clone())
            .await
            .unwrap();
        assert!(
            applied.source.file.starts_with("load/imports/")
                && applied.source.file.ends_with(".xlsx")
                && d.file(&applied.source.file).is_file()
        );
        assert!(!applied.source.imported_at.is_empty());
        let got = prod_load_get(path.clone()).await.unwrap();
        assert_eq!(
            got.source.file, applied.source.file,
            "source записан в yaml"
        );
        assert_eq!(
            got.operations["kafka_consume:orders.created"].lag_max_s,
            Some(5.0)
        );
        // Повторный импорт в ту же секунду не затирает прежнюю копию.
        let again = prod_load_apply_import(path.clone(), file_s).await.unwrap();
        assert_ne!(again.source.file, applied.source.file);
        assert!(d.file(&applied.source.file).is_file());

        let c = profile_compute(path.clone(), "peak".into()).await.unwrap();
        assert_eq!((c.targets.len(), c.total), (2, 320.0));
        assert_eq!(
            profile_compute(path.clone(), "nope".into())
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let mut f = profile_get(path.clone()).await.unwrap();
        f.coefficients.k_env = 0.5;
        profile_save(path.clone(), f).await.unwrap();
        assert_eq!(
            profile_compute(path, "peak".into()).await.unwrap().total,
            160.0
        );
    }

    #[tokio::test]
    async fn invalid_cells_block_apply_and_write_nothing() {
        let (tmp, d) = project_with_ops();
        let path = d.root().to_string_lossy().into_owned();
        // Пик ниже среднего — ячейка ошибочна.
        let mut prod = ProdLoad::default();
        prod.operations.insert(
            "http:orders:POST /v1/orders".into(),
            Figures {
                avg: 100.0,
                peak: 50.0,
                ..Default::default()
            },
        );
        d.save_prod_load(&prod).unwrap();
        let file = tmp.path().join("bad.xlsx");
        let file_s = file.to_string_lossy().into_owned();
        prod_load_export_template(path.clone(), file_s.clone(), labels(), true)
            .await
            .unwrap();
        d.save_prod_load(&ProdLoad::default()).unwrap();
        let e = prod_load_apply_import(path.clone(), file_s)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::TemplateInvalid);
        assert_eq!(e.params["count"], "1");
        assert!(
            std::fs::read_dir(d.file("load/imports"))
                .unwrap()
                .next()
                .is_none()
        );
        assert!(prod_load_get(path).await.unwrap().operations.is_empty());
    }
}
