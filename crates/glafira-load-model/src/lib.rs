//! Модель файлов проекта НТ: типы и чистые функции без I/O (спека L1 §3, §6–§8).
//! Диск читает и пишет приложение; раннер (L2) использует те же типы.

pub mod diff;
pub mod merge;
pub mod project;
pub mod registry;
pub mod schema;

/// Ошибка чтения файла проекта: текст для `details` и строка, если YAML её знает.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ModelError {
    pub message: String,
    pub line: Option<u32>,
}

impl From<serde_yaml_ng::Error> for ModelError {
    fn from(e: serde_yaml_ng::Error) -> Self {
        Self {
            message: e.to_string(),
            line: e.location().map(|l| l.line() as u32),
        }
    }
}

pub fn from_yaml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ModelError> {
    Ok(serde_yaml_ng::from_str(text)?)
}

/// Файлы проекта пишутся только из согласованных структур: ошибка сериализации — баг, не ввод.
pub fn to_yaml<T: serde::Serialize>(value: &T) -> String {
    serde_yaml_ng::to_string(value).expect("project files serialize")
}
