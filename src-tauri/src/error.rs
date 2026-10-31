use std::collections::BTreeMap;

use serde::Serialize;

use crate::i18n::LocalizedText;

/// Коды ошибок (спека L1 §11). Фронтенд выбирает поведение по коду, текст — по `key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Timeout,
    Cancelled,
    Internal,
    /// Манифест или файл проекта не читается или не проходит проверки.
    ProjectInvalid,
    /// Файл контракта не разобран (остальные контракты разбираются).
    ContractParse,
    /// Нет `git`, папка не репозиторий, таймаут.
    GitUnavailable,
    /// Шаблон другого проекта или формата.
    TemplateMismatch,
    /// В шаблоне есть строки с ошибками.
    TemplateInvalid,
    /// Путь выходит за проект или клон.
    PathOutsideProject,
    /// Разбор сервиса уже идёт.
    ScanBusy,
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    /// Ключ словаря интерфейса (`src/locales/*.json`).
    pub key: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// Английский текст (для логов и как запасной вариант перевода).
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    pub retryable: bool,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn localized(code: ErrorCode, text: crate::i18n::LocalizedText) -> Self {
        Self {
            code,
            key: text.key,
            params: text.params,
            message: text.text,
            details: None,
            retryable: Self::retryable_code(code),
        }
    }

    fn retryable_code(code: ErrorCode) -> bool {
        matches!(code, ErrorCode::Timeout)
    }

    /// Текст ошибки для полей-сообщений (ключ, параметры, английский текст).
    pub fn text(&self) -> LocalizedText {
        LocalizedText {
            key: self.key.clone(),
            params: self.params.clone(),
            text: self.message.clone(),
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Неожиданная внутренняя ошибка: общий текст «Internal error», подробности — в `details`
    /// (технический текст, не переводится).
    pub fn internal_with(details: impl Into<String>) -> Self {
        app_err!(ErrorCode::Internal, "errors.internal", "Internal error").with_details(details)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        app_err!(
            ErrorCode::Internal,
            "errors.io",
            "I/O error: {{error}}",
            error = e
        )
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        app_err!(
            ErrorCode::Internal,
            "errors.json",
            "JSON error: {{error}}",
            error = e
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_for_frontend() {
        let json = serde_json::to_value(app_err!(
            ErrorCode::InvalidInput,
            "errors.project.emptyName",
            "Project name cannot be empty"
        ))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "code": "INVALID_INPUT",
                "key": "errors.project.emptyName",
                "message": "Project name cannot be empty",
                "retryable": false
            })
        );
        let json = serde_json::to_value(AppError::internal_with("boom")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "code": "INTERNAL",
                "key": "errors.internal",
                "message": "Internal error",
                "details": "boom",
                "retryable": false
            })
        );
    }

    #[test]
    fn codes_serialize_and_only_timeout_is_retryable() {
        for (code, name, retryable) in [
            (ErrorCode::InvalidInput, "INVALID_INPUT", false),
            (ErrorCode::NotFound, "NOT_FOUND", false),
            (ErrorCode::Timeout, "TIMEOUT", true),
            (ErrorCode::Cancelled, "CANCELLED", false),
            (ErrorCode::Internal, "INTERNAL", false),
            (ErrorCode::ProjectInvalid, "PROJECT_INVALID", false),
            (ErrorCode::ContractParse, "CONTRACT_PARSE", false),
            (ErrorCode::GitUnavailable, "GIT_UNAVAILABLE", false),
            (ErrorCode::TemplateMismatch, "TEMPLATE_MISMATCH", false),
            (ErrorCode::TemplateInvalid, "TEMPLATE_INVALID", false),
            (ErrorCode::PathOutsideProject, "PATH_OUTSIDE_PROJECT", false),
            (ErrorCode::ScanBusy, "SCAN_BUSY", false),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), name);
            assert_eq!(
                AppError::localized(code, LocalizedText::raw("x")).retryable,
                retryable,
                "{name}"
            );
        }
    }
}
