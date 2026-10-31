//! Автопоиск контрактов в клоне по маскам (спека L1 §5.1).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use glafira_load_model::project::{ContractKind, ContractRef};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::scan::migrations::is_down;

pub const SKIP_DIRS: [&str; 8] = [
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    "vendor",
    ".idea",
    ".gradle",
];

const PATTERNS: [(ContractKind, &[&str]); 6] = [
    (
        ContractKind::Openapi,
        &[
            "**/openapi*.{yaml,yml,json}",
            "**/swagger*.{yaml,yml,json}",
            "**/api-docs*.json",
        ],
    ),
    (ContractKind::Asyncapi, &["**/asyncapi*.{yaml,yml,json}"]),
    (ContractKind::Proto, &["**/*.proto"]),
    (ContractKind::Schema, &["**/*.avsc", "**/*.schema.json"]),
    (
        ContractKind::Sql,
        &[
            "**/db/migration/**/*.sql",
            "**/migrations/**/*.sql",
            "**/db/changelog/**/*.{xml,yaml,yml,sql}",
        ],
    ),
    (
        ContractKind::Spring,
        &["**/src/main/java/**/*.java", "**/src/main/kotlin/**/*.kt"],
    ),
];

static SETS: LazyLock<Vec<(ContractKind, GlobSet)>> = LazyLock::new(|| {
    PATTERNS
        .iter()
        .map(|(kind, globs)| {
            let mut b = GlobSetBuilder::new();
            for g in *globs {
                b.add(Glob::new(g).expect("static glob"));
            }
            (*kind, b.build().expect("static globset"))
        })
        .collect()
});

fn is_jvm_project(clone: &Path) -> bool {
    ["pom.xml", "build.gradle", "build.gradle.kts"]
        .iter()
        .any(|f| clone.join(f).is_file())
}

/// Каталоги `import` для proto-файла: все его предки и корень клона — `import "common/x.proto"`
/// обычно считается от каталога `proto/`, который может лежать на любой глубине.
fn proto_includes(rel: &str) -> Vec<String> {
    let mut set = BTreeSet::from([String::new()]);
    let mut dir = rel;
    while let Some(i) = dir.rfind('/') {
        dir = &dir[..i];
        set.insert(dir.to_string());
    }
    set.into_iter().collect()
}

/// Контракты клона по маскам. Пути — относительно канонического корня клона, разделитель `/`.
/// Служебные каталоги (`SKIP_DIRS`) и файлы из `.gitignore` пропускаются, ссылки не обходятся;
/// откатные миграции (`*.down.sql`, Flyway `U<версия>__`) не попадают в список.
pub fn discover(clone: &Path) -> Vec<ContractRef> {
    let clone = dunce::canonicalize(clone).unwrap_or_else(|_| clone.to_path_buf());
    let jvm = is_jvm_project(&clone);
    let mut out = Vec::new();
    let walk = WalkBuilder::new(&clone)
        .hidden(false)
        .git_ignore(true)
        // Сам корень клона может называться, например, `build` — проверяются только вложенные.
        .filter_entry(|e| {
            e.depth() == 0
                || e.file_name()
                    .to_str()
                    .map(|n| !SKIP_DIRS.contains(&n))
                    .unwrap_or(true)
        })
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let rel = crate::project::paths::relative(&clone, entry.path());
        let Some((kind, _)) = SETS.iter().find(|(_, set)| set.is_match(&rel)) else {
            continue;
        };
        if *kind == ContractKind::Spring && !jvm {
            continue;
        }
        if *kind == ContractKind::Sql && is_down(&rel) {
            continue;
        }
        let include = if *kind == ContractKind::Proto {
            proto_includes(&rel)
        } else {
            vec![]
        };
        out.push(ContractRef {
            kind: *kind,
            path: rel,
            include,
            enabled: true,
        });
    }
    out.sort_by(|a, b| (a.kind, a.path.as_str()).cmp(&(b.kind, b.path.as_str())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_fixture_contracts_by_kind() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dev/fixtures");
        let found = discover(&fixtures);
        let by_kind = |k: ContractKind| {
            found
                .iter()
                .filter(|c| c.kind == k)
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            by_kind(ContractKind::Openapi),
            ["orders-openapi/openapi.yaml", "orders-swagger/swagger.json"]
        );
        assert_eq!(by_kind(ContractKind::Asyncapi).len(), 2);
        assert_eq!(
            by_kind(ContractKind::Proto),
            [
                "orders-proto/proto/common/money.proto",
                "orders-proto/proto/orders/v1/orders.proto"
            ]
        );
        assert_eq!(
            by_kind(ContractKind::Schema),
            ["schemas/orders-created.avsc", "schemas/payment.schema.json"]
        );
        assert_eq!(by_kind(ContractKind::Sql).len(), 4);
        assert!(
            by_kind(ContractKind::Spring).is_empty(),
            "pom.xml лежит не в корне fixtures"
        );
        let proto = found
            .iter()
            .find(|c| c.path.ends_with("orders.proto"))
            .unwrap();
        assert_eq!(
            proto.include,
            [
                "",
                "orders-proto",
                "orders-proto/proto",
                "orders-proto/proto/orders",
                "orders-proto/proto/orders/v1"
            ]
        );
        let spring = discover(&fixtures.join("spring-java"));
        assert_eq!(
            spring.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            [
                "src/main/java/demo/OrderController.java",
                "src/main/java/demo/OrderListener.java"
            ]
        );
        let kotlin = discover(&fixtures.join("spring-kotlin"));
        assert_eq!(
            kotlin.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["src/main/kotlin/demo/PaymentController.kt"]
        );
    }

    #[test]
    fn skips_service_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(
            dir.path().join("node_modules/x/openapi.yaml"),
            "openapi: 3.0.0",
        )
        .unwrap();
        std::fs::write(dir.path().join("openapi.yaml"), "openapi: 3.0.0").unwrap();
        assert_eq!(discover(dir.path()).len(), 1);
    }

    #[test]
    fn down_migrations_are_not_contracts_and_clone_named_like_a_skip_dir_is_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("build");
        let m = dir.join("migrations");
        std::fs::create_dir_all(&m).unwrap();
        for f in [
            "1_init.up.sql",
            "1_init.down.sql",
            "2_more.UP.sql",
            "2_more.DOWN.sql",
        ] {
            std::fs::write(m.join(f), "SELECT 1;").unwrap();
        }
        let fly = dir.join("db/migration");
        std::fs::create_dir_all(&fly).unwrap();
        std::fs::write(fly.join("V1__a.sql"), "SELECT 1;").unwrap();
        std::fs::write(fly.join("U1__a.sql"), "SELECT 1;").unwrap();
        let paths: Vec<String> = discover(&dir).into_iter().map(|c| c.path).collect();
        assert_eq!(
            paths,
            [
                "db/migration/V1__a.sql",
                "migrations/1_init.up.sql",
                "migrations/2_more.UP.sql"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_the_clone_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("openapi.yaml"), "openapi: 3.0.0").unwrap();
        std::os::unix::fs::symlink(other.path(), dir.path().join("linked")).unwrap();
        std::os::unix::fs::symlink(
            other.path().join("openapi.yaml"),
            dir.path().join("swagger.yaml"),
        )
        .unwrap();
        assert!(discover(dir.path()).is_empty());
    }
}
