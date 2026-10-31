//! Папка проекта НТ: манифест и файлы этапа L1 (спека L1 §3). Все записи — через `atomic_write_shared`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use glafira_load_model::prod_load::{PROD_LOAD_FILE, ProdLoad};
use glafira_load_model::profile::{PROFILE_FILE, ProfileFile};
use glafira_load_model::project::{MANIFEST_FILE, Manifest, ManifestProblem, validate};
use glafira_load_model::registry::{REGISTRY_FILE, Registry};
use glafira_load_model::{ModelError, from_yaml, to_yaml};
use serde::Serialize;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::paths::canonical;
use crate::project::paths::{inside, resolve};
use crate::storage::fs::atomic_write_shared;
use crate::sync::MutexExt;

/// Блокировки записи реестра по каноническому корню проекта. Общие на процесс (а не в
/// `AppState`): ими пользуются блокирующие функции разбора и команд реестра и их тесты.
static REGISTRY_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(Mutex::default);

/// Проект для фронтенда: манифест, проблемы проверки и сервисы без папки клона.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub manifest: Manifest,
    pub problems: Vec<ManifestProblem>,
    /// Есть проблемы проверки: запись заблокирована, пока их не исправят в файле.
    pub read_only: bool,
    pub missing_services: Vec<String>,
}

/// Строка списка недавних проектов; `missing` — папки или манифеста больше нет (или он не читается).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: Option<String>,
    pub missing: bool,
}

/// Папка проекта: чтение и атомарная запись файлов только внутри неё.
#[derive(Debug)]
pub struct ProjectDir {
    /// Канонический корень (`dunce`): с ним сравнивают результаты `paths::inside`.
    root: PathBuf,
}

/// Файл проекта есть, но YAML не разбирается: `PROJECT_INVALID`, строка — в `details`.
fn invalid(file: &str, e: ModelError) -> AppError {
    let err = app_err!(
        ErrorCode::ProjectInvalid,
        "errors.project.invalidFile",
        "{{file}}: {{error}}",
        file = file,
        error = e.message
    );
    match e.line {
        Some(l) => err.with_details(format!("line {l}")),
        None => err,
    }
}

fn no_manifest(path: &impl std::fmt::Display) -> AppError {
    app_err!(
        ErrorCode::NotFound,
        "errors.project.notFound",
        "No {{file}} in {{path}}",
        file = MANIFEST_FILE,
        path = path
    )
}

