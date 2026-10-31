//! Порядок применения SQL-миграций для справочника таблиц: changelog Liquibase (его `include`,
//! `includeAll`, `sqlFile` — сразу после него, в порядке документа), затем Flyway `V<версия>__`
//! и числовые префиксы golang-migrate по версии, затем повторяемые Flyway `R__`, затем прочее.
//! Откатные миграции (`*.down.sql`, Flyway `U<версия>__`) в справочник не входят.

use std::collections::{BTreeMap, BTreeSet};

use glafira_load_contracts::sql::liquibase_includes;
use glafira_load_model::project::ContractRef;

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase()
}

/// Версия Flyway из имени `<буква><версия>__…` (`V1__`, `V1_2__`, `V1.2__`); `name` — в нижнем регистре.
fn flyway_version(name: &str, prefix: char) -> Option<Vec<u64>> {
    let (version, _) = name.strip_prefix(prefix)?.split_once("__")?;
    if version.is_empty() {
        return None;
    }
    version.split(['.', '_']).map(|p| p.parse().ok()).collect()
}

/// Числовой префикс golang-migrate и подобных (`0001_init.up.sql`, `20240101_x.sql`, `002.sql`).
fn numeric_prefix(name: &str) -> Option<u64> {
    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Откатная миграция: golang-migrate `*.down.sql`, Flyway `U<версия>__*.sql`.
pub fn is_down(path: &str) -> bool {
    let name = file_name(path);
    name.ends_with(".down.sql") || flyway_version(&name, 'u').is_some()
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    /// Changelog Liquibase (XML/YAML/JSON): его включения встают сразу после него.
    Changelog,
    Versioned(Vec<u64>),
    /// Flyway `R__`: применяются после всех версионных.
    Repeatable,
    Other,
}

fn rank(path: &str) -> Rank {
    let name = file_name(path);
    if [".xml", ".yaml", ".yml", ".json"]
        .iter()
        .any(|e| name.ends_with(e))
    {
        Rank::Changelog
    } else if let Some(v) = flyway_version(&name, 'v') {
        Rank::Versioned(v)
    } else if name.starts_with("r__") {
        Rank::Repeatable
    } else if let Some(n) = numeric_prefix(&name) {
        Rank::Versioned(vec![n])
    } else {
        Rank::Other
    }
}

/// SQL-контракты в порядке применения. `read` читает changelog-файлы (с проверкой границ клона).
/// Откатные миграции вызывающий убирает заранее (`is_down`).
pub fn order(sql: Vec<ContractRef>, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<ContractRef> {
    let mut sql = sql;
    sql.sort_by_cached_key(|c| (rank(&c.path), c.path.clone()));
    // Включения каждого changelog: индексы контрактов в порядке документа.
    let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut included = BTreeSet::new();
    for (i, c) in sql.iter().enumerate() {
        if rank(&c.path) != Rank::Changelog {
            continue;
        }
        let Some(bytes) = read(&c.path) else { continue };
        let mut kids = Vec::new();
        for inc in liquibase_includes(&c.path, &bytes) {
            let matches = |p: &str| {
                if inc.dir {
                    let prefix = format!("{}/", inc.path);
                    p.starts_with(&prefix) || (!inc.relative && p.contains(&format!("/{prefix}")))
                } else {
                    p == inc.path || (!inc.relative && p.ends_with(&format!("/{}", inc.path)))
                }
            };
            // `includeAll` — по алфавиту пути; `sql` уже отсортирован, но по рангу, поэтому досортировываем.
            let mut found: Vec<usize> = (0..sql.len())
                .filter(|&j| j != i && matches(&sql[j].path))
                .collect();
            found.sort_by(|&a, &b| sql[a].path.cmp(&sql[b].path));
            for j in found {
                if !kids.contains(&j) {
                    kids.push(j);
                    included.insert(j);
                }
            }
        }
        children.insert(i, kids);
    }
    fn emit(
        i: usize,
        children: &BTreeMap<usize, Vec<usize>>,
        done: &mut BTreeSet<usize>,
        out: &mut Vec<usize>,
    ) {
        if !done.insert(i) {
            return;
        }
        out.push(i);
        for &k in children.get(&i).into_iter().flatten() {
            emit(k, children, done, out);
        }
    }
    let (mut done, mut out) = (BTreeSet::new(), Vec::new());
    for i in (0..sql.len()).filter(|i| !included.contains(i)) {
        emit(i, &children, &mut done, &mut out);
    }
    // Включённые только по кругу (`a` включает `b`, `b` — `a`): в конце, в базовом порядке.
    for i in 0..sql.len() {
        emit(i, &children, &mut done, &mut out);
    }
    let mut slots: Vec<Option<ContractRef>> = sql.into_iter().map(Some).collect();
    out.into_iter().filter_map(|i| slots[i].take()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::project::ContractKind;

    fn c(path: &str) -> ContractRef {
        ContractRef {
            kind: ContractKind::Sql,
            path: path.into(),
            include: vec![],
            enabled: true,
        }
    }

    fn paths(v: &[ContractRef]) -> Vec<&str> {
        v.iter().map(|c| c.path.as_str()).collect()
    }

    #[test]
    fn down_and_undo_migrations() {
        assert!(is_down("migrations/1_init.down.sql"));
        assert!(is_down("migrations/1_init.DOWN.SQL"));
        assert!(is_down("db/migration/U2__drop.sql"));
        assert!(is_down("db/migration/u2_1__drop.sql"));
        assert!(!is_down("migrations/1_init.up.sql"));
        assert!(!is_down("db/migration/V2__add.sql"));
        assert!(
            !is_down("db/migration/Users.sql"),
            "U без версии — не откат"
        );
    }

    #[test]
    fn flyway_versions_are_numeric_and_repeatables_go_last() {
        let read = |_: &str| None;
        let got = order(
            vec![
                c("db/migration/R__views.sql"),
                c("db/migration/V10__ten.sql"),
                c("db/migration/V2__two.sql"),
                c("db/migration/V1_1__one_one.sql"),
                c("db/migration/sub/V1__one.sql"),
                c("db/migration/V1.2__one_two.sql"),
            ],
            &read,
        );
        assert_eq!(
            paths(&got),
            [
                "db/migration/sub/V1__one.sql",
                "db/migration/V1_1__one_one.sql",
                "db/migration/V1.2__one_two.sql",
                "db/migration/V2__two.sql",
                "db/migration/V10__ten.sql",
                "db/migration/R__views.sql",
            ]
        );
    }

    #[test]
    fn golang_migrate_numeric_prefixes() {
        let read = |_: &str| None;
        let got = order(
            vec![
                c("migrations/20240102000000_b.up.sql"),
                c("migrations/9_a.up.sql"),
                c("migrations/README.sql"),
                c("migrations/10_c.up.sql"),
            ],
            &read,
        );
        assert_eq!(
            paths(&got),
            [
                "migrations/9_a.up.sql",
                "migrations/10_c.up.sql",
                "migrations/20240102000000_b.up.sql",
                "migrations/README.sql",
            ]
        );
    }

    #[test]
    fn liquibase_changelog_order_wins_over_names() {
        let master = br#"<databaseChangeLog>
  <include file="changes/b.xml" relativeToChangelogFile="true"/>
  <include file="db/changelog/changes/a.sql"/>
  <includeAll path="db/changelog/all/"/>
</databaseChangeLog>"#
            .to_vec();
        let b = br#"<databaseChangeLog><sqlFile path="b-data.sql" relativeToChangelogFile="true"/></databaseChangeLog>"#.to_vec();
        let read = move |p: &str| match p {
            "src/main/resources/db/changelog/master.xml" => Some(master.clone()),
            "src/main/resources/db/changelog/changes/b.xml" => Some(b.clone()),
            _ => None,
        };
        let root = "src/main/resources/db/changelog";
        let got = order(
            vec![
                c(&format!("{root}/changes/a.sql")),
                c(&format!("{root}/all/2.sql")),
                c(&format!("{root}/changes/b-data.sql")),
                c(&format!("{root}/all/1.sql")),
                c(&format!("{root}/changes/b.xml")),
                c(&format!("{root}/master.xml")),
            ],
            &read,
        );
        assert_eq!(
            paths(&got),
            [
                "src/main/resources/db/changelog/master.xml",
                "src/main/resources/db/changelog/changes/b.xml",
                "src/main/resources/db/changelog/changes/b-data.sql",
                "src/main/resources/db/changelog/changes/a.sql",
                "src/main/resources/db/changelog/all/1.sql",
                "src/main/resources/db/changelog/all/2.sql",
            ]
        );
    }

    #[test]
    fn include_cycles_do_not_loop() {
        let a = br#"<databaseChangeLog><include file="b.xml" relativeToChangelogFile="true"/></databaseChangeLog>"#.to_vec();
        let b = br#"<databaseChangeLog><include file="a.xml" relativeToChangelogFile="true"/></databaseChangeLog>"#.to_vec();
        let read = move |p: &str| match p {
            "db/changelog/a.xml" => Some(a.clone()),
            "db/changelog/b.xml" => Some(b.clone()),
            _ => None,
        };
        let got = order(
            vec![c("db/changelog/b.xml"), c("db/changelog/a.xml")],
            &read,
        );
        assert_eq!(got.len(), 2);
    }
}
