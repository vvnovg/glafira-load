//! Пользовательские настройки (язык интерфейса, открытые проекты) и выбор действующего языка.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{AppResult, ErrorCode};
use crate::storage::fs::{read_json_or_quarantine, write_json};
use crate::sync::RwLockExt;

/// Языки, для которых есть словарь `src/locales/<код>.json`. Остальные 10 языков семейства
/// добавляются на этапе I7 (спец. §17).
pub const LANGUAGES: [&str; 2] = ["en", "ru"];

fn is_known(code: &str) -> bool {
    LANGUAGES.contains(&code)
}

pub fn match_language(tag: &str) -> Option<&'static str> {
    match_language_in(tag, &LANGUAGES)
}

/// Язык ОС → код из `langs`. Китайский: явный Hans → zh-CN; Hant/TW/HK/MO → zh-TW; иначе zh-CN.
fn match_language_in(tag: &str, langs: &[&'static str]) -> Option<&'static str> {
    let lower = tag.replace('_', "-").to_ascii_lowercase();
    let mut parts = lower.split('-');
    let primary = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    let pick = |code: &'static str| langs.iter().copied().find(|c| *c == code);
    match primary {
        "pt" => pick("pt-BR"),
        "zh" => {
            let traditional = !rest.contains(&"hans")
                && rest
                    .iter()
                    .any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"));
            pick(if traditional { "zh-TW" } else { "zh-CN" })
        }
        _ => langs
            .iter()
            .copied()
            .find(|c| c.eq_ignore_ascii_case(primary)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageReason {
    Policy,
    User,
    System,
    Default,
}

/// Политика → выбор → ОС → en. Все языки доступны без лицензии.
pub fn resolve(
    system: Option<&str>,
    chosen: Option<&str>,
    policy: Option<&str>,
) -> (String, LanguageReason) {
    let candidate = policy
        .map(|p| (p.to_string(), LanguageReason::Policy))
        .or_else(|| chosen.map(|c| (c.to_string(), LanguageReason::User)))
        .or_else(|| {
            system
                .and_then(match_language)
                .map(|s| (s.to_string(), LanguageReason::System))
        });
    match candidate {
        Some((code, reason)) if is_known(&code) => (code, reason),
        _ => ("en".into(), LanguageReason::Default),
    }
}

/// Сколько последних проектов помнит приложение.
pub const MAX_RECENT_PROJECTS: usize = 20;

/// Открытые проекты НТ (спека L1 §3.4): пути к папкам и последний активный.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsSettings {
    #[serde(default)]
    pub recent: Vec<String>,
    #[serde(default)]
    pub active: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    schema_version: u32,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    projects: ProjectsSettings,
}

pub struct SettingsStore {
    path: PathBuf,
    data: RwLock<SettingsFile>,
}

impl SettingsStore {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("settings.json");
        let mut data = match read_json_or_quarantine::<SettingsFile>(&path, 1) {
            Ok(f) => f.unwrap_or_default(),
            Err(e) => {
                tracing::warn!("settings not read: {}", e.message);
                SettingsFile::default()
            }
        };
        data.language = data.language.filter(|l| is_known(l));
        Self {
            path,
            data: RwLock::new(data),
        }
    }

    pub fn language(&self) -> Option<String> {
        self.data.read_safe().language.clone()
    }

    pub fn projects(&self) -> ProjectsSettings {
        self.data.read_safe().projects.clone()
    }

    /// Блокирующий вызов: меняет копию, пишет файл и только потом обновляет кэш.
    fn update(&self, f: impl FnOnce(&mut SettingsFile)) -> AppResult<()> {
        let mut data = self.data.write_safe();
        let mut next = data.clone();
        f(&mut next);
        next.schema_version = 1;
        write_json(&self.path, &next)?;
        *data = next;
        Ok(())
    }

    /// Блокирующий вызов (запись файла).
    pub fn set_language(&self, lang: Option<&str>) -> AppResult<()> {
        if let Some(l) = lang
            && !is_known(l)
        {
            return Err(crate::app_err!(
                ErrorCode::InvalidInput,
                "errors.settings.unknownLanguage",
                "Unknown language “{{code}}”",
                code = l
            ));
        }
        self.update(|d| d.language = lang.map(String::from))
    }

    /// Блокирующий вызов. Проект поднимается в начало списка; список обрезается до `MAX_RECENT_PROJECTS`.
    pub fn remember_project(&self, path: &str) -> AppResult<()> {
        self.update(|d| {
            d.projects.recent.retain(|p| p != path);
            d.projects.recent.insert(0, path.to_string());
            d.projects.recent.truncate(MAX_RECENT_PROJECTS);
            d.projects.active = Some(path.to_string());
        })
    }

    /// Блокирующий вызов. Убирает проект из списка; активный проект сбрасывается, если это он.
    pub fn forget_project(&self, path: &str) -> AppResult<()> {
        self.update(|d| {
            d.projects.recent.retain(|p| p != path);
            if d.projects.active.as_deref() == Some(path) {
                d.projects.active = None;
            }
        })
    }

    /// Блокирующий вызов.
    pub fn set_active_project(&self, path: Option<&str>) -> AppResult<()> {
        self.update(|d| d.projects.active = path.map(String::from))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageSettings {
    pub system: Option<String>,
    pub system_match: Option<String>,
    pub chosen: Option<String>,
    pub policy: Option<String>,
    pub effective: String,
    pub reason: LanguageReason,
    pub languages: Vec<String>,
}

pub fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

pub fn language_settings(
    system: Option<String>,
    chosen: Option<String>,
    policy: Option<String>,
) -> LanguageSettings {
    let (effective, reason) = resolve(system.as_deref(), chosen.as_deref(), policy.as_deref());
    LanguageSettings {
        system_match: system.as_deref().and_then(match_language).map(String::from),
        system,
        chosen,
        policy,
        effective,
        reason,
        languages: LANGUAGES.iter().map(|c| c.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_system_tags() {
        assert_eq!(match_language("ru_RU"), Some("ru"));
        assert_eq!(match_language("en-GB"), Some("en"));
        assert_eq!(match_language("de-AT"), None, "словаря de пока нет");
        assert_eq!(match_language("zh-Hans-CN"), None);
        assert_eq!(match_language("uk-UA"), None);
    }

    const ALL: [&str; 12] = [
        "en", "ru", "de", "fr", "es", "it", "pl", "pt-BR", "zh-CN", "zh-TW", "ja", "ko",
    ];

    #[test]
    fn matching_asian_tags() {
        for tag in [
            "zh-Hans-CN",
            "zh-CN",
            "zh-SG",
            "zh",
            "zh-Hans-HK",
            "zh-Hans",
        ] {
            assert_eq!(match_language_in(tag, &ALL), Some("zh-CN"), "{tag}");
        }
        for tag in ["zh-Hant-TW", "zh-TW", "zh-HK", "zh-MO", "zh_TW", "zh-Hant"] {
            assert_eq!(match_language_in(tag, &ALL), Some("zh-TW"), "{tag}");
        }
        assert_eq!(match_language_in("ja-JP", &ALL), Some("ja"));
        assert_eq!(match_language_in("ko-KR", &ALL), Some("ko"));
        assert_eq!(match_language_in("pt-PT", &ALL), Some("pt-BR"));
        assert_eq!(match_language_in("de-AT", &ALL), Some("de"));
        assert_eq!(match_language_in("xx", &ALL), None);
    }

    #[test]
    fn asian_tags_without_asian_languages() {
        const BEFORE_E: [&str; 8] = ["en", "ru", "de", "fr", "es", "it", "pl", "pt-BR"];
        assert_eq!(match_language_in("zh-CN", &BEFORE_E), None);
        assert_eq!(match_language_in("zh-Hant-TW", &BEFORE_E), None);
        assert_eq!(match_language_in("ja-JP", &BEFORE_E), None);
    }

    #[test]
    fn priority_of_sources() {
        use LanguageReason::*;
        assert_eq!(resolve(Some("ru-RU"), None, None), ("ru".into(), System));
        assert_eq!(resolve(Some("ru"), Some("en"), None), ("en".into(), User));
        assert_eq!(resolve(None, Some("ru"), None), ("ru".into(), User));
        assert_eq!(
            resolve(Some("en"), Some("en"), Some("ru")),
            ("ru".into(), Policy)
        );
        assert_eq!(resolve(Some("de-DE"), None, None), ("en".into(), Default));
        assert_eq!(resolve(None, Some("fr"), None), ("en".into(), Default));
        assert_eq!(resolve(None, None, None), ("en".into(), Default));
    }

    #[test]
    fn recent_projects_are_ordered_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let s = SettingsStore::open(dir.path());
        for i in 0..25 {
            s.remember_project(&format!("/p/{i}")).unwrap();
        }
        let p = s.projects();
        assert_eq!(p.recent.len(), MAX_RECENT_PROJECTS);
        assert_eq!(p.recent[0], "/p/24");
        assert_eq!(p.active.as_deref(), Some("/p/24"));
        s.remember_project("/p/10").unwrap();
        assert_eq!(s.projects().recent[0], "/p/10");
        s.forget_project("/p/10").unwrap();
        let p = SettingsStore::open(dir.path()).projects();
        assert!(!p.recent.contains(&"/p/10".to_string()));
        assert_eq!(p.active, None, "активный сброшен вместе с удалением");
    }

    #[test]
    fn language_and_projects_do_not_reset_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let s = SettingsStore::open(dir.path());
        s.set_language(Some("ru")).unwrap();
        s.remember_project("/p/a").unwrap();
        let again = SettingsStore::open(dir.path());
        assert_eq!(again.language().as_deref(), Some("ru"));
        assert_eq!(again.projects().recent, vec!["/p/a".to_string()]);
    }

    #[test]
    fn store_persists_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let s = SettingsStore::open(dir.path());
        assert_eq!(s.language(), None);
        s.set_language(Some("ru")).unwrap();
        assert_eq!(
            SettingsStore::open(dir.path()).language().as_deref(),
            Some("ru")
        );
        assert_eq!(
            s.set_language(Some("xx")).unwrap_err().code,
            crate::error::ErrorCode::InvalidInput
        );
        s.set_language(None).unwrap();
        assert_eq!(SettingsStore::open(dir.path()).language(), None);
    }

    #[test]
    fn corrupt_settings_do_not_block() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{broken").unwrap();
        assert_eq!(SettingsStore::open(dir.path()).language(), None);
    }
}
