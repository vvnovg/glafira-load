//! Файлы сьютов: список `suites/*/suite.yaml`, чтение текста, сведения о наборах данных,
//! строки сценариев и шагов, проверка (`check`) с данными проекта.

use std::collections::BTreeMap;

use glafira_load_engine::context::{data_info as engine_data_info, is_inside_project};
use glafira_load_model::project::{Environment, is_id};
use glafira_load_model::suite::check::{DataInfo, Finding, Level, check};
use glafira_load_model::suite::expr::PathSeg;
use glafira_load_model::suite::lines::line_of;
use glafira_load_model::suite::{DataSet, SUITE_FILE, SUITES_DIR, Suite, parse};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::project::paths::{inside, resolve};

/// `suites/<имя>/suite.yaml`.
pub fn suite_file(name: &str) -> String {
    format!("{SUITES_DIR}/{name}/{SUITE_FILE}")
}

pub fn not_found(name: &str) -> AppError {
    app_err!(
        ErrorCode::NotFound,
        "errors.suite.notFound",
        "Suite {{name}} not found",
        name = name
    )
}

/// Имя сьюта — `[a-z0-9-]{1,64}` (Global Constraints).
pub fn bad_name(name: &str) -> AppError {
    app_err!(
        ErrorCode::InvalidInput,
        "errors.suite.badName",
        "Suite name must be 1–64 lowercase latin letters, digits or dashes: {{name}}",
        name = name
    )
}

pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(|f| matches!(f.level, Level::Error))
}

/// Имена сьютов: папки `suites/<имя>` с допустимым именем и `suite.yaml`, по алфавиту.
pub fn list_names(d: &ProjectDir) -> AppResult<Vec<String>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(d.file(SUITES_DIR)) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_id(&name) && e.path().join(SUITE_FILE).is_file() {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

/// Текст сьюта; недопустимое имя — `INVALID_INPUT`, нет файла — `NOT_FOUND`.
pub fn read_suite(d: &ProjectDir, name: &str) -> AppResult<String> {
    if !is_id(name) {
        return Err(bad_name(name));
    }
    d.read_text(&suite_file(name))?
        .ok_or_else(|| not_found(name))
}

/// Сведения о наборах для `check`: `Some` — файл прочитан, `None` — файла нет (или он испорчен).
/// Путь вне проекта по тексту (`..`, абсолютный) не читается и в ответ не попадает: о нём
/// сообщает `check` находкой со строкой. Путь, который выходит из проекта по ссылке, —
/// `PATH_OUTSIDE_PROJECT`.
pub fn data_info(
    d: &ProjectDir,
    data: &BTreeMap<String, DataSet>,
) -> AppResult<BTreeMap<String, Option<DataInfo>>> {
    let mut out = BTreeMap::new();
    for (name, set) in data {
        if !is_inside_project(&set.path) {
            continue;
        }
        // Границы проекта по каноническому пути (ссылки); чтение и разбор — функция движка.
        inside(d.root(), &resolve(d.root(), &set.path))?;
        out.insert(name.clone(), engine_data_info(d.root(), &set.path));
    }
    Ok(out)
}

/// Строки `scenarios[i]` и `scenarios[i].steps[j]` (1-based) для карточек вкладки;
/// ненайденные пути опускаются (решение плана 1).
pub fn step_lines(text: &str, suite: &Suite) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for (i, sc) in suite.scenarios.iter().enumerate() {
        let scenario = [PathSeg::Key("scenarios".into()), PathSeg::Index(i)];
        if let Some(l) = line_of(text, &scenario) {
            out.insert(format!("scenarios[{i}]"), l);
        }
        for j in 0..sc.steps.len() {
            let step = [
                PathSeg::Key("scenarios".into()),
                PathSeg::Index(i),
                PathSeg::Key("steps".into()),
                PathSeg::Index(j),
            ];
            if let Some(l) = line_of(text, &step) {
                out.insert(format!("scenarios[{i}].steps[{j}]"), l);
            }
        }
    }
    out
}

/// Сьют после `check`: текст, разобранное дерево (если разобрано) и находки.
#[derive(Debug)]
pub struct Checked {
    pub text: String,
    pub suite: Option<Suite>,
    pub findings: Vec<Finding>,
}