impl ProjectDir {
    /// Папка с манифестом; без манифеста — `NOT_FOUND`.
    pub fn open(path: &str) -> AppResult<Self> {
        let raw = PathBuf::from(path);
        if !raw.join(MANIFEST_FILE).is_file() {
            return Err(no_manifest(&path));
        }
        Ok(Self {
            root: canonical(&raw)?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn file(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Текст файла проекта; нет файла — `None`.
    pub fn read_text(&self, rel: &str) -> AppResult<Option<String>> {
        let p = inside(&self.root, &self.file(rel))?;
        match std::fs::read_to_string(&p) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::from(e).with_details(p.display().to_string())),
        }
    }

    /// Атомарная запись с обычными правами (файлы проекта коммитят); путь вне проекта —
    /// `PATH_OUTSIDE_PROJECT`.
    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) -> AppResult<()> {
        let p = inside(&self.root, &self.file(rel))?;
        atomic_write_shared(&p, bytes)
    }

    pub fn write_text(&self, rel: &str, text: &str) -> AppResult<()> {
        self.write_bytes(rel, text.as_bytes())
    }

    fn read_yaml<T: serde::de::DeserializeOwned>(&self, rel: &str) -> AppResult<Option<T>> {
        self.read_text(rel)?
            .map(|t| from_yaml(&t).map_err(|e| invalid(rel, e)))
            .transpose()
    }

    pub fn manifest(&self) -> AppResult<Manifest> {
        self.read_yaml(MANIFEST_FILE)?
            .ok_or_else(|| no_manifest(&self.root.display()))
    }

    pub fn save_manifest(&self, m: &Manifest) -> AppResult<()> {
        self.write_text(MANIFEST_FILE, &to_yaml(m))
    }

    /// Реестр операций; без файла — пустой.
    pub fn registry(&self) -> AppResult<Registry> {
        Ok(self.read_yaml(REGISTRY_FILE)?.unwrap_or_default())
    }

    pub fn save_registry(&self, r: &Registry) -> AppResult<()> {
        self.write_text(REGISTRY_FILE, &to_yaml(r))
    }

    /// Выполняет «прочитать реестр → изменить → записать» под блокировкой проекта: правка ручных
    /// полей во время слияния разбора не теряется (кто второй — читает уже записанный файл).
    /// Ключ — канонический корень, поэтому разные написания пути дают одну блокировку.
    pub fn with_registry_lock<T>(&self, f: impl FnOnce() -> T) -> T {
        let lock = REGISTRY_LOCKS
            .lock_safe()
            .entry(self.root.clone())
            .or_default()
            .clone();
        let _guard = lock.lock_safe();
        f()
    }

    /// Нагрузка ПРОМ; без файла — пустая.
    pub fn prod_load(&self) -> AppResult<ProdLoad> {
        Ok(self.read_yaml(PROD_LOAD_FILE)?.unwrap_or_default())
    }

    pub fn save_prod_load(&self, p: &ProdLoad) -> AppResult<()> {
        self.write_text(PROD_LOAD_FILE, &to_yaml(p))
    }

    /// Профили; без файла — три профиля по умолчанию.
    pub fn profile(&self) -> AppResult<ProfileFile> {
        Ok(self.read_yaml(PROFILE_FILE)?.unwrap_or_default())
    }

    pub fn save_profile(&self, p: &ProfileFile) -> AppResult<()> {
        self.write_text(PROFILE_FILE, &to_yaml(p))
    }

    /// Папка клона сервиса: существует и является каталогом.
    pub fn service_root(&self, m: &Manifest, service_id: &str) -> AppResult<PathBuf> {
        let s = m.service(service_id).ok_or_else(|| {
            app_err!(
                ErrorCode::NotFound,
                "errors.project.noService",
                "Service {{id}} is not in the manifest",
                id = service_id
            )
        })?;
        let p = resolve(&self.root, &s.path);
        if !p.is_dir() {
            return Err(app_err!(
                ErrorCode::NotFound,
                "errors.project.serviceDirMissing",
                "Folder of service {{id}} not found: {{path}}",
                id = service_id,
                path = p.display()
            ));
        }
        Ok(p)
    }

    /// Манифест с проверками: YAML не читается — ошибка `PROJECT_INVALID`; не проходит
    /// проверки — проект только для чтения со списком проблем.
    pub fn view(&self) -> AppResult<Project> {
        let manifest = self.manifest()?;
        let problems = validate(&manifest);
        let missing_services = manifest
            .services
            .iter()
            .filter(|s| !resolve(&self.root, &s.path).is_dir())
            .map(|s| s.id.clone())
            .collect();
        Ok(Project {
            path: self.root.to_string_lossy().into_owned(),
            read_only: !problems.is_empty(),
            manifest,
            problems,
            missing_services,
        })
    }
}

/// Новый проект: папка не существует или пуста. Без `git init` — проект кладут в свой репозиторий.
pub fn create(path: &str, name: &str) -> AppResult<Project> {
    if name.trim().is_empty() {
        return Err(app_err!(
            ErrorCode::InvalidInput,
            "errors.project.emptyName",
            "Project name cannot be empty"
        ));
    }
    let root = PathBuf::from(path);
    if root.exists() && std::fs::read_dir(&root)?.next().is_some() {
        return Err(app_err!(
            ErrorCode::InvalidInput,
            "errors.project.dirNotEmpty",
            "Folder {{path}} is not empty",
            path = path
        ));
    }
    std::fs::create_dir_all(root.join("registry/scans"))?;
    std::fs::create_dir_all(root.join("load/imports"))?;
    // Дальше всё идёт от канонического корня (как в `ProjectDir::open`).
    let root = canonical(&root)?;
    atomic_write_shared(
        &root.join(MANIFEST_FILE),
        to_yaml(&Manifest::new(name.trim())).as_bytes(),
    )?;
    atomic_write_shared(
        &root.join(REGISTRY_FILE),
        to_yaml(&Registry::default()).as_bytes(),
    )?;
    atomic_write_shared(&root.join(".gitignore"), b"runs/\n")?;
    ProjectDir::open(&root.to_string_lossy())?.view()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(id: &str, path: &str) -> glafira_load_model::project::Service {
        glafira_load_model::project::Service {
            id: id.into(),
            name: "Orders".into(),
            path: path.into(),
            contracts: vec![],
            kafka: Default::default(),
            extra: Default::default(),
        }
    }

    #[test]
    fn create_open_save_and_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        let p = create(path.to_str().unwrap(), "orders").unwrap();
        assert_eq!(p.manifest.name, "orders");
        assert!(!p.read_only && p.problems.is_empty());
        assert!(path.join(".gitignore").exists());
        assert!(path.join("registry/operations.yaml").is_file());
        assert!(path.join("load/imports").is_dir());
        assert!(
            create(path.to_str().unwrap(), "again").is_err(),
            "папка не пуста"
        );
        assert_eq!(
            create(dir.path().join("q").to_str().unwrap(), "  ")
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        let mut m = d.manifest().unwrap();
        m.services.push(service("orders", "../nope"));
        d.save_manifest(&m).unwrap();
        let v = d.view().unwrap();
        assert_eq!(v.missing_services, vec!["orders"]);
        assert_eq!(
            d.service_root(&m, "orders").unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            d.service_root(&m, "absent").unwrap_err().key,
            "errors.project.noService"
        );
        assert_eq!(d.registry().unwrap().operations.len(), 0);
        assert_eq!(
            d.profile().unwrap().profiles.len(),
            3,
            "профиль по умолчанию без файла"
        );

        std::fs::write(path.join(MANIFEST_FILE), "format: 1\nname: [\n").unwrap();
        let e = d.manifest().unwrap_err();
        assert_eq!(e.code, ErrorCode::ProjectInvalid);
        assert!(
            e.details.as_deref().unwrap_or("").starts_with("line "),
            "{e:?}"
        );
        assert_eq!(d.view().unwrap_err().code, ErrorCode::ProjectInvalid);
        assert_eq!(
            ProjectDir::open(dir.path().join("missing").to_str().unwrap())
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn invalid_manifest_opens_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        std::fs::create_dir(dir.path().join("clone")).unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        let mut m = d.manifest().unwrap();
        m.services.push(service("Bad Id", "../clone"));
        d.save_manifest(&m).unwrap();
        let v = d.view().unwrap();
        assert!(v.read_only);
        assert_eq!(v.problems.len(), 1, "{:?}", v.problems);
        assert!(v.missing_services.is_empty(), "клон на месте");
        assert_eq!(
            d.service_root(&m, "Bad Id").unwrap(),
            d.root().join("../clone"),
            "клон может лежать вне папки проекта"
        );
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["readOnly"], true);
        assert!(json["missingServices"].is_array());
    }

    #[test]
    fn writes_stay_inside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        assert_eq!(
            d.write_text("../escape.txt", "no").unwrap_err().code,
            ErrorCode::PathOutsideProject
        );
        assert!(!dir.path().join("escape.txt").exists());
        assert_eq!(
            d.read_text("../p/../escape.txt").unwrap_err().code,
            ErrorCode::PathOutsideProject
        );
        d.write_text("load/profile.yaml", "format: 1\n").unwrap();
        assert!(d.read_text("load/profile.yaml").unwrap().is_some());
        assert!(d.read_text("load/prod-load.yaml").unwrap().is_none());
        d.write_bytes("load/imports/new/x.xlsx", b"PK").unwrap();
        assert!(path.join("load/imports/new/x.xlsx").is_file());
    }

    #[test]
    fn root_is_canonical_and_equal_for_trailing_slash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        let p = create(path.to_str().unwrap(), "x").unwrap();
        let canon = dunce::canonicalize(&path).unwrap();
        assert_eq!(p.path, canon.to_string_lossy());
        let with_slash = format!("{}/", path.display());
        let d = ProjectDir::open(&with_slash).unwrap();
        assert_eq!(d.root(), canon);
        assert_eq!(
            d.view().unwrap().path,
            p.path,
            "один путь для списка недавних"
        );
        // Путь внутри проекта сохраняется относительным.
        let inner = inside(d.root(), &d.file("registry/operations.yaml")).unwrap();
        assert_eq!(
            crate::project::paths::relative(d.root(), &inner),
            "registry/operations.yaml"
        );
    }

    #[cfg(unix)]
    #[test]
    fn project_files_are_not_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        d.write_text("load/profile.yaml", "format: 1\n").unwrap();
        for f in [MANIFEST_FILE, "load/profile.yaml", ".gitignore"] {
            let mode = std::fs::metadata(path.join(f))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_ne!(mode, 0o600, "{f}");
        }
    }
}
