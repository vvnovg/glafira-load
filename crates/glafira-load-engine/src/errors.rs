//! Коды и ошибки шагов, маскирование секретов и обрезка примеров (спека L2 §5.3).

use std::fmt;

/// Предел длины запроса и ответа в примере ошибки, байт.
pub const MAX_SAMPLE: usize = 4096;
/// Чем заменяется значение секрета.
pub const MASK: &str = "***";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StepCode {
    Http4xx,
    Http5xx,
    Timeout,
    Connect,
    ExpectStatus,
    ExpectJsonpath,
    ExpectSchema,
    ExpectMaxMs,
    ExtractMissing,
    KafkaProduce,
    KafkaConsume,
    AvroEncode,
}

impl StepCode {
    pub fn as_str(self) -> &'static str {
        match self {
            StepCode::Http4xx => "HTTP_4xx",
            StepCode::Http5xx => "HTTP_5xx",
            StepCode::Timeout => "TIMEOUT",
            StepCode::Connect => "CONNECT",
            StepCode::ExpectStatus => "EXPECT_STATUS",
            StepCode::ExpectJsonpath => "EXPECT_JSONPATH",
            StepCode::ExpectSchema => "EXPECT_SCHEMA",
            StepCode::ExpectMaxMs => "EXPECT_MAX_MS",
            StepCode::ExtractMissing => "EXTRACT_MISSING",
            StepCode::KafkaProduce => "KAFKA_PRODUCE",
            StepCode::KafkaConsume => "KAFKA_CONSUME",
            StepCode::AvroEncode => "AVRO_ENCODE",
        }
    }

    pub const ALL: [StepCode; 12] = [
        StepCode::Http4xx,
        StepCode::Http5xx,
        StepCode::Timeout,
        StepCode::Connect,
        StepCode::ExpectStatus,
        StepCode::ExpectJsonpath,
        StepCode::ExpectSchema,
        StepCode::ExpectMaxMs,
        StepCode::ExtractMissing,
        StepCode::KafkaProduce,
        StepCode::KafkaConsume,
        StepCode::AvroEncode,
    ];
}

impl fmt::Display for StepCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ошибка шага: код, причина и пример запроса/ответа (уже замаскированные и обрезанные).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct StepError {
    pub code: StepCode,
    pub message: String,
    pub request: Option<String>,
    pub response: Option<String>,
}

impl StepError {
    pub fn new(code: StepCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            request: None,
            response: None,
        }
    }

    /// Добавляет пример запроса и ответа, если их ещё нет.
    pub fn with_sample(mut self, request: Option<String>, response: Option<String>) -> Self {
        if self.request.is_none() {
            self.request = request;
        }
        if self.response.is_none() {
            self.response = response;
        }
        self
    }
}

/// Заменяет каждое непустое значение секрета на `***`.
pub fn mask(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        out = out.replace(s.as_str(), MASK);
    }
    out
}

/// Обрезает текст до `max` байт по границе символа UTF-8; обрезанный хвост помечается `...`.
pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    const TAIL: &str = "...";
    let keep = max.saturating_sub(TAIL.len());
    let mut cut = keep;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{TAIL}", &text[..cut])
}

/// Маскирование и обрезка одним вызовом: так готовятся `request` и `response` примеров.
pub fn sample(text: &str, secrets: &[String]) -> String {
    truncate(&mask(text, secrets), MAX_SAMPLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_spec_names() {
        let names: Vec<&str> = StepCode::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "HTTP_4xx",
                "HTTP_5xx",
                "TIMEOUT",
                "CONNECT",
                "EXPECT_STATUS",
                "EXPECT_JSONPATH",
                "EXPECT_SCHEMA",
                "EXPECT_MAX_MS",
                "EXTRACT_MISSING",
                "KAFKA_PRODUCE",
                "KAFKA_CONSUME",
                "AVRO_ENCODE"
            ]
        );
        assert_eq!(
            StepError::new(StepCode::Timeout, "no reply in 5000 ms").to_string(),
            "TIMEOUT: no reply in 5000 ms"
        );
    }

    #[test]
    fn mask_replaces_every_secret_everywhere() {
        let secrets = vec!["s3cr3t".to_string(), "p@ss".to_string(), String::new()];
        let text = "Authorization: Bearer s3cr3t\n{\"password\":\"p@ss\",\"again\":\"s3cr3t\"}";
        let masked = mask(text, &secrets);
        assert_eq!(
            masked,
            "Authorization: Bearer ***\n{\"password\":\"***\",\"again\":\"***\"}"
        );
        assert!(!masked.contains("s3cr3t") && !masked.contains("p@ss"));
        assert_eq!(mask("plain", &[]), "plain");
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 10), "abcdefghij");
        assert_eq!(truncate("abcdefghijk", 10), "abcdefg...");
        // Кириллица: два байта на символ, граница не рвёт символ.
        let text = "привет мир";
        let cut = truncate(text, 8);
        assert!(cut.len() <= 8, "{cut:?}");
        assert_eq!(cut, "пр...");
        let text = "日本語テキスト";
        let cut = truncate(text, 10);
        assert!(cut.len() <= 10 && cut.ends_with("..."), "{cut:?}");
        assert_eq!(cut, "日本...");
    }

    #[test]
    fn sample_masks_then_truncates_to_the_limit() {
        let secret = "x".repeat(10);
        let body = format!("{secret}{}", "y".repeat(MAX_SAMPLE * 2));
        let s = sample(&body, std::slice::from_ref(&secret));
        assert!(s.starts_with(MASK));
        assert!(s.len() <= MAX_SAMPLE && s.ends_with("..."));
        let e = StepError::new(StepCode::Http5xx, "status 500")
            .with_sample(Some("POST /x".into()), None)
            .with_sample(Some("ignored".into()), Some("500".into()));
        assert_eq!(e.request.as_deref(), Some("POST /x"));
        assert_eq!(e.response.as_deref(), Some("500"));
    }
}
