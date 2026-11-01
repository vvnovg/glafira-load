//! Команды сьютов: список, карточка с находками и строками, проверка с покрытием профиля,
//! создание заготовки из реестра (спека §7.2).

use std::collections::BTreeMap;

use glafira_load_model::suite::check::Finding;
use glafira_load_model::suite::coverage::{Coverage, coverage};
use glafira_load_model::suite::{Suite, parse};
use serde::Serialize;

use super::blocking;
use crate::error::AppResult;
use crate::load::profile::calculate;
use crate::project::manifest::ProjectDir;
use crate::suites::create::create;
use crate::suites::files::{list_names, load_checked, read_suite, step_lines, suite_file};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteMeta {
    pub name: String,
    /// `suites/<имя>/suite.yaml` относительно проекта (для «Открыть файл»).
    pub file: String,
    pub scenarios: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteView {
    pub name: String,
    pub file: String,
    pub text: String,
    /// `None` — не разобран; причина — в `findings`.
    pub suite: Option<Suite>,
    pub findings: Vec<Finding>,
    /// `"scenarios[0]"`, `"scenarios[0].steps[1]"` → строка файла (1-based).
    pub lines: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub findings: Vec<Finding>,
    /// Только если сьют разобран и в проекте есть профиль.
    pub coverage: Option<Coverage>,
    /// Профиль покрытия; пусто — профилей нет.
    pub profile: String,
}

/// Профиль для покрытия: `nominal`, если есть, иначе первый по алфавиту; пусто — профилей нет.
pub fn default_profile(d: &ProjectDir) -> AppResult<String> {
    let p = d.profile()?;
    Ok(if p.profiles.contains_key("nominal") {
        "nominal".into()
    } else {
        p.profiles.keys().next().cloned().unwrap_or_default()
    })
}

/// Сьюты проекта по алфавиту; у неразобранного сьюта `scenarios = 0`.
#[tauri::command]
pub async fn suite_list(path: String) -> AppResult<Vec<SuiteMeta>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let mut out = Vec::new();
        for name in list_names(&d)? {
            let scenarios = parse(&read_suite(&d, &name)?)
                .map(|s| s.scenarios.len())
                .unwrap_or(0);
            out.push(SuiteMeta {
                file: suite_file(&name),
                name,
                scenarios,
            });
        }
        Ok(out)
    })
    .await
}

/// Карточка сьюта: текст, дерево, находки `check` (без стенда) и строки сценариев и шагов.
#[tauri::command]
pub async fn suite_get(path: String, name: String) -> AppResult<SuiteView> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let c = load_checked(&d, &name, None)?;
        let lines = c
            .suite
            .as_ref()
            .map(|s| step_lines(&c.text, s))
            .unwrap_or_default();
        Ok(SuiteView {
            file: suite_file(&name),
            name,
            text: c.text,
            suite: c.suite,
            findings: c.findings,
            lines,
        })
    })
    .await
}

/// `check` и покрытие профиля (`nominal` либо первый по алфавиту).
#[tauri::command]
pub async fn suite_check(path: String, name: String) -> AppResult<CheckReport> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let c = load_checked(&d, &name, None)?;
        let profile = default_profile(&d)?;
        let coverage = match &c.suite {
            Some(suite) if !profile.is_empty() => Some(coverage(suite, &calculate(&d, &profile)?)),
            _ => None,
        };
        Ok(CheckReport {
            findings: c.findings,
            coverage,
            profile,
        })
    })
    .await
}

