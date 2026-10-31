# Glafira for Load Testing

A free, open-source desktop tool for preparing and maintaining load tests on macOS, Windows and Linux, built with
Tauri 2, Rust and React. It builds the registry of operations from the contracts of your services, exports the
production-load template to Excel, imports the filled template and computes the load profile. The CLI runner, suites,
mocks and reports come in the next stages.

[Русская версия](README.ru.md)

Apache Kafka is a trademark of the Apache Software Foundation. Other product names are trademarks of their respective
owners. This project is not affiliated with them.

## Install

Installers for macOS (`.dmg`), Windows (`.msi`) and Linux (`.deb`, `.rpm`, `.AppImage`):
https://github.com/vvnovg/glafira-load-releases/releases

Version 0.1.0 is not signed yet, and updates from inside the app come with stage L7: until then, install new versions
from the releases page. The app uses the system `git` to read the version of service clones.

User guide in English and Russian: [English](docs/guide/en/getting-started.md), [Русский](docs/guide/ru/getting-started.md);
it is also built into the app (F1). For administrators (silent install, policy, where data lives):
[`docs/admin.md`](docs/admin.md) (in Russian).

## Features

- **Registry from contracts.** OpenAPI 3.0/3.1 and Swagger 2.0, Protobuf, AsyncAPI 2 and 3, Avro and JSON Schema,
  Flyway and Liquibase migrations and Spring annotations in Java and Kotlin become one registry of HTTP, Kafka, gRPC
  and SQL operations with compact request and response schemas. Contracts are found in the clones automatically; a
  rescan marks new, changed, removed and restored operations and keeps your tags and notes; any two scans can be
  compared.
- **Excel template.** The production-load template has a row per operation with locked IDs and checked columns for
  average and peak RPS, errors, sizes and p95, so the team with access to production monitoring fills in only numbers.
- **Import with a preview.** Filled, empty, unknown and invalid rows and a before/after diff are shown before anything
  is written; invalid rows block the import. Every imported file is kept next to the figures.
- **Load profile with a chart.** Coefficients for the stand and growth with overrides by service and operation,
  profiles of ramp, steady and step phases, targets and shares per operation, operations not covered by figures, and a
  timeline chart.
- **Everything in git.** A load project is a folder of readable YAML files (manifest, registry, scan snapshots,
  production figures, profile) that you commit next to your tests; the clones of the services are only read.

## Stages

| Stage | Contents | Status |
|---|---|---|
| L1. Skeleton and operation registry | Project, services and stands; contract importers and the Spring scanner; registry with diff; Excel template and import; load profile with a phase chart | done (0.1.0) |
| L2. Runner and suite | Suite format, `glafira-load check/run/smoke/report`, HTTP and Kafka generators, metrics, HTML report, stand secrets; L2b — gRPC and SQL | planned |
| L3. Reports in the app | Import of runs, history, charts, comparison, SLA and verdict, export | planned |
| L4. Mocks | `glafira-load mock`: HTTP, gRPC, Kafka, state, record and replay | planned |
| L5. MCP gateway | Read tools for LLM assistants, proposals of scenarios and edits, stand logs with masking | planned |
| L6. Maintenance | Registry diff by versions, affected scenarios, smoke in the app, suite health | planned |
| L7. Languages and distribution | 10 more languages, full guide, signing, automatic updates | planned |

## Development

Requirements:
- Rust stable (`rustup`), Xcode Command Line Tools on macOS, the Tauri system packages on Linux
  (`libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`)
- Node 22+, pnpm 11
- `git` in `PATH`

Run:
```bash
pnpm install
pnpm tauri dev      # the Vite dev server listens on port 1450
```

Contract fixtures for trying the app and for the tests are in `dev/fixtures/` (`orders-openapi`, `orders-swagger`,
`orders-proto`, `orders-asyncapi`, `schemas`, `flyway`, `liquibase`, `spring-java`, `spring-kotlin`): add them as
services of a project.

Tests:
```bash
pnpm test               # frontend
pnpm test:scripts       # scripts/*.mjs
pnpm i18n:check         # dictionaries, keys used by Rust, the guide
cargo test --workspace  # Rust: the app and the crates glafira-load-{model,contracts,xlsx}
```
Logs: `RUST_LOG=debug pnpm tauri dev`. The repository is a Cargo workspace: `src-tauri/` (the app) and `crates/`
(model, contract importers and the Excel template, without Tauri). Design specs are in `docs/superpowers/specs/`;
more developer notes are in [README.ru.md](README.ru.md) (in Russian).

## License and support

- The code is licensed under [Apache License 2.0](LICENSE). Every feature is free for everyone: no paid tiers, no license keys.
- The Glafira name and icon are not covered by the license, see [`TRADEMARK.md`](TRADEMARK.md).
  Dependency licenses: [`THIRD_PARTY.md`](THIRD_PARTY.md).
- The project lives on donations: [support the project](https://github.com/vvnovg/glafira-load-releases/blob/main/SUPPORT.md).
- Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md). Security reports: [`SECURITY.md`](SECURITY.md).
