//! Дифф сжатых схем и снимков разбора (спека L1 §6.2–§6.3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::merge::MergeSummary;
use crate::schema::SchemaSummary;

pub const SCAN_FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum FieldChange {
    Added {
        path: String,
        kind: String,
    },
    Removed {
        path: String,
        kind: String,
    },
    TypeChanged {
        path: String,
        from: String,
        to: String,
    },
    RequiredChanged {
        path: String,
        required: bool,
    },
    AddressChanged {
        from: String,
        to: String,
    },
}

/// Изменения между двумя схемами; `prefix` — `request`, `response`, `responses.404`.
pub fn schema_diff(
    prefix: &str,
    old: Option<&SchemaSummary>,
    new: Option<&SchemaSummary>,
) -> Vec<FieldChange> {
    let mut out = Vec::new();
    match (old, new) {
        (None, None) => {}
        (None, Some(n)) => out.push(FieldChange::Added {
            path: prefix.into(),
            kind: n.kind.clone(),
        }),
        (Some(o), None) => out.push(FieldChange::Removed {
            path: prefix.into(),
            kind: o.kind.clone(),
        }),
        (Some(o), Some(n)) => diff_node(prefix, o, n, &mut out),
    }
    out
}

fn kind_label(s: &SchemaSummary) -> String {
    if s.enum_values.is_empty() {
        s.kind.clone()
    } else {
        format!("{}[{}]", s.kind, s.enum_values.join("|"))
    }
}

fn diff_node(path: &str, o: &SchemaSummary, n: &SchemaSummary, out: &mut Vec<FieldChange>) {
    if o.kind != n.kind || o.enum_values != n.enum_values {
        out.push(FieldChange::TypeChanged {
            path: path.into(),
            from: kind_label(o),
            to: kind_label(n),
        });
        if o.kind != n.kind {
            return;
        }
    }
    if o.required != n.required {
        out.push(FieldChange::RequiredChanged {
            path: path.into(),
            required: n.required,
        });
    }
    let old: BTreeMap<&str, &SchemaSummary> =
        o.children.iter().map(|c| (c.name.as_str(), c)).collect();
    let new: BTreeMap<&str, &SchemaSummary> =
        n.children.iter().map(|c| (c.name.as_str(), c)).collect();
    for (name, c) in &new {
        let child = format!("{path}.{name}");
        match old.get(name) {
            Some(oc) => diff_node(&child, oc, c, out),
            None => out.push(FieldChange::Added {
                path: child,
                kind: c.kind.clone(),
            }),
        }
    }
    for (name, c) in &old {
        if !new.contains_key(name) {
            out.push(FieldChange::Removed {
                path: format!("{path}.{name}"),
                kind: c.kind.clone(),
            });
        }
    }
}

/// Снимок разбора `registry/scans/<метка>.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scan {
    pub format: u32,
    /// RFC 3339, UTC.
    pub at: String,
    pub service: String,
    pub version: String,
    pub commit: String,
    pub summary: MergeSummary,
    /// Отпечатки всех активных операций реестра после слияния (всех сервисов).
    pub fingerprints: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RegistryDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

/// Что изменилось между двумя снимками по отпечаткам.
pub fn registry_diff(from: &Scan, to: &Scan) -> RegistryDiff {
    let mut d = RegistryDiff::default();
    for (id, fp) in &to.fingerprints {
        match from.fingerprints.get(id) {
            None => d.added.push(id.clone()),
            Some(old) if old != fp => d.changed.push(id.clone()),
            Some(_) => {}
        }
    }
    d.removed = from
        .fingerprints
        .keys()
        .filter(|id| !to.fingerprints.contains_key(*id))
        .cloned()
        .collect();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_diff_reports_added_removed_type_and_required() {
        let old = SchemaSummary::object(
            "body",
            vec![
                SchemaSummary::leaf("id", "string").required(true),
                SchemaSummary::leaf("qty", "integer"),
                SchemaSummary::array("tags", SchemaSummary::leaf("x", "string")),
            ],
        );
        let new = SchemaSummary::object(
            "body",
            vec![
                SchemaSummary::leaf("id", "string"),
                SchemaSummary::leaf("qty", "number"),
                SchemaSummary::array("tags", SchemaSummary::leaf("x", "integer")),
                SchemaSummary::leaf("note", "string"),
            ],
        );
        let d = schema_diff("request", Some(&old), Some(&new));
        assert_eq!(
            d,
            vec![
                FieldChange::RequiredChanged {
                    path: "request.id".into(),
                    required: false
                },
                FieldChange::Added {
                    path: "request.note".into(),
                    kind: "string".into()
                },
                FieldChange::TypeChanged {
                    path: "request.qty".into(),
                    from: "integer".into(),
                    to: "number".into()
                },
                FieldChange::TypeChanged {
                    path: "request.tags.items".into(),
                    from: "string".into(),
                    to: "integer".into()
                },
            ]
        );
        assert_eq!(
            schema_diff("response", None, Some(&new)),
            vec![FieldChange::Added {
                path: "response".into(),
                kind: "object".into()
            }]
        );
        assert_eq!(
            schema_diff("response", Some(&old), None),
            vec![FieldChange::Removed {
                path: "response".into(),
                kind: "object".into()
            }]
        );
    }

    #[test]
    fn registry_diff_compares_fingerprints() {
        let scan = |pairs: &[(&str, &str)]| Scan {
            format: SCAN_FORMAT,
            at: String::new(),
            service: "s".into(),
            version: "v".into(),
            commit: "c".into(),
            summary: MergeSummary::default(),
            fingerprints: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        let d = registry_diff(
            &scan(&[("a", "1"), ("b", "1"), ("c", "1")]),
            &scan(&[("a", "1"), ("b", "2"), ("d", "1")]),
        );
        assert_eq!(
            d,
            RegistryDiff {
                added: vec!["d".into()],
                removed: vec!["c".into()],
                changed: vec!["b".into()]
            }
        );
    }
}
