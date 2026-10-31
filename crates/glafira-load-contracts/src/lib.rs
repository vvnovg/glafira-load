//! Импортёры контрактов: байты файла → черновики операций (спека L1 §5.2). Крейт не читает диск:
//! соседние файлы (`$ref`, `import`) приходят через замыкание вызывающего, которое проверяет
//! границы клона и возвращает `None` за его пределами.

pub mod asyncapi;
pub mod openapi;
pub mod proto;
pub mod schema;
pub mod spring;
pub mod sql;
pub mod summary;

use std::collections::BTreeMap;

pub use glafira_load_model::project::{ContractKind, Direction};
use glafira_load_model::registry::OperationDraft;
use serde::{Deserialize, Serialize};

/// Чтение файла клона по относительному пути с разделителем `/`.
pub type ReadFile<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

pub struct ContractInput<'a> {
    /// Путь файла относительно клона, разделитель `/`.
    pub path: &'a str,
    pub bytes: &'a [u8],
    pub read: ReadFile<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicBinding {
    pub topic: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    pub service: String,
    /// Путь файла схемы → топик (из `kafka.topics` манифеста).
    pub topics: BTreeMap<String, TopicBinding>,
    /// Каталоги `import` для proto, относительно клона.
    pub include: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub path: String,
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParseResult {
    pub operations: Vec<OperationDraft>,
    pub tables: Vec<Table>,
    pub warnings: Vec<Diagnostic>,
}

impl ParseResult {
    pub fn warn(&mut self, path: &str, line: Option<u32>, message: impl Into<String>) {
        self.warnings.push(Diagnostic {
            path: path.into(),
            line,
            message: message.into(),
        });
    }
}

/// Файл не разобран целиком (синтаксис). Частичные проблемы — `ParseResult::warnings`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    pub message: String,
    pub line: Option<u32>,
}

pub fn parse(
    kind: ContractKind,
    input: &ContractInput,
    opts: &ParseOptions,
) -> Result<ParseResult, ParseError> {
    match kind {
        ContractKind::Openapi => openapi::parse(input, opts),
        ContractKind::Proto => proto::parse(input, opts),
        ContractKind::Asyncapi => asyncapi::parse(input, opts),
        ContractKind::Schema => schema::parse(input, opts),
        ContractKind::Sql => sql::parse(input, opts),
        ContractKind::Spring => spring::parse(input, opts),
    }
}

/// YAML или JSON → `serde_json::Value` (JSON — подмножество YAML, но свой разбор даёт строку ошибки точнее).
pub(crate) fn parse_document(bytes: &[u8]) -> Result<serde_json::Value, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError {
        message: format!("not UTF-8: {e}"),
        line: None,
    })?;
    if text.trim_start().starts_with('{') {
        return serde_json::from_str(text).map_err(|e| ParseError {
            message: e.to_string(),
            line: Some((e.line() as u32).min(text.lines().count().max(1) as u32)),
        });
    }
    serde_yaml_ng::from_str(text).map_err(|e| ParseError {
        message: e.to_string(),
        // Ошибка на конце файла указывает на пустую строку после последнего `\n` — берём последнюю строку текста.
        line: e
            .location()
            .map(|l| (l.line() as u32).min(text.lines().count().max(1) as u32)),
    })
}

/// Строка (с 1) первого вхождения `needle`.
pub(crate) fn line_of(text: &str, needle: &str) -> Option<u32> {
    text.find(needle)
        .map(|pos| text[..pos].matches('\n').count() as u32 + 1)
}

pub(crate) fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// `dir` + `rel` с нормализацией `.` и `..`; выход выше корня даёт пустые сегменты, которые
/// отбрасываются — вызывающий всё равно проверяет границы клона.
pub(crate) fn join_path(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_lines() {
        assert_eq!(
            join_path("api/v1", "../schemas/x.yaml"),
            "api/schemas/x.yaml"
        );
        assert_eq!(join_path("", "./a.yaml"), "a.yaml");
        assert_eq!(dir_of("a.yaml"), "");
        assert_eq!(dir_of("api/a.yaml"), "api");
        assert_eq!(line_of("a\nb\n  /v1/x:\n", "/v1/x:"), Some(3));
        assert_eq!(line_of("a", "zzz"), None);
        assert!(parse_document(b"{\"a\": 1}").is_ok());
        assert_eq!(parse_document(b"a: [\n").unwrap_err().line, Some(1));
    }
}
