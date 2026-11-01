//! Извлечение значений `extract` из тела ответа или сообщения (спека L2 §3.4).

use std::collections::BTreeMap;

use serde_json::Value;
use serde_json_path::JsonPath;

use crate::errors::{StepCode, StepError};

fn missing(name: &str, path: &str, why: &str) -> StepError {
    StepError::new(
        StepCode::ExtractMissing,
        format!("`{name}`: {why} at `{path}`"),
    )
}

/// Все `extract` шага: имя → первое значение по JSONPath. Нет тела, плохой путь или
/// пустой результат — `EXTRACT_MISSING` с именем и путём.
pub fn extract_all(
    extract: &BTreeMap<String, String>,
    body: Option<&Value>,
) -> Result<BTreeMap<String, Value>, StepError> {
    let mut out = BTreeMap::new();
    for (name, path) in extract {
        let body = body.ok_or_else(|| missing(name, path, "no JSON body"))?;
        let query = JsonPath::parse(path)
            .map_err(|e| missing(name, path, &format!("bad jsonpath ({e})")))?;
        let value = query
            .query(body)
            .first()
            .cloned()
            .ok_or_else(|| missing(name, path, "nothing"))?;
        out.insert(name.clone(), value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn extracts_values_keeping_their_types() {
        let body = json!({"id": "A-1", "qty": 3, "items": [{"sku": "S1"}, {"sku": "S2"}], "nested": {"ok": true}});
        let got = extract_all(
            &map(&[
                ("order_id", "$.id"),
                ("qty", "$.qty"),
                ("sku", "$.items[1].sku"),
                ("flag", "$.nested.ok"),
                ("all", "$.items[*].sku"),
            ]),
            Some(&body),
        )
        .unwrap();
        assert_eq!(got["order_id"], json!("A-1"));
        assert_eq!(got["qty"], json!(3));
        assert_eq!(got["sku"], json!("S2"));
        assert_eq!(got["flag"], json!(true));
        assert_eq!(got["all"], json!("S1"), "несколько узлов — берётся первый");
        assert!(extract_all(&BTreeMap::new(), None).unwrap().is_empty());
    }

    #[test]
    fn missing_path_body_or_bad_path_is_extract_missing() {
        let body = json!({"id": "A-1"});
        let e = extract_all(&map(&[("order_id", "$.nope")]), Some(&body)).unwrap_err();
        assert_eq!(e.code, StepCode::ExtractMissing);
        assert_eq!(e.message, "`order_id`: nothing at `$.nope`");
        let e = extract_all(&map(&[("order_id", "$.id")]), None).unwrap_err();
        assert_eq!(e.code, StepCode::ExtractMissing);
        assert!(
            e.message.contains("no JSON body") && e.message.contains("`order_id`"),
            "{}",
            e.message
        );
        let e = extract_all(&map(&[("x", "$[")]), Some(&body)).unwrap_err();
        assert_eq!(e.code, StepCode::ExtractMissing);
        assert!(e.message.contains("bad jsonpath"), "{}", e.message);
        // Первое отсутствующее имя (по алфавиту) останавливает извлечение.
        let e = extract_all(
            &map(&[("a", "$.id"), ("b", "$.nope"), ("c", "$.id")]),
            Some(&body),
        )
        .unwrap_err();
        assert!(e.message.starts_with("`b`"), "{}", e.message);
    }
}
