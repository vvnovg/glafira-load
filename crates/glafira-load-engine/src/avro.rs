//! Avro по схеме контракта в формате Confluent (спека L2 §5.2).
//!
//! Схема — текст `.avsc` операции реестра (карта «операция → схема» передаётся при создании
//! исполнителя), id схемы — `GET {schema_registry}/subjects/{topic}-value/versions/latest`,
//! кэш на прогон. Кадр: магический байт `0x00`, id как u32 big-endian, тело Avro.

use std::collections::BTreeMap;
use std::sync::Mutex;

use apache_avro::types::Value as AvroValue;
use apache_avro::{Schema, from_avro_datum, to_avro_datum};
use serde::Deserialize;

use crate::errors::{MAX_SAMPLE, StepCode, StepError, truncate};

/// Магический байт формата Confluent.
pub const MAGIC: u8 = 0;

/// Кадр Confluent: `0x00`, id схемы (u32 big-endian), тело.
pub fn frame(id: u32, datum: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + datum.len());
    out.push(MAGIC);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(datum);
    out
}

/// Разбор кадра Confluent: id схемы и тело.
pub fn unframe(bytes: &[u8]) -> Result<(u32, &[u8]), String> {
    if bytes.len() < 5 {
        return Err(format!(
            "not a Confluent Avro frame: {} byte(s)",
            bytes.len()
        ));
    }
    if bytes[0] != MAGIC {
        return Err(format!(
            "not a Confluent Avro frame: magic byte {:#04x}",
            bytes[0]
        ));
    }
    let id = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    Ok((id, &bytes[5..]))
}

/// JSON → значение Avro по схеме (объекты → record/map, числа → int/long/double, union по
/// варианту, отсутствующие поля — из `default`) → тело без заголовка.
pub fn encode(schema: &Schema, value: &serde_json::Value) -> Result<Vec<u8>, String> {
    let avro = AvroValue::from(value.clone())
        .resolve(schema)
        .map_err(|e| format!("value does not match the Avro schema: {e}"))?;
    to_avro_datum(schema, avro).map_err(|e| format!("avro encode: {e}"))
}

/// Тело → JSON по той же схеме (тесты и чтение сообщений).
pub fn decode(schema: &Schema, datum: &[u8]) -> Result<serde_json::Value, String> {
    let mut cursor = datum;
    let value =
        from_avro_datum(schema, &mut cursor, None).map_err(|e| format!("avro decode: {e}"))?;
    serde_json::Value::try_from(value).map_err(|e| format!("avro to json: {e}"))
}

#[derive(Deserialize)]
struct LatestVersion {
    id: u32,
}

/// Клиент Schema Registry: только id последней версии субъекта `<topic>-value`, кэш на прогон.
pub struct SchemaRegistry {
    base: String,
    http: reqwest::Client,
    ids: Mutex<BTreeMap<String, u32>>,
}

impl SchemaRegistry {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            ids: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn latest_id(&self, topic: &str) -> Result<u32, String> {
        let subject = format!("{topic}-value");
        if let Some(id) = self.ids.lock().expect("registry cache").get(&subject) {
            return Ok(*id);
        }
        let url = format!("{}/subjects/{subject}/versions/latest", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("schema registry {url}: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("schema registry {url}: HTTP {}", status.as_u16()));
        }
        let latest: LatestVersion = resp
            .json()
            .await
            .map_err(|e| format!("schema registry {url}: {e}"))?;
        self.ids
            .lock()
            .expect("registry cache")
            .insert(subject, latest.id);
        Ok(latest.id)
    }
}

/// Схемы операций `kafka_produce` с `encoding: avro` и реестр стенда.
pub struct AvroCodec {
    registry: Option<SchemaRegistry>,
    schemas: BTreeMap<String, Schema>,
}

fn avro_err(message: impl Into<String>, request: Option<String>) -> StepError {
    StepError {
        code: StepCode::AvroEncode,
        message: message.into(),
        request,
        response: None,
    }
}

impl AvroCodec {
    /// `schema_texts`: id операции → текст `.avsc`. Ошибка — нечитаемая схема.
    pub fn new(
        registry: Option<String>,
        schema_texts: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let mut schemas = BTreeMap::new();
        for (operation, text) in schema_texts {
            let schema = Schema::parse_str(text)
                .map_err(|e| format!("Avro schema of `{operation}`: {e}"))?;
            schemas.insert(operation.clone(), schema);
        }
        Ok(Self {
            registry: registry.as_deref().map(SchemaRegistry::new),
            schemas,
        })
    }

    pub fn has_schema(&self, operation: &str) -> bool {
        self.schemas.contains_key(operation)
    }

