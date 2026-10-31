//! JSON Schema (OpenAPI, AsyncAPI, JSON Schema) → `SchemaSummary` с разрешением `$ref`.

use std::cell::RefCell;
use std::collections::BTreeMap;

use glafira_load_model::schema::{MAX_DEPTH, SchemaSummary};
use serde_json::Value;

use crate::{Diagnostic, ReadFile, dir_of, join_path, parse_document};

/// Разрешение ссылок: локальные `#/a/b` в текущем документе, внешние `file.yaml#/a` — через `read`.
pub struct Resolver<'a> {
    pub path: &'a str,
    read: ReadFile<'a>,
    docs: RefCell<BTreeMap<String, Value>>,
    warnings: RefCell<Vec<Diagnostic>>,
}

impl<'a> Resolver<'a> {
    pub fn new(path: &'a str, root: &Value, read: ReadFile<'a>) -> Self {
        let mut docs = BTreeMap::new();
        docs.insert(path.to_string(), root.clone());
        Self {
            path,
            read,
            docs: RefCell::new(docs),
            warnings: RefCell::new(Vec::new()),
        }
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.warnings.borrow_mut().push(Diagnostic {
            path: self.path.into(),
            line: None,
            message: message.into(),
        });
    }

    pub fn take_warnings(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut *self.warnings.borrow_mut())
    }

    /// Документ по пути (кэш); внешние адреса и отсутствующие файлы — предупреждение и `None`.
    fn document(&self, file: &str) -> Option<Value> {
        if let Some(d) = self.docs.borrow().get(file) {
            return Some(d.clone());
        }
        let bytes = (self.read)(file)?;
        match parse_document(&bytes) {
            Ok(v) => {
                self.docs.borrow_mut().insert(file.into(), v.clone());
                Some(v)
            }
            Err(e) => {
                self.warn(format!("{file}: {}", e.message));
                None
            }
        }
    }

    /// `$ref` относительно документа `doc_file` → (файл, узел).
    pub fn resolve(&self, doc_file: &str, reference: &str) -> Option<(String, Value)> {
        if reference.contains("://") {
            self.warn(format!("external $ref is not supported: {reference}"));
            return None;
        }
        let (file_part, pointer) = reference.split_once('#').unwrap_or((reference, ""));
        let file = if file_part.is_empty() {
            doc_file.to_string()
        } else {
            join_path(dir_of(doc_file), file_part)
        };
        let Some(doc) = self.document(&file) else {
            // Файла нет, он вне клона (замыкание вернуло `None`) или не разобран.
            self.warn(format!("$ref not found: {reference}"));
            return None;
        };
        let node = if pointer.is_empty() {
            Some(&doc)
        } else {
            doc.pointer(&pointer.replace("~1", "/").replace("~0", "~"))
        };
        match node {
            Some(n) => Some((file, n.clone())),
            None => {
                self.warn(format!("$ref not found: {reference}"));
                None
            }
        }
    }
}

fn short(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > 64 {
        s.chars().take(61).collect::<String>() + "…"
    } else {
        s
    }
}

fn type_of(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .map(String::from),
        _ => None,
    }
}

