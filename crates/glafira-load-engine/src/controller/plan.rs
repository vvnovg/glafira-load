//! Чтение проекта для прогона контроллером: манифест, стенд, сьют с `check`, профиль и цели,
//! схемы для агентов (спека §4.5 п. 1). Чтение и проверка — те же функции, что у локального
//! прогона (`local::{load_project, load_suite, compute_profile}`).

use std::path::{Path, PathBuf};

use glafira_load_model::suite::{SUITE_FILE, SUITES_DIR};

use super::{EngineError, RunPlan};
use crate::agent_api::Mode;
use crate::local::{
    avro_schema_texts, compute_profile, load_project, load_suite, response_schemas_of,
};

/// Параметры запуска: проект, сьют, профиль, стенд и ключи CLI/окна прогона.
#[derive(Debug, Clone)]
pub struct PlanArgs {
    pub root: PathBuf,
    /// `None` — единственный сьют проекта.
    pub suite: Option<String>,
    pub profile: String,
    pub env: String,
    pub mode: Mode,
    pub max_in_flight: Option<u32>,
    /// Папка результата; `None` — `runs/<дата>/` в проекте.
    pub out_dir: Option<PathBuf>,
}

/// Имя сьюта: заданное (с проверкой файла) или единственное в `suites/`; ни одного или несколько
/// без имени — `NOT_FOUND` (несколько — с именами в `names`).
pub fn resolve_suite(root: &Path, name: Option<&str>) -> Result<String, EngineError> {
    if let Some(n) = name {
        if root.join(SUITES_DIR).join(n).join(SUITE_FILE).is_file() {
            return Ok(n.to_string());
        }
        return Err(EngineError::new(
            "NOT_FOUND",
            format!("suite `{n}` not found in {SUITES_DIR}/"),
        )
        .with_names(vec![n.into()]));
    }
    let mut names: Vec<String> = match std::fs::read_dir(root.join(SUITES_DIR)) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join(SUITE_FILE).is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    };
    names.sort();
    match names.len() {
        1 => Ok(names.remove(0)),
        0 => Err(EngineError::new(
            "NOT_FOUND",
            format!("the project has no suites in {SUITES_DIR}/"),
        )),
        _ => Err(EngineError::new(
            "NOT_FOUND",
            "the project has several suites, pass the suite name",
        )
        .with_names(names)),
    }
}

