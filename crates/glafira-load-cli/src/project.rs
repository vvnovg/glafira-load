//! Чтение файлов проекта для CLI: манифест, реестр, профили, сьюты. Ошибки — текст для stderr.

use std::path::{Path, PathBuf};

use glafira_load_model::from_yaml;
use glafira_load_model::profile::{PROFILE_FILE, ProfileFile};
use glafira_load_model::project::{MANIFEST_FILE, Manifest};
use glafira_load_model::registry::{REGISTRY_FILE, Registry};
use glafira_load_model::suite::{SUITE_FILE, SUITES_DIR};

/// Абсолютный путь проекта без `..`; на Windows — без префикса `\\?\`.
pub fn project_root(path: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(path).map_err(|e| format!("project {}: {e}", path.display()))
}

fn read_text(root: &Path, rel: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(rel))
        .map_err(|e| format!("cannot read {}: {e}", root.join(rel).display()))
}

fn read_file<T: serde::de::DeserializeOwned>(root: &Path, rel: &str) -> Result<T, String> {
    let text = read_text(root, rel)?;
    from_yaml(&text).map_err(|e| match e.line {
        Some(line) => format!("{rel}:{line}: {}", e.message),
        None => format!("{rel}: {}", e.message),
    })
}

pub fn read_manifest(root: &Path) -> Result<Manifest, String> {
    read_file(root, MANIFEST_FILE)
}

pub fn read_registry(root: &Path) -> Result<Registry, String> {
    read_file(root, REGISTRY_FILE)
}

pub fn read_profiles(root: &Path) -> Result<ProfileFile, String> {
    read_file(root, PROFILE_FILE)
}

/// `suites/<name>/suite.yaml` относительно проекта — так же пишутся `file` в находках `check`.
pub fn suite_file(name: &str) -> String {
    format!("{SUITES_DIR}/{name}/{SUITE_FILE}")
}

pub fn read_suite_text(root: &Path, name: &str) -> Result<String, String> {
    read_text(root, &suite_file(name))
}

/// Имена папок `suites/*` с `suite.yaml`, по алфавиту.
pub fn list_suites(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(SUITES_DIR);
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    for entry in
        std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?
    {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.join(SUITE_FILE).is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// Имя сьюта для `run`: `--suite`, иначе единственный сьют проекта; несколько — ошибка запуска.
pub fn resolve_suite(root: &Path, wanted: Option<&str>) -> Result<String, String> {
    if let Some(name) = wanted {
        return if root.join(suite_file(name)).is_file() {
            Ok(name.to_string())
        } else {
            Err(format!("suite `{name}` not found: {}", suite_file(name)))
        };
    }
    let names = list_suites(root)?;
    match names.as_slice() {
        [] => Err("the project has no suites".into()),
        [one] => Ok(one.clone()),
        many => Err(format!(
            "the project has {} suites, pass --suite: {}",
            many.len(),
            many.join(", ")
        )),
    }
}

/// Первый по алфавиту профиль — для `smoke`, которому цели не нужны.
pub fn first_profile(root: &Path) -> Result<String, String> {
    read_profiles(root)?
        .profiles
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| format!("{PROFILE_FILE} has no profiles"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_resolves_suites() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(
            resolve_suite(root, None).unwrap_err(),
            "the project has no suites"
        );
        for name in ["b", "a"] {
            std::fs::create_dir_all(root.join("suites").join(name)).unwrap();
            std::fs::write(
                root.join("suites").join(name).join("suite.yaml"),
                "format: 1\n",
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("suites/empty")).unwrap();
        assert_eq!(list_suites(root).unwrap(), vec!["a", "b"]);
        assert!(
            resolve_suite(root, None)
                .unwrap_err()
                .contains("pass --suite")
        );
        assert_eq!(resolve_suite(root, Some("b")).unwrap(), "b");
        assert!(
            resolve_suite(root, Some("zzz"))
                .unwrap_err()
                .contains("suites/zzz/suite.yaml")
        );
        std::fs::remove_dir_all(root.join("suites/b")).unwrap();
        assert_eq!(resolve_suite(root, None).unwrap(), "a");
        assert!(
            read_registry(root)
                .unwrap_err()
                .contains("registry/operations.yaml")
        );
        std::fs::create_dir_all(root.join("load")).unwrap();
        std::fs::write(
            root.join("load/profile.yaml"),
            "format: 1\nprofiles:\n  peak: { basis: peak }\n  nominal: { basis: avg }\n",
        )
        .unwrap();
        assert_eq!(first_profile(root).unwrap(), "nominal");
    }
}