/// Читает сьют, собирает сведения о наборах и выполняет `check` против реестра проекта.
/// `env` — стенд для проверок, зависящих от него (`avro` без `schema_registry`).
pub fn load_checked(d: &ProjectDir, name: &str, env: Option<&Environment>) -> AppResult<Checked> {
    let text = read_suite(d, name)?;
    let file = suite_file(name);
    // Имена наборов известны только после разбора; не разобрался — `check` сообщит сам.
    let data = match parse(&text) {
        Ok(s) => data_info(d, &s.data)?,
        Err(_) => BTreeMap::new(),
    };
    let registry = d.registry()?;
    let (suite, findings) = check(&text, &file, &registry, &data, env);
    Ok(Checked {
        text,
        suite,
        findings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest::create;
    use glafira_load_model::suite::DataMode;

    fn project() -> (tempfile::TempDir, ProjectDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").to_string_lossy().into_owned();
        create(&path, "t").unwrap();
        let d = ProjectDir::open(&path).unwrap();
        (dir, d)
    }

    #[test]
    fn lists_only_valid_suite_folders() {
        let (_tmp, d) = project();
        assert!(list_names(&d).unwrap().is_empty(), "папки suites ещё нет");
        d.write_text("suites/b-suite/suite.yaml", "format: 1\n")
            .unwrap();
        d.write_text("suites/a/suite.yaml", "format: 1\n").unwrap();
        d.write_text("suites/Bad Name/suite.yaml", "format: 1\n")
            .unwrap();
        d.write_text("suites/empty/notes.txt", "x").unwrap();
        assert_eq!(list_names(&d).unwrap(), ["a", "b-suite"]);
        assert_eq!(suite_file("a"), "suites/a/suite.yaml");
        assert_eq!(read_suite(&d, "a").unwrap(), "format: 1\n");
        assert_eq!(
            read_suite(&d, "none").unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            read_suite(&d, "Bad Name").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    fn set(path: &str) -> DataSet {
        DataSet {
            path: path.into(),
            mode: DataMode::Shared,
        }
    }

    #[test]
    fn data_info_reads_csv_and_jsonl_and_reports_missing_files() {
        let (_tmp, d) = project();
        d.write_text("data/users.csv", "id,name\n1,\"a, b\"\n2,c\n")
            .unwrap();
        d.write_text(
            "data/skus.jsonl",
            "{\"sku\": \"A\", \"qty\": 1}\n\n{\"sku\": \"B\", \"qty\": 2}\n",
        )
        .unwrap();
        let mut data = BTreeMap::new();
        data.insert("users".to_string(), set("data/users.csv"));
        data.insert("skus".to_string(), set("data/skus.jsonl"));
        data.insert("none".to_string(), set("data/none.csv"));
        let info = data_info(&d, &data).unwrap();
        let users = info["users"].as_ref().unwrap();
        assert_eq!(
            (users.columns.clone(), users.rows),
            (vec!["id".to_string(), "name".into()], 2)
        );
        let skus = info["skus"].as_ref().unwrap();
        assert_eq!(
            (skus.columns.clone(), skus.rows),
            (vec!["qty".to_string(), "sku".into()], 2)
        );
        assert!(info["none"].is_none());
        // Путь вне проекта по тексту не читается: о нём сообщает `check` находкой со строкой.
        data.insert("out".to_string(), set("../outside.csv"));
        let info = data_info(&d, &data).unwrap();
        assert!(!info.contains_key("out"));
    }

    #[cfg(unix)]
    #[test]
    fn data_info_rejects_a_link_out_of_the_project() {
        let (tmp, d) = project();
        std::fs::write(tmp.path().join("outside.csv"), "id\n1\n").unwrap();
        std::fs::create_dir_all(d.file("data")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside.csv"), d.file("data/link.csv"))
            .unwrap();
        let mut data = BTreeMap::new();
        data.insert("link".to_string(), set("data/link.csv"));
        assert_eq!(
            data_info(&d, &data).unwrap_err().code,
            ErrorCode::PathOutsideProject
        );
    }

    #[test]
    fn step_lines_follow_the_yaml_text() {
        let text = "format: 1\nname: s\nscenarios:\n  - id: a\n    steps:\n      - http:\n          operation: \"http:orders:GET /v1/x\"\n      - http:\n          operation: \"http:orders:GET /v1/y\"\n";
        let suite = parse(text).unwrap();
        let lines = step_lines(text, &suite);
        assert_eq!(lines["scenarios[0]"], 4);
        assert_eq!(lines["scenarios[0].steps[0]"], 6);
        assert_eq!(lines["scenarios[0].steps[1]"], 8);
    }

    #[test]
    fn load_checked_reports_parse_errors_and_missing_operations() {
        let (_tmp, d) = project();
        d.write_text(
            "suites/bad/suite.yaml",
            "format: 1\nname: bad\nscenarios: [\n",
        )
        .unwrap();
        let c = load_checked(&d, "bad", None).unwrap();
        assert!(c.suite.is_none() && has_errors(&c.findings));
        d.write_text(
            "suites/noop/suite.yaml",
            "format: 1\nname: noop\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:orders:GET /nope\" }\n",
        )
        .unwrap();
        let c = load_checked(&d, "noop", None).unwrap();
        assert!(
            has_errors(&c.findings),
            "{:?}",
            c.findings.iter().map(|f| &f.message).collect::<Vec<_>>()
        );
        assert_eq!(c.findings[0].file, "suites/noop/suite.yaml");
        assert_eq!(
            load_checked(&d, "none", None).unwrap_err().code,
            ErrorCode::NotFound
        );
        // Набор вне проекта — находка `check` со строкой, а не ошибка команды.
        d.write_text(
            "suites/out/suite.yaml",
            "format: 1\nname: out\ndata:\n  users: ../users.csv\nscenarios: []\n",
        )
        .unwrap();
        let c = load_checked(&d, "out", None).unwrap();
        let f = c
            .findings
            .iter()
            .find(|f| f.message.contains("inside the project"))
            .expect("finding about the path");
        assert_eq!(f.line, Some(4));
    }
}