/// Заготовка сьюта из операций реестра; gRPC и SQL — `INVALID_INPUT`.
#[tauri::command]
pub async fn suite_create(
    path: String,
    name: String,
    operation_ids: Vec<String>,
) -> AppResult<SuiteMeta> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let (file, suite) = create(&d, &name, &operation_ids)?;
        Ok(SuiteMeta {
            name,
            file,
            scenarios: suite.scenarios.len(),
        })
    })
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use glafira_load_model::project::Agent;
    use glafira_load_model::registry::OpKind;
    use glafira_load_model::suite::check::Level;
    use std::path::{Path, PathBuf};

    pub(crate) fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let (src, dst) = (e.path(), to.join(e.file_name()));
            if src.is_dir() {
                copy_dir(&src, &dst);
            } else {
                std::fs::copy(&src, &dst).unwrap();
            }
        }
    }

    pub(crate) fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dev/fixtures/suites")
            .join(name)
    }

    /// Временный проект из фикстур Task 3: `project/*` как есть, `orders/suite.yaml` →
    /// `suites/orders/suite.yaml`, файлы данных `orders/data` → `data/`.
    pub(crate) fn copy_fixture_project() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        copy_dir(&fixture("project"), &root);
        let orders = fixture("orders");
        std::fs::create_dir_all(root.join("suites/orders")).unwrap();
        std::fs::copy(
            orders.join("suite.yaml"),
            root.join("suites/orders/suite.yaml"),
        )
        .unwrap();
        copy_dir(&orders.join("data"), &root.join("data"));
        (tmp, root.to_string_lossy().into_owned())
    }

    /// Стенд `test` фикстуры: HTTP-адрес заглушки, без Kafka, заданные агенты.
    pub(crate) fn point_env_to(path: &str, url: &str, agents: Vec<Agent>) {
        let d = ProjectDir::open(path).unwrap();
        let mut m = d.manifest().unwrap();
        let env = m
            .environments
            .iter_mut()
            .find(|e| e.id == "test")
            .expect("fixture has environment test");
        env.http = Some(url.into());
        env.kafka = None;
        env.schema_registry = None;
        env.agents = agents;
        d.save_manifest(&m).unwrap();
    }

    fn errors(findings: &[Finding]) -> Vec<String> {
        findings
            .iter()
            .filter(|f| matches!(f.level, Level::Error))
            .map(|f| format!("{}:{:?}: {}", f.file, f.line, f.message))
            .collect()
    }

    #[tokio::test]
    async fn list_get_and_check_on_the_fixture_project() {
        let (_tmp, path) = copy_fixture_project();
        let list = suite_list(path.clone()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].name.as_str(), list[0].file.as_str()),
            ("orders", "suites/orders/suite.yaml")
        );
        assert!(list[0].scenarios >= 1);
        let view = suite_get(path.clone(), "orders".into()).await.unwrap();
        assert!(view.suite.is_some(), "{:?}", errors(&view.findings));
        assert!(
            errors(&view.findings).is_empty(),
            "{:?}",
            errors(&view.findings)
        );
        assert!(view.text.contains("format: 1"));
        assert!(
            view.lines.contains_key("scenarios[0]")
                && view.lines.contains_key("scenarios[0].steps[0]")
        );
        let json = serde_json::to_value(&view).unwrap();
        assert!(json.get("lines").is_some() && json.get("findings").is_some());
        let report = suite_check(path.clone(), "orders".into()).await.unwrap();
        assert!(errors(&report.findings).is_empty());
        assert_eq!(report.profile, "nominal");
        let coverage = report.coverage.expect("suite parsed, profile exists");
        assert!(!coverage.rows.is_empty());
        assert_eq!(
            suite_get(path.clone(), "none".into())
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let d = ProjectDir::open(&path).unwrap();
        d.write_text(
            "suites/broken/suite.yaml",
            "format: 1\nname: broken\nscenarios: [\n",
        )
        .unwrap();
        let list = suite_list(path.clone()).await.unwrap();
        assert_eq!(
            (list[0].name.as_str(), list[0].scenarios),
            ("broken", 0),
            "неразобранный сьют в списке с нулём сценариев"
        );
        let view = suite_get(path.clone(), "broken".into()).await.unwrap();
        assert!(view.suite.is_none() && !errors(&view.findings).is_empty());
        assert!(view.lines.is_empty());
        let report = suite_check(path, "broken".into()).await.unwrap();
        assert!(
            report.coverage.is_none(),
            "покрытие только для разобранного сьюта"
        );
    }

    #[tokio::test]
    async fn created_suite_passes_check() {
        let (_tmp, path) = copy_fixture_project();
        let d = ProjectDir::open(&path).unwrap();
        let ids: Vec<String> = d
            .registry()
            .unwrap()
            .operations
            .iter()
            .filter(|o| {
                matches!(
                    o.draft.kind,
                    OpKind::Http | OpKind::KafkaProduce | OpKind::KafkaConsume
                )
            })
            .map(|o| o.draft.id.clone())
            .collect();
        assert!(ids.len() >= 2, "фикстура содержит операции HTTP и Kafka");
        let meta = suite_create(path.clone(), "generated".into(), ids.clone())
            .await
            .unwrap();
        assert_eq!(
            (meta.name.as_str(), meta.scenarios),
            ("generated", ids.len())
        );
        assert!(d.file(&meta.file).is_file());
        let report = suite_check(path.clone(), "generated".into()).await.unwrap();
        assert!(
            errors(&report.findings).is_empty(),
            "{:?}",
            errors(&report.findings)
        );
        let coverage = report.coverage.unwrap();
        assert!(!coverage.rows.is_empty());
        for row in &coverage.rows {
            assert!(row.planned > 0.0 || row.target == 0.0, "{row:?}");
        }
        assert!(coverage.uncovered.is_empty(), "{:?}", coverage.uncovered);
        assert_eq!(suite_list(path.clone()).await.unwrap().len(), 2);
        assert_eq!(
            suite_create(path.clone(), "generated".into(), ids.clone())
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            suite_create(path.clone(), "Bad".into(), ids)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            suite_create(path, "other".into(), vec!["nope".into()])
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn default_profile_prefers_nominal() {
        let (_tmp, path) = copy_fixture_project();
        let d = ProjectDir::open(&path).unwrap();
        assert_eq!(default_profile(&d).unwrap(), "nominal");
        let mut f = d.profile().unwrap();
        f.profiles.remove("nominal");
        d.save_profile(&f).unwrap();
        assert_eq!(default_profile(&d).unwrap(), "peak", "первый по алфавиту");
        f.profiles.clear();
        d.save_profile(&f).unwrap();
        assert_eq!(default_profile(&d).unwrap(), "");
        let report = suite_check(path, "orders".into()).await.unwrap();
        assert!(report.coverage.is_none() && report.profile.is_empty());
    }
}