    /// Кадр Confluent для значения операции; `topic` — субъект `<topic>-value`.
    pub async fn encode_message(
        &self,
        operation: &str,
        topic: &str,
        value: &serde_json::Value,
    ) -> Result<Vec<u8>, StepError> {
        let preview = || Some(truncate(&value.to_string(), MAX_SAMPLE));
        let schema = self
            .schemas
            .get(operation)
            .ok_or_else(|| avro_err(format!("no Avro schema for `{operation}`"), preview()))?;
        let registry = self.registry.as_ref().ok_or_else(|| {
            avro_err(
                "schema registry is not configured for the environment",
                preview(),
            )
        })?;
        let id = registry
            .latest_id(topic)
            .await
            .map_err(|m| avro_err(m, preview()))?;
        let datum = encode(schema, value).map_err(|m| avro_err(m, preview()))?;
        Ok(frame(id, &datum))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use apache_avro::Schema;
    use axum::extract::Path;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;

    use super::*;

    pub(crate) const SCHEMA: &str = r#"{"type":"record","name":"OrderPaid","namespace":"orders","fields":[
        {"name":"orderId","type":"string"},
        {"name":"amount","type":"long"},
        {"name":"note","type":["null","string"],"default":null}]}"#;

    #[test]
    fn encodes_json_by_schema_in_confluent_frame() {
        let schema = Schema::parse_str(SCHEMA).unwrap();
        let datum = encode(&schema, &json!({"orderId": "o-1", "amount": 250})).unwrap();
        let framed = frame(7, &datum);
        assert_eq!(&framed[..5], &[0, 0, 0, 0, 7]);
        let (id, body) = unframe(&framed).unwrap();
        assert_eq!(id, 7);
        assert_eq!(body, &datum[..]);
        assert_eq!(
            decode(&schema, body).unwrap(),
            json!({"orderId": "o-1", "amount": 250, "note": null}),
            "значение по умолчанию заполнено, long остался числом"
        );
        assert!(
            encode(&schema, &json!({"orderId": 5, "amount": 1})).is_err(),
            "число вместо строки"
        );
        assert!(
            encode(&schema, &json!({"orderId": "x"})).is_err(),
            "нет обязательного поля без default"
        );
        assert!(
            unframe(&[1, 0, 0, 0, 7, 9]).is_err(),
            "не тот магический байт"
        );
        assert!(unframe(&[0, 0, 0]).is_err(), "короче заголовка");
    }

    /// Заглушка Schema Registry: отвечает `{"id": 7}` и считает обращения.
    pub(crate) async fn registry_stub() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = Router::new().route(
            "/subjects/{subject}/versions/latest",
            get(move |Path(subject): Path<String>| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"id": 7, "version": 1, "subject": subject, "schema": SCHEMA}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), hits)
    }

    #[tokio::test]
    async fn registry_id_is_fetched_once_per_subject() {
        let (base, hits) = registry_stub().await;
        let registry = SchemaRegistry::new(&format!("{base}/"));
        assert_eq!(registry.latest_id("orders.paid").await.unwrap(), 7);
        assert_eq!(registry.latest_id("orders.paid").await.unwrap(), 7);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "кэш на прогон");
        assert_eq!(registry.latest_id("orders.created").await.unwrap(), 7);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        let dead = SchemaRegistry::new("http://127.0.0.1:1");
        assert!(
            dead.latest_id("x")
                .await
                .unwrap_err()
                .contains("schema registry")
        );
    }

    #[tokio::test]
    async fn codec_frames_message_and_reports_missing_registry() {
        let (base, _) = registry_stub().await;
        let schemas = std::collections::BTreeMap::from([(
            "kafka_produce:orders.paid".to_string(),
            SCHEMA.to_string(),
        )]);
        let codec = AvroCodec::new(Some(base), &schemas).unwrap();
        let bytes = codec
            .encode_message(
                "kafka_produce:orders.paid",
                "orders.paid",
                &json!({"orderId": "o-1", "amount": 1}),
            )
            .await
            .unwrap();
        assert_eq!(&bytes[..5], &[0, 0, 0, 0, 7]);
        let err = codec
            .encode_message("kafka_produce:other", "other", &json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err.code, StepCode::AvroEncode) && err.message.contains("no Avro schema"),
            "{err:?}"
        );
        let err = codec
            .encode_message(
                "kafka_produce:orders.paid",
                "orders.paid",
                &json!({"orderId": 1}),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err.code, StepCode::AvroEncode) && err.request.is_some(),
            "{err:?}"
        );

        let no_registry = AvroCodec::new(None, &schemas).unwrap();
        let err = no_registry
            .encode_message(
                "kafka_produce:orders.paid",
                "orders.paid",
                &json!({"orderId": "o", "amount": 1}),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("schema registry is not configured"),
            "{err:?}"
        );
        assert!(
            AvroCodec::new(
                None,
                &std::collections::BTreeMap::from([("op".to_string(), "{bad".to_string())])
            )
            .is_err()
        );
    }
}
