//! Слияние результата разбора с реестром (спека L1 §6.2).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::diff::{FieldChange, schema_diff};
use crate::registry::{
    Manual, Operation, OperationDraft, Registry, Seen, SourceKind, Status, address_label,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangedOperation {
    pub id: String,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MergeSummary {
    pub added: Vec<String>,
    pub changed: Vec<ChangedOperation>,
    pub removed: Vec<String>,
    pub restored: Vec<String>,
    pub unchanged: usize,
}

impl MergeSummary {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.changed.is_empty()
            && self.removed.is_empty()
            && self.restored.is_empty()
    }
}

/// Сливает черновики одного сервиса в реестр. Черновик чужого сервиса и повтор `id` в одном
/// разборе пропускаются (первый побеждает). Идемпотентно: повторное слияние тех же черновиков
/// даёт пустую сводку и тот же файл.
///
/// Правило `last_seen`: у неизменившейся операции (тот же отпечаток), у которой сохранённые
/// `last_seen.version` и `last_seen.commit` совпадают с `seen`, `last_seen` остаётся прежним —
/// повторный разбор той же версии не меняет файл реестра (в том числе время `at`). Во всех
/// остальных случаях — новая версия или коммит, изменившаяся, восстановленная или новая
/// операция — `last_seen = seen`. Так у клона без версии (`unversioned`) или грязного дерева
/// изменившаяся операция всё равно получает свежий `last_seen`.
pub fn merge(
    registry: &mut Registry,
    service: &str,
    drafts: Vec<OperationDraft>,
    seen: &Seen,
) -> MergeSummary {
    let mut summary = MergeSummary::default();
    let mut ids = BTreeSet::new();
    for draft in drafts {
        if draft.service != service || !ids.insert(draft.id.clone()) {
            continue;
        }
        let id = draft.id.clone();
        let fp = draft.fingerprint();
        match registry.get_mut(&id) {
            Some(op) => {
                let was_removed = op.status == Status::Removed;
                let changed = op.fingerprint != fp;
                let mut changes = Vec::new();
                if changed {
                    changes.extend(schema_diff(
                        "request",
                        op.draft.request.as_ref(),
                        draft.request.as_ref(),
                    ));
                    changes.extend(schema_diff(
                        "response",
                        op.draft.response.as_ref(),
                        draft.response.as_ref(),
                    ));
                    let codes: BTreeSet<&String> = op
                        .draft
                        .responses
                        .keys()
                        .chain(draft.responses.keys())
                        .collect();
                    for code in codes {
                        changes.extend(schema_diff(
                            &format!("responses.{code}"),
                            op.draft.responses.get(code),
                            draft.responses.get(code),
                        ));
                    }
                    if op.draft.address != draft.address {
                        changes.push(FieldChange::AddressChanged {
                            from: address_label(&op.draft.address),
                            to: address_label(&draft.address),
                        });
                    }
                }
                let same_version =
                    op.last_seen.version == seen.version && op.last_seen.commit == seen.commit;
                op.draft = draft;
                op.fingerprint = fp;
                if changed || was_removed || !same_version {
                    op.last_seen = seen.clone();
                }
                op.status = Status::Active;
                if was_removed {
                    summary.restored.push(id);
                } else if changed {
                    summary.changed.push(ChangedOperation { id, changes });
                } else {
                    summary.unchanged += 1;
                }
            }
            None => {
                summary.added.push(id);
                registry.operations.push(Operation {
                    draft,
                    fingerprint: fp,
                    status: Status::Active,
                    first_seen: seen.clone(),
                    last_seen: seen.clone(),
                    manual: Manual::default(),
                });
            }
        }
    }
    for op in registry.operations.iter_mut().filter(|o| {
        o.draft.service == service
            && o.status == Status::Active
            && o.draft.source.kind != SourceKind::Manual
            && !ids.contains(&o.draft.id)
    }) {
        op.status = Status::Removed;
        summary.removed.push(op.draft.id.clone());
    }
    registry.sort();
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Address, OpKind, Source};
    use crate::schema::SchemaSummary;

    fn seen(v: &str) -> Seen {
        Seen {
            version: v.into(),
            commit: format!("c-{v}"),
            at: "2026-10-31T10:00:00Z".into(),
        }
    }

    fn http(path: &str, body: Option<SchemaSummary>) -> OperationDraft {
        let mut d = OperationDraft::new(
            "orders",
            OpKind::Http,
            Address::Http {
                method: "POST".into(),
                path: path.into(),
            },
            Source {
                kind: SourceKind::Openapi,
                path: "api.yaml".into(),
                line: Some(1),
            },
        );
        d.request = body;
        d
    }

    #[test]
    fn add_change_remove_restore_and_manual_fields_survive() {
        let mut r = Registry::default();
        let s1 = merge(
            &mut r,
            "orders",
            vec![http("/a", None), http("/b", None)],
            &seen("v1"),
        );
        assert_eq!(s1.added, vec!["http:orders:POST /a", "http:orders:POST /b"]);
        r.get_mut("http:orders:POST /a").unwrap().manual.notes = "keep me".into();

        let s2 = merge(
            &mut r,
            "orders",
            vec![http("/a", Some(SchemaSummary::leaf("body", "object")))],
            &seen("v2"),
        );
        assert_eq!(s2.changed[0].id, "http:orders:POST /a");
        assert_eq!(
            s2.changed[0].changes,
            vec![FieldChange::Added {
                path: "request".into(),
                kind: "object".into()
            }]
        );
        assert_eq!(s2.removed, vec!["http:orders:POST /b"]);
        let a = r.get("http:orders:POST /a").unwrap();
        assert_eq!(a.manual.notes, "keep me");
        assert_eq!(
            (a.first_seen.version.as_str(), a.last_seen.version.as_str()),
            ("v1", "v2")
        );
        let b = r.get("http:orders:POST /b").unwrap();
        assert_eq!(
            (b.status, b.last_seen.version.as_str()),
            (Status::Removed, "v1")
        );

        let s3 = merge(
            &mut r,
            "orders",
            vec![
                http("/a", Some(SchemaSummary::leaf("body", "object"))),
                http("/b", None),
            ],
            &seen("v3"),
        );
        assert_eq!(s3.restored, vec!["http:orders:POST /b"]);
        assert_eq!(s3.unchanged, 1);
        assert_eq!(r.get("http:orders:POST /b").unwrap().status, Status::Active);
    }

    #[test]
    fn manual_and_foreign_operations_are_not_removed() {
        let mut r = Registry::default();
        merge(
            &mut r,
            "billing",
            vec![{
                let mut d = http("/pay", None);
                d.service = "billing".into();
                d.id = "http:billing:POST /pay".into();
                d
            }],
            &seen("v1"),
        );
        let mut manual = OperationDraft::new(
            "orders",
            OpKind::Sql,
            Address::Sql { name: "x".into() },
            Source::manual(),
        );
        manual.name = "x".into();
        r.operations.push(Operation {
            fingerprint: manual.fingerprint(),
            draft: manual,
            status: Status::Active,
            first_seen: seen("v1"),
            last_seen: seen("v1"),
            manual: Manual::default(),
        });
        let s = merge(&mut r, "orders", vec![], &seen("v2"));
        assert!(s.removed.is_empty());
        assert_eq!(r.operations.len(), 2);
        assert!(r.operations.iter().all(|o| o.status == Status::Active));
    }

    fn seen_at(v: &str, commit: &str, at: &str) -> Seen {
        Seen {
            version: v.into(),
            commit: commit.into(),
            at: at.into(),
        }
    }

    const ID: &str = "http:orders:POST /a";

    fn body() -> Option<SchemaSummary> {
        Some(SchemaSummary::leaf("body", "object"))
    }

    #[test]
    fn unchanged_operation_with_the_same_version_keeps_last_seen() {
        let mut r = Registry::default();
        merge(
            &mut r,
            "orders",
            vec![http("/a", None)],
            &seen_at("v1", "c1", "t1"),
        );
        let s = merge(
            &mut r,
            "orders",
            vec![http("/a", None)],
            &seen_at("v1", "c1", "t2"),
        );
        assert_eq!(s.unchanged, 1);
        assert_eq!(r.get(ID).unwrap().last_seen, seen_at("v1", "c1", "t1"));
    }

    #[test]
    fn unchanged_operation_with_a_new_commit_updates_last_seen() {
        let mut r = Registry::default();
        merge(
            &mut r,
            "orders",
            vec![http("/a", None)],
            &seen_at("v1", "c1", "t1"),
        );
        merge(
            &mut r,
            "orders",
            vec![http("/a", None)],
            &seen_at("v1", "c2", "t2"),
        );
        assert_eq!(r.get(ID).unwrap().last_seen, seen_at("v1", "c2", "t2"));
    }

    #[test]
    fn changed_operation_with_the_same_version_updates_last_seen() {
        let mut r = Registry::default();
        merge(
            &mut r,
            "orders",
            vec![http("/a", None)],
            &seen_at("v1", "c1", "t1"),
        );
        let s = merge(
            &mut r,
            "orders",
            vec![http("/a", body())],
            &seen_at("v1", "c1", "t2"),
        );
        assert_eq!(s.changed.len(), 1);
        assert_eq!(r.get(ID).unwrap().last_seen, seen_at("v1", "c1", "t2"));
    }

    #[test]
    fn unversioned_clone_with_a_changed_operation_updates_last_seen() {
        let mut r = Registry::default();
        let first = seen_at("unversioned", "", "t1");
        merge(
            &mut r,
            "orders",
            vec![http("/a", None), http("/b", None)],
            &first,
        );
        let second = seen_at("unversioned", "", "t2");
        merge(
            &mut r,
            "orders",
            vec![http("/a", body()), http("/b", None)],
            &second,
        );
        assert_eq!(r.get(ID).unwrap().last_seen, second);
        assert_eq!(
            r.get("http:orders:POST /b").unwrap().last_seen,
            first,
            "неизменившаяся операция той же версии"
        );
        // Восстановленная операция той же версии тоже получает новый `last_seen`.
        merge(&mut r, "orders", vec![http("/a", body())], &second);
        let third = seen_at("unversioned", "", "t3");
        merge(
            &mut r,
            "orders",
            vec![http("/a", body()), http("/b", None)],
            &third,
        );
        assert_eq!(r.get("http:orders:POST /b").unwrap().last_seen, third);
    }

    #[test]
    fn merge_is_idempotent_and_sorted() {
        let mut r = Registry::default();
        merge(
            &mut r,
            "orders",
            vec![http("/z", None), http("/a", None)],
            &seen("v1"),
        );
        let text1 = crate::to_yaml(&r);
        let s = merge(
            &mut r,
            "orders",
            vec![http("/a", None), http("/z", None), http("/a", None)],
            &seen("v1"),
        );
        assert!(s.is_empty());
        assert_eq!(s.unchanged, 2, "дубликат в разборе не считается");
        assert_eq!(crate::to_yaml(&r), text1);
        assert_eq!(r.operations[0].draft.id, "http:orders:POST /a");
    }
}