/// Читает проект, выполняет `check` сьюта и считает цели профиля; схемы ответов — из реестра,
/// тексты `.avsc` — из клонов сервисов. Ошибки: `NOT_FOUND` (манифест, стенд, сьют, профиль),
/// `SUITE_INVALID` (находки `check` в `names`), `INVALID_INPUT`, `IO`.
pub fn load_plan(a: PlanArgs) -> Result<RunPlan, EngineError> {
    let root = a.root;
    let project = load_project(&root)?;
    let env = project
        .manifest
        .environments
        .iter()
        .find(|e| e.id == a.env)
        .cloned()
        .ok_or_else(|| {
            EngineError::new(
                "NOT_FOUND",
                format!("environment `{}` is not in the manifest", a.env),
            )
            .with_names(vec![a.env.clone()])
        })?;
    let suite_name = resolve_suite(&root, a.suite.as_deref())?;
    let loaded = load_suite(&root, &suite_name, &project.registry, Some(&env))?;
    let computed = compute_profile(&project, &a.profile)?;
    let response_schemas = response_schemas_of(&project.registry);
    let avro_schemas =
        avro_schema_texts(&root, &project.manifest, &project.registry, &loaded.suite)?;
    Ok(RunPlan {
        root,
        suite_name,
        suite: loaded.suite,
        suite_text: loaded.text,
        profile: a.profile,
        computed,
        env,
        mode: a.mode,
        max_in_flight: a.max_in_flight,
        out_dir: a.out_dir,
        response_schemas,
        avro_schemas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::prod_load::{Figures, PROD_LOAD_FILE, ProdLoad};
    use glafira_load_model::profile::{Basis, PROFILE_FILE, Phase, Profile, ProfileFile};
    use glafira_load_model::project::MANIFEST_FILE;
    use glafira_load_model::registry::{
        Address, Manual, OpKind, Operation, OperationDraft, REGISTRY_FILE, Registry, Seen, Source,
        Status,
    };
    use glafira_load_model::to_yaml;

    const MANIFEST: &str = "format: 1
name: p
services: [{ id: orders, path: . }]
environments:
  - id: test
    kind: test
    http: http://127.0.0.1:1
    agents: [{ id: a, url: http://127.0.0.1:2 }]
";

    const SUITE: &str = "format: 1
name: orders
data:
  users: { path: data/users.csv, mode: slice }
scenarios:
  - id: ping
    steps:
      - http: { operation: \"http:orders:GET /ping\", query: { user: \"{{users.id}}\" } }
        expect: { status: 200 }
";

    /// Проект с одним сьютом, реестром из одной операции и профилем `short` (≈3 с).
    fn project(root: &Path) {
        std::fs::write(root.join(MANIFEST_FILE), MANIFEST).unwrap();
        let draft = OperationDraft::new(
            "orders",
            OpKind::Http,
            Address::Http {
                method: "GET".into(),
                path: "/ping".into(),
            },
            Source::manual(),
        );
        let mut registry = Registry::default();
        registry.operations.push(Operation {
            fingerprint: draft.fingerprint(),
            draft,
            status: Status::Active,
            first_seen: Seen::default(),
            last_seen: Seen {
                version: "1.0.0".into(),
                commit: "c".into(),
                at: "2026-11-01T00:00:00Z".into(),
            },
            manual: Manual::default(),
        });
        std::fs::create_dir_all(root.join("registry")).unwrap();
        std::fs::write(root.join(REGISTRY_FILE), to_yaml(&registry)).unwrap();
        let mut prod = ProdLoad::default();
        prod.operations.insert(
            "http:orders:GET /ping".into(),
            Figures {
                avg: 20.0,
                peak: 40.0,
                ..Default::default()
            },
        );
        std::fs::create_dir_all(root.join("load")).unwrap();
        std::fs::write(root.join(PROD_LOAD_FILE), to_yaml(&prod)).unwrap();
        let mut profile = ProfileFile::default();
        profile.profiles.insert(
            "short".into(),
            Profile {
                basis: Basis::Avg,
                phases: vec![
                    Phase::Ramp {
                        minutes: 0.02,
                        to_pct: 100.0,
                    },
                    Phase::Steady { minutes: 0.03 },
                ],
            },
        );
        std::fs::write(root.join(PROFILE_FILE), to_yaml(&profile)).unwrap();
        std::fs::create_dir_all(root.join("suites/orders")).unwrap();
        std::fs::write(root.join("suites/orders/suite.yaml"), SUITE).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data/users.csv"), "id,name\n1,a\n2,b\n3,c\n").unwrap();
    }

    fn args(root: &Path) -> PlanArgs {
        PlanArgs {
            root: root.to_path_buf(),
            suite: None,
            profile: "short".into(),
            env: "test".into(),
            mode: Mode::Run,
            max_in_flight: Some(5),
            out_dir: None,
        }
    }

    #[test]
    fn loads_checks_and_computes() {
        let tmp = tempfile::tempdir().unwrap();
        project(tmp.path());
        let plan = load_plan(args(tmp.path())).unwrap();
        assert_eq!(
            (
                plan.suite_name.as_str(),
                plan.env.id.as_str(),
                plan.max_in_flight
            ),
            ("orders", "test", Some(5))
        );
        assert_eq!(plan.suite_text, SUITE);
        assert_eq!(plan.computed.targets[0].target, 20.0);
        assert!((plan.computed.total_minutes - 0.05).abs() < 1e-9);
        assert_eq!(plan.suite.data["users"].path, "data/users.csv");
        assert!(
            plan.response_schemas.is_empty() && plan.avro_schemas.is_empty(),
            "операция без схемы ответа и без avro"
        );
        assert_eq!(
            super::super::service_versions(tmp.path())["orders"],
            "1.0.0"
        );
        assert_eq!(super::super::project_commit(tmp.path()), None);
        std::fs::create_dir_all(tmp.path().join(".git/refs/heads")).unwrap();
        std::fs::write(tmp.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(tmp.path().join(".git/refs/heads/main"), "abc123\n").unwrap();
        assert_eq!(
            super::super::project_commit(tmp.path()).as_deref(),
            Some("abc123")
        );
        let mut named = args(tmp.path());
        named.suite = Some("orders".into());
        assert_eq!(load_plan(named).unwrap().suite_name, "orders");
    }

    #[test]
    fn reports_missing_pieces_and_check_errors() {
        let tmp = tempfile::tempdir().unwrap();
        project(tmp.path());
        let mut a = args(tmp.path());
        a.env = "prod".into();
        let e = load_plan(a).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.names.as_slice()),
            ("NOT_FOUND", &["prod".to_string()][..])
        );
        let mut a = args(tmp.path());
        a.profile = "nope".into();
        assert_eq!(load_plan(a).unwrap_err().code, "NOT_FOUND");
        let mut a = args(tmp.path());
        a.suite = Some("other".into());
        assert_eq!(load_plan(a).unwrap_err().code, "NOT_FOUND");
        std::fs::create_dir_all(tmp.path().join("suites/second")).unwrap();
        std::fs::write(tmp.path().join("suites/second/suite.yaml"), SUITE).unwrap();
        let e = load_plan(args(tmp.path())).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.names.as_slice()),
            (
                "NOT_FOUND",
                &["orders".to_string(), "second".to_string()][..]
            )
        );
        std::fs::remove_dir_all(tmp.path().join("suites/second")).unwrap();
        std::fs::write(
            tmp.path().join("suites/orders/suite.yaml"),
            SUITE.replace("GET /ping", "GET /nope"),
        )
        .unwrap();
        let e = load_plan(args(tmp.path())).unwrap_err();
        assert_eq!(e.code, "SUITE_INVALID");
        assert!(
            e.names[0].starts_with("suites/orders/suite.yaml:"),
            "{:?}",
            e.names
        );
        std::fs::write(
            tmp.path().join("suites/orders/suite.yaml"),
            "format: 1\nname: x\nbogus: 1\nscenarios: []\n",
        )
        .unwrap();
        assert_eq!(
            load_plan(args(tmp.path())).unwrap_err().code,
            "SUITE_INVALID"
        );
        std::fs::remove_dir_all(tmp.path().join("suites")).unwrap();
        assert_eq!(load_plan(args(tmp.path())).unwrap_err().code, "NOT_FOUND");
        std::fs::remove_file(tmp.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(load_plan(args(tmp.path())).unwrap_err().code, "NOT_FOUND");
    }
}
