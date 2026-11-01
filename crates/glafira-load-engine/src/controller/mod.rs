//! Контроллер прогона: опрашивает агентов, раздаёт бандлы, сводит события и собирает `runs/<дата>/`
//! (спека §4.5, §6). Один код для приложения и `glafira-load run --agents` (отступление плана 8).

pub mod bundle;
pub mod merge;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use glafira_load_model::profile::Computed;
use glafira_load_model::project::Environment;
use glafira_load_model::registry::{REGISTRY_FILE, Registry};
use glafira_load_model::schema::SchemaSummary;
use glafira_load_model::suite::Suite;

use crate::agent_api::Mode;

pub use crate::agent_link::{AgentState, ControllerEvent, EngineError, RunResult};
pub use crate::local::project_commit;
pub use bundle::{slice_rows, split_targets};

/// Версия контроллера — версия крейта; сравнивается с версией агента по `major.minor`.
pub const CONTROLLER_VERSION: &str = crate::agent_link::VERSION;
/// Сколько держать секунду в буфере слияния, если тик кого-то из живых агентов не пришёл.
pub const MERGE_HOLD: Duration = Duration::from_secs(2);

/// Всё, что нужно для прогона: проект уже прочитан и проверен (`plan::load_plan`, Task 11c).
#[derive(Debug, Clone)]
pub struct RunPlan {
    pub root: PathBuf,
    pub suite_name: String,
    pub suite: Suite,
    pub suite_text: String,
    pub profile: String,
    pub computed: Computed,
    pub env: Environment,
    pub mode: Mode,
    pub max_in_flight: Option<u32>,
    pub out_dir: Option<PathBuf>,
    /// Схемы ответов операций реестра (`expect.schema: response`) — уходят агентам в `RunRequest`.
    pub response_schemas: BTreeMap<String, SchemaSummary>,
    /// Тексты `.avsc` операций `kafka_produce` с `encoding: avro` — уходят агентам в `RunRequest`.
    pub avro_schemas: BTreeMap<String, String>,
}

/// Вес агента по манифесту стенда; агент вне манифеста (например, `LocalAgent`) — 1.
pub fn weight_of(env: &Environment, id: &str) -> u32 {
    env.agents
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.weight.max(1))
        .unwrap_or(1)
}

/// Версии сервисов из `registry/operations.yaml` проекта (`local::service_versions` по прочитанному
/// реестру); нет файла или он не разбирается — пусто.
pub fn service_versions(root: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(root.join(REGISTRY_FILE)) else {
        return BTreeMap::new();
    };
    let Ok(registry) = glafira_load_model::from_yaml::<Registry>(&text) else {
        return BTreeMap::new();
    };
    crate::local::service_versions(&registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_of_reads_the_manifest_and_defaults_to_one() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = bundle::tests::plan(tmp.path().to_path_buf());
        assert_eq!(weight_of(&plan.env, "a"), 1);
        assert_eq!(weight_of(&plan.env, "b"), 2);
        assert_eq!(
            weight_of(&plan.env, "local"),
            1,
            "agent outside the manifest"
        );
        let mut env = plan.env.clone();
        env.agents[1].weight = 0;
        assert_eq!(weight_of(&env, "b"), 1, "weight 0 counts as 1");
    }

    #[test]
    fn service_versions_without_registry_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(service_versions(tmp.path()).is_empty());
        std::fs::create_dir_all(tmp.path().join("registry")).unwrap();
        std::fs::write(tmp.path().join(REGISTRY_FILE), ": not yaml [").unwrap();
        assert!(service_versions(tmp.path()).is_empty());
    }
}
