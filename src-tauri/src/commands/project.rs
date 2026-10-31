//! Команды проекта: создание, открытие, сохранение манифеста, недавние проекты, открытие пути.

use glafira_load_model::project::{MANIFEST_FILE, Manifest, validate};
use tauri::State;

use super::blocking;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::{Project, ProjectDir, RecentProject, create};
use crate::project::paths::{inside, require_absolute, resolve};
use crate::state::AppState;

fn remember(st: &AppState, path: &str) -> AppResult<()> {
    st.settings.remember_project(path)
}

#[tauri::command]
pub async fn project_create(
    state: State<'_, AppState>,
    path: String,
    name: String,
) -> AppResult<Project> {
    let st = state.inner().clone();
    blocking(move || {
        require_absolute(&path)?;
        let p = create(&path, &name)?;
        remember(&st, &p.path)?;
        Ok(p)
    })
    .await
}

/// YAML манифеста не читается — ошибка `PROJECT_INVALID`; манифест с проблемами проверки
/// открывается только для чтения (`readOnly`, `problems`).
#[tauri::command]
pub async fn project_open(state: State<'_, AppState>, path: String) -> AppResult<Project> {
    let st = state.inner().clone();
    blocking(move || {
        require_absolute(&path)?;
        let p = ProjectDir::open(&path)?.view()?;
        remember(&st, &p.path)?;
        Ok(p)
    })
    .await
}

/// «Убрать из списка»: проект на диске не трогается.
#[tauri::command]
pub async fn project_close(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let st = state.inner().clone();
    blocking(move || st.settings.forget_project(&path)).await
}

/// Сохраняет манифест только без проблем проверки; список проблем — JSON в `details`.
#[tauri::command]
pub async fn project_save(path: String, manifest: Manifest) -> AppResult<Project> {
    blocking(move || {
        let problems = validate(&manifest);
        if !problems.is_empty() {
            return Err(app_err!(
                ErrorCode::InvalidInput,
                "errors.project.invalidManifest",
                "The manifest has {{count}} problem(s)",
                count = problems.len()
            )
            .with_details(serde_json::to_string(&problems).unwrap_or_default()));
        }
        let d = ProjectDir::open(&path)?;
        // Манифест на диске не читается или не проходит проверки — не затираем его.
        let on_disk = d.manifest()?;
        if !validate(&on_disk).is_empty() {
            return Err(app_err!(
                ErrorCode::ProjectInvalid,
                "errors.project.readOnly",
                "Project is read-only until its manifest is fixed: {{path}}",
                path = d.root().join(MANIFEST_FILE).display()
            ));
        }
        d.save_manifest(&manifest)?;
        d.view()
    })
    .await
}

fn recent(paths: Vec<String>) -> Vec<RecentProject> {
    paths
        .into_iter()
        .map(
            |path| match ProjectDir::open(&path).and_then(|d| d.manifest()) {
                Ok(m) => RecentProject {
                    path,
                    name: Some(m.name),
                    missing: false,
                },
                Err(_) => RecentProject {
                    path,
                    name: None,
                    missing: true,
                },
            },
        )
        .collect()
}

#[tauri::command]
pub async fn project_recent(state: State<'_, AppState>) -> AppResult<Vec<RecentProject>> {
    let st = state.inner().clone();
    blocking(move || Ok(recent(st.settings.projects().recent))).await
}

/// Путь для `open_path`: внутри проекта или клона одного из его сервисов, иначе `PATH_OUTSIDE_PROJECT`.
fn openable(project: &str, path: &str) -> AppResult<std::path::PathBuf> {
    let d = ProjectDir::open(project)?;
    let m = d.manifest()?;
    let candidate = std::path::PathBuf::from(path);
    let mut roots = vec![d.root().to_path_buf()];
    roots.extend(
        m.services
            .iter()
            .map(|s| resolve(d.root(), &s.path))
            .filter(|p| p.is_dir()),
    );
    roots
        .iter()
        .find_map(|r| inside(r, &candidate).ok())
        .ok_or_else(|| {
            app_err!(
                ErrorCode::PathOutsideProject,
                "errors.project.outside",
                "Path {{path}} is outside {{base}}",
                path = candidate.display(),
                base = d.root().display()
            )
        })
}

