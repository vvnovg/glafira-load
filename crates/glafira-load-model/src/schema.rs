//! Сжатая схема запроса и ответа и её отпечаток (спека L1 §5.2).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_DEPTH: usize = 6;
pub const MAX_CHILDREN: usize = 200;

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SchemaSummary {
    pub name: String,
    /// `object`, `array`, `string`, `integer`, `number`, `boolean`, `bytes`, `enum`, `one_of`, `any`, имя message у proto.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, rename = "enum", skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// Поля объекта, единственный элемент `items` у массива, варианты у `one_of`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SchemaSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

impl SchemaSummary {
    pub fn leaf(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            ..Default::default()
        }
    }

    pub fn object(name: impl Into<String>, children: Vec<SchemaSummary>) -> Self {
        Self {
            name: name.into(),
            kind: "object".into(),
            children,
            ..Default::default()
        }
    }

    pub fn array(name: impl Into<String>, mut items: SchemaSummary) -> Self {
        items.name = "items".into();
        Self {
            name: name.into(),
            kind: "array".into(),
            children: vec![items],
            ..Default::default()
        }
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn with_example(mut self, example: impl Into<String>) -> Self {
        self.example = Some(example.into());
        self
    }

    /// Обрезает дерево до `MAX_DEPTH` уровней и `MAX_CHILDREN` полей на уровне, помечая обрезанное.
    pub fn limit(&mut self) {
        self.limit_at(1);
    }

    fn limit_at(&mut self, depth: usize) {
        if self.children.len() > MAX_CHILDREN {
            self.children.truncate(MAX_CHILDREN);
            self.truncated = true;
        }
        if depth >= MAX_DEPTH && !self.children.is_empty() {
            self.children.clear();
            self.truncated = true;
            return;
        }
        for c in &mut self.children {
            c.limit_at(depth + 1);
        }
    }

    /// Каноническая форма для отпечатка: без примеров, поля объекта по имени.
    pub fn canonical(&self) -> serde_json::Value {
        let mut children: Vec<&SchemaSummary> = self.children.iter().collect();
        if self.kind == "object" {
            children.sort_by(|a, b| a.name.cmp(&b.name));
        }
        serde_json::json!({
            "n": self.name,
            "t": self.kind,
            "r": self.required,
            "e": self.enum_values,
            "c": children.iter().map(|c| c.canonical()).collect::<Vec<_>>(),
        })
    }
}

/// Пишет JSON в каноническом виде: без пробелов, ключи объектов отсортированы на каждом уровне,
/// массивы сохраняют порядок. Порядок ключей `serde_json::Map` зависит от features сборки
/// (`preserve_order` у соседних крейтов переключает его с `BTreeMap` на `IndexMap`), поэтому
/// на него опираться нельзя.
fn write_canonical(value: &serde_json::Value, out: &mut Vec<u8>) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push(b'{');
            for (i, (key, v)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key).expect("string serializes");
                out.push(b':');
                write_canonical(v, out);
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(v, out);
            }
            out.push(b']');
        }
        scalar => serde_json::to_writer(&mut *out, scalar).expect("json scalar serializes"),
    }
}

/// `sha256:<hex>` канонического JSON (ключи объектов сортируются явно — результат не зависит
/// от features `serde_json` и порядка вставки).
pub fn fingerprint_of(value: &serde_json::Value) -> String {
    let mut bytes = Vec::new();
    write_canonical(value, &mut bytes);
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

pub fn fingerprint(s: &SchemaSummary) -> String {
    fingerprint_of(&s.canonical())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> SchemaSummary {
        SchemaSummary::object(
            "body",
            vec![
                SchemaSummary::leaf("id", "string")
                    .required(true)
                    .with_example("A-1"),
                SchemaSummary::array(
                    "items",
                    SchemaSummary::object("x", vec![SchemaSummary::leaf("sku", "string")]),
                ),
            ],
        )
    }

    #[test]
    fn fingerprint_ignores_examples_and_field_order() {
        let a = order();
        let mut b = order();
        b.children.reverse();
        b.children[1].example = None;
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut c = order();
        c.children[0].kind = "integer".into();
        assert_ne!(fingerprint(&a), fingerprint(&c));
        assert!(fingerprint(&a).starts_with("sha256:"));
    }

    #[test]
    fn array_items_keep_their_order_in_the_fingerprint() {
        let a = SchemaSummary {
            name: "v".into(),
            kind: "one_of".into(),
            children: vec![
                SchemaSummary::leaf("a", "string"),
                SchemaSummary::leaf("b", "integer"),
            ],
            ..Default::default()
        };
        let mut b = a.clone();
        b.children.reverse();
        assert_ne!(
            fingerprint(&a),
            fingerprint(&b),
            "варианты one_of не объект: порядок значим"
        );
    }

    #[test]
    fn fingerprint_is_pinned_to_a_known_value() {
        let s = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("id", "string").required(true)],
        );
        assert_eq!(
            fingerprint(&s),
            "sha256:41142de01731254e6483c6fed38a55ea426036a30042b0916f37b8e40c228908"
        );
    }

    #[test]
    fn fingerprint_of_ignores_object_key_insertion_order() {
        let mut ab = serde_json::Map::new();
        ab.insert("a".into(), serde_json::json!({"x": 1, "y": [2, 1]}));
        ab.insert("b".into(), serde_json::json!("v"));
        let mut ba = serde_json::Map::new();
        ba.insert("b".into(), serde_json::json!("v"));
        ba.insert("a".into(), serde_json::json!({"y": [2, 1], "x": 1}));
        assert_eq!(
            fingerprint_of(&serde_json::Value::Object(ab)),
            fingerprint_of(&serde_json::Value::Object(ba))
        );
    }

    #[test]
    fn limit_cuts_depth_and_width() {
        let mut deep = SchemaSummary::leaf("l", "string");
        for i in 0..10 {
            deep = SchemaSummary::object(format!("o{i}"), vec![deep]);
        }
        deep.limit();
        let mut depth = 1;
        let mut node = &deep;
        while let Some(c) = node.children.first() {
            node = c;
            depth += 1;
        }
        assert_eq!(depth, MAX_DEPTH);
        assert!(node.truncated);

        let wide = SchemaSummary::object(
            "w",
            (0..250)
                .map(|i| SchemaSummary::leaf(format!("f{i}"), "string"))
                .collect(),
        );
        let mut w = wide;
        w.limit();
        assert_eq!(w.children.len(), MAX_CHILDREN);
        assert!(w.truncated);
    }

    #[test]
    fn yaml_round_trip_skips_defaults() {
        let s = SchemaSummary::leaf("id", "string");
        let text = crate::to_yaml(&s);
        assert_eq!(text.trim(), "name: id\ntype: string");
        assert_eq!(crate::from_yaml::<SchemaSummary>(&text).unwrap(), s);
    }
}
