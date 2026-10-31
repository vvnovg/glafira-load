# Changelog

Versions in the other interface languages are in [docs/changelog](docs/changelog/). The section of a version is shown in About as the update notes, in the interface language.

## 0.1.0
Released 2026-10-31. Stage L1 "Skeleton and operation registry" of Glafira for Load Testing, a desktop tool for preparing and maintaining load tests.
- Load project as a folder in git: the manifest `glafira-load.yaml`, services from local git clones with the version from `git describe`, stands without passwords.
- Contract importers: OpenAPI 3.0/3.1 and Swagger 2.0, Protobuf, AsyncAPI 2 and 3, Avro and JSON Schema, Flyway and Liquibase migrations, Spring annotations in Java and Kotlin. Contracts are found in the clone automatically.
- Operation registry with merge on rescan (new, changed, removed, restored; manual fields are kept), scan snapshots and a diff between any two of them; manual operations, including SQL.
- Excel production-load template with locked IDs; import with a preview of filled, empty, unknown and invalid rows and a diff before writing `load/prod-load.yaml`.
- Load profile: coefficients with overrides by service and operation, profiles of ramp, steady and step phases, targets and shares, operations not covered, a timeline chart.
- Languages: English, Russian. Built-in guide (F1). Administrator policy: `updatesEnabled`, `updateUrl`, `language`.