/// Открывает файл или папку системным средством, если путь внутри проекта или клона его сервиса.
#[tauri::command]
pub async fn open_path(app: tauri::AppHandle, project: String, path: String) -> AppResult<()> {
    let target = blocking(move || openable(&project, &path)).await?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(target.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::internal_with(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(id: &str, path: &str) -> glafira_load_model::project::Service {
        glafira_load_model::project::Service {
            id: id.into(),
            name: String::new(),
            path: path.into(),
            contracts: vec![],
            kafka: Default::default(),
            extra: Default::default(),
        }
    }

    #[tokio::test]
    async fn save_rejects_invalid_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        let mut m = Manifest::new("x");
        m.services.push(service("Bad", "."));
        let e = project_save(path.to_string_lossy().into_owned(), m)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert!(e.details.unwrap().contains("bad_service_id"));

        let mut m = Manifest::new("renamed");
        m.services.push(service("orders", "../clone"));
        let p = project_save(path.to_string_lossy().into_owned(), m)
            .await
            .unwrap();
        assert_eq!(p.manifest.name, "renamed");
        assert_eq!(p.missing_services, vec!["orders"]);
        let back = ProjectDir::open(path.to_str().unwrap())
            .unwrap()
            .manifest()
            .unwrap();
        assert_eq!(back.name, "renamed", "сохранено на диск");
    }

    #[tokio::test]
    async fn save_does_not_clobber_unreadable_or_read_only_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        let ps = path.to_string_lossy().into_owned();
        create(&ps, "x").unwrap();
        let manifest_file = path.join(MANIFEST_FILE);

        // YAML не читается.
        std::fs::write(&manifest_file, "name: [\n").unwrap();
        let e = project_save(ps.clone(), Manifest::new("new"))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::ProjectInvalid);
        assert_eq!(e.key, "errors.project.invalidFile");
        assert_eq!(
            std::fs::read_to_string(&manifest_file).unwrap(),
            "name: [\n"
        );

        // YAML читается, но не проходит проверки: проект только для чтения.
        let d = ProjectDir::open(&ps).unwrap();
        let mut bad = Manifest::new("x");
        bad.services.push(service("Bad Id", "."));
        d.save_manifest(&bad).unwrap();
        let before = std::fs::read_to_string(&manifest_file).unwrap();
        let e = project_save(ps, Manifest::new("new")).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ProjectInvalid);
        assert_eq!(e.key, "errors.project.readOnly");
        assert_eq!(std::fs::read_to_string(&manifest_file).unwrap(), before);
    }

    #[test]
    fn recent_marks_missing_and_broken_projects() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok");
        create(ok.to_str().unwrap(), "orders").unwrap();
        let broken = dir.path().join("broken");
        create(broken.to_str().unwrap(), "b").unwrap();
        std::fs::write(
            broken.join(glafira_load_model::project::MANIFEST_FILE),
            "name: [\n",
        )
        .unwrap();
        let gone = dir.path().join("gone");
        let list = recent(vec![
            ok.to_string_lossy().into_owned(),
            broken.to_string_lossy().into_owned(),
            gone.to_string_lossy().into_owned(),
        ]);
        assert_eq!(
            (list[0].name.as_deref(), list[0].missing),
            (Some("orders"), false)
        );
        assert!(list[1].missing && list[2].missing);
        assert_eq!(
            serde_json::to_value(&list[0]).unwrap(),
            serde_json::json!({ "path": ok.to_string_lossy(), "name": "orders", "missing": false })
        );
    }

    #[test]
    fn open_path_stays_in_project_or_clones() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p");
        create(p.to_str().unwrap(), "x").unwrap();
        let clone = dir.path().join("clone");
        std::fs::create_dir(&clone).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "s").unwrap();
        let d = ProjectDir::open(p.to_str().unwrap()).unwrap();
        let mut m = d.manifest().unwrap();
        m.services.push(service("orders", "../clone"));
        d.save_manifest(&m).unwrap();
        let project = p.to_str().unwrap();
        assert!(openable(project, p.join("registry").to_str().unwrap()).is_ok());
        assert!(openable(project, clone.to_str().unwrap()).is_ok());
        let e = openable(project, dir.path().join("secret.txt").to_str().unwrap()).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
    }
}