/// Сжатие схемы `schema` из документа `doc_file`. `stack` — цепочка `$ref` против циклов.
pub fn summarize(
    name: &str,
    schema: &Value,
    required: bool,
    doc_file: &str,
    r: &Resolver,
    depth: usize,
    stack: &mut Vec<String>,
) -> SchemaSummary {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if stack.iter().any(|s| s == reference) {
            return SchemaSummary::leaf(
                name,
                format!("ref:{}", reference.rsplit('/').next().unwrap_or(reference)),
            )
            .required(required);
        }
        let Some((file, node)) = r.resolve(doc_file, reference) else {
            return SchemaSummary::leaf(name, "unresolved").required(required);
        };
        stack.push(reference.to_string());
        let out = summarize(name, &node, required, &file, r, depth, stack);
        stack.pop();
        return out;
    }
    let mut out = SchemaSummary::leaf(name, "any").required(required);
    if depth >= MAX_DEPTH {
        out.truncated = true;
        return out;
    }
    let example = schema
        .get("example")
        .or_else(|| schema.get("default"))
        .or_else(|| schema.get("examples").and_then(|e| e.get(0)));
    out.example = example.map(short);
    if let Some(variants) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        out.kind = "one_of".into();
        out.children = variants
            .iter()
            .enumerate()
            .map(|(i, v)| {
                summarize(
                    &format!("variant{}", i + 1),
                    v,
                    false,
                    doc_file,
                    r,
                    depth + 1,
                    stack,
                )
            })
            .collect();
        return out;
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        out.kind = "object".into();
        for p in parts {
            let s = summarize(name, p, false, doc_file, r, depth, stack);
            out.children.extend(s.children);
        }
        return out;
    }
    let kind = type_of(schema).unwrap_or_else(|| {
        if schema.get("properties").is_some() {
            "object".into()
        } else if schema.get("items").is_some() {
            "array".into()
        } else if schema.get("enum").is_some() {
            "string".into()
        } else {
            "any".into()
        }
    });
    out.kind = kind.clone();
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        out.enum_values = e.iter().map(short).collect();
    }
    match kind.as_str() {
        "object" => {
            let required_names: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                out.children = props
                    .iter()
                    .map(|(n, s)| {
                        summarize(
                            n,
                            s,
                            required_names.contains(&n.as_str()),
                            doc_file,
                            r,
                            depth + 1,
                            stack,
                        )
                    })
                    .collect();
            }
        }
        "array" => {
            if let Some(items) = schema.get("items") {
                out.children = vec![summarize(
                    "items",
                    items,
                    false,
                    doc_file,
                    r,
                    depth + 1,
                    stack,
                )];
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_local_external_and_cyclic_refs() {
        let root: Value = serde_json::json!({
            "components": { "schemas": {
                "Node": { "type": "object", "properties": { "next": { "$ref": "#/components/schemas/Node" }, "ext": { "$ref": "./other.json#/X" } } }
            } }
        });
        let read = |p: &str| {
            (p == "api/other.json").then(|| {
                br#"{"X": {"type": "string", "enum": ["a", "b"], "example": "a"}}"#.to_vec()
            })
        };
        let r = Resolver::new("api/openapi.json", &root, &read);
        let s = summarize(
            "body",
            &serde_json::json!({ "$ref": "#/components/schemas/Node" }),
            true,
            "api/openapi.json",
            &r,
            1,
            &mut vec![],
        );
        assert_eq!(s.kind, "object");
        assert!(s.required);
        let next = s.children.iter().find(|c| c.name == "next").unwrap();
        assert_eq!(next.kind, "ref:Node", "цикл обрывается ссылкой на имя");
        let ext = s.children.iter().find(|c| c.name == "ext").unwrap();
        assert_eq!(
            (
                ext.kind.as_str(),
                ext.enum_values.clone(),
                ext.example.as_deref()
            ),
            ("string", vec!["a".into(), "b".into()], Some("a"))
        );
        assert!(r.take_warnings().is_empty());
    }

    #[test]
    fn unknown_refs_and_urls_warn_but_do_not_fail() {
        let root = serde_json::json!({});
        let read = |_: &str| None;
        let r = Resolver::new("a.yaml", &root, &read);
        let s = summarize(
            "x",
            &serde_json::json!({ "$ref": "#/nope" }),
            false,
            "a.yaml",
            &r,
            1,
            &mut vec![],
        );
        assert_eq!(s.kind, "unresolved");
        summarize(
            "y",
            &serde_json::json!({ "$ref": "https://x/y.json" }),
            false,
            "a.yaml",
            &r,
            1,
            &mut vec![],
        );
        let w = r.take_warnings();
        assert_eq!(w.len(), 2);
        assert!(w[1].message.contains("external $ref"));
    }

    #[test]
    fn nullable_type_arrays_one_of_and_all_of() {
        let root = serde_json::json!({});
        let read = |_: &str| None;
        let r = Resolver::new("a.yaml", &root, &read);
        let s = summarize(
            "v",
            &serde_json::json!({ "type": ["string", "null"] }),
            false,
            "a.yaml",
            &r,
            1,
            &mut vec![],
        );
        assert_eq!(s.kind, "string");
        let s = summarize(
            "v",
            &serde_json::json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] }),
            false,
            "a.yaml",
            &r,
            1,
            &mut vec![],
        );
        assert_eq!((s.kind.as_str(), s.children.len()), ("one_of", 2));
        let s = summarize(
            "v",
            &serde_json::json!({ "allOf": [{ "type": "object", "properties": { "a": { "type": "string" } } }, { "type": "object", "properties": { "b": { "type": "integer" } } }] }),
            false,
            "a.yaml",
            &r,
            1,
            &mut vec![],
        );
        assert_eq!(
            s.children
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }
}
