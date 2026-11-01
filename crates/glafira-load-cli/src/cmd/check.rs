//! `glafira-load check`: проверка сьютов без сети (спека §3.5).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glafira_load_engine::context::data_info;
use glafira_load_model::suite;
use glafira_load_model::suite::check::{DataInfo, Level};

use super::startup_error;
use crate::console;
use crate::exit::{Exit, FINDINGS, OK, STARTUP};
use crate::project::{list_suites, project_root, read_registry, read_suite_text, suite_file};

pub async fn check(project: PathBuf, suite: Option<String>) -> Exit {
    tokio::task::spawn_blocking(move || check_sync(&project, suite.as_deref()))
        .await
        .unwrap_or(Exit(STARTUP))
}

/// Один сьют или все сьюты проекта; код 4, если есть находки уровня `error`.
pub fn check_sync(project: &Path, suite: Option<&str>) -> Exit {
    let root = match project_root(project) {
        Ok(r) => r,
        Err(e) => return startup_error(&e),
    };
    let registry = match read_registry(&root) {
        Ok(r) => r,
        Err(e) => return startup_error(&e),
    };
    let names = match suite {
        Some(name) => vec![name.to_string()],
        None => match list_suites(&root) {
            Ok(names) => names,
            Err(e) => return startup_error(&e),
        },
    };
    if names.is_empty() {
        return startup_error("the project has no suites");
    }
    let mut findings = Vec::new();
    for name in names {
        let file = suite_file(&name);
        let text = match read_suite_text(&root, &name) {
            Ok(t) => t,
            Err(e) => return startup_error(&e),
        };
        let data = data_infos(&root, &text);
        let (_, found) = suite::check::check(&text, &file, &registry, &data, None);
        findings.extend(found);
    }
    print!("{}", console::check_report(&findings));
    if findings.iter().any(|f| matches!(f.level, Level::Error)) {
        Exit(FINDINGS)
    } else {
        Exit(OK)
    }
}

/// Колонки и строки наборов данных сьюта; если YAML не разбирается, карта пуста — `check` сообщит об ошибке разбора.
fn data_infos(root: &Path, text: &str) -> BTreeMap<String, Option<DataInfo>> {
    match suite::parse(text) {
        Ok(s) => s
            .data
            .iter()
            .map(|(name, set)| (name.clone(), data_info(root, &set.path)))
            .collect(),
        Err(_) => BTreeMap::new(),
    }
}
