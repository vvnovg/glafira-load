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

/// `sha256:<hex>` канонического JSON (ключи у `serde_json::Map` упорядочены — результат стабилен).
pub fn fingerprint_of(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).expect("json value serializes");
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
