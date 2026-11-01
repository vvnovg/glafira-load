# Changelog

Versions in the other interface languages are in [docs/changelog](docs/changelog/). The section of a version is shown in About as the update notes, in the interface language.

## 0.2.0
Released 2026-11-xx (replace with the release date when tagging). Stage L2 "Suite, agents and run": load suites, the `glafira-load` CLI with agents on the load machines, and the run window.
- Suite `suites/<name>/suite.yaml`: scenarios of HTTP, `kafka_produce` and `kafka_consume` steps bound to registry operations, `expect` checks (status, JSONPath, response schema, `max_ms`), `extract` between steps, data files (CSV, JSON Lines) and `{{…}}` expressions; unknown fields are errors.
- The Suite tab: scenarios and steps as a tree, `check` with file and line, profile coverage per operation and "not covered", creating a suite from registry operations with bodies generated from request schemas.
- CLI `glafira-load`: `check` (no network), `smoke` (every scenario once, with the broken steps), `run` (in its own process, or on the agents of a stand with `--agents`), `agent` (HTTP API with a bearer token and metrics over SSE). Exit codes: 0 completed, 2 stopped, 3 start error, 4 check or smoke found errors. Binaries for Linux x86_64 and arm64 (static), macOS arm64 and x86_64, Windows x64.
- Load engine: open loop by RPS with Poisson or constant arrivals, `max_in_flight` with `dropped`, HTTP/1.1 and HTTP/2 over rustls, Kafka producer and consumer with lag, Avro via Schema Registry, HDR histograms merged across agents; error samples with masked secrets.
- Agents in the stand of the manifest with weights; tokens in the OS keychain; stand secrets only as `GLAFIRA_LOAD_SECRET_<NAME>` on the agents. A stand without agents runs the load inside the application.
- The Run tab: suite, profile, stand, `run` or `smoke`, agent status, live RPS and p95 charts, counters, operations table, stop, summary, past runs; closing the window during a run asks whether to stop it or leave it to the agents. The results go to `runs/<date>/`: `run.yaml`, `metrics.jsonl`, `phases.yaml`, `errors.jsonl`, `summary.yaml`, raw agent files; an unfinished run is completed from the agents when the project is opened again.
- CI runs the Kafka tests against Redpanda; the release attaches the CLI binaries next to the installers. Guide chapters "Suites", "Agents and stands", "Run"; the administrator guide covers the agent as a service; About shows the agent download command for the current platform.

## 0.1.0
Released 2026-10-31. Stage L1 "Skeleton and operation registry" of Glafira for Load Testing, a desktop tool for preparing and maintaining load tests.
- Load project as a folder in git: the manifest `glafira-load.yaml`, services from local git clones with the version from `git describe`, stands without passwords.
- Contract importers: OpenAPI 3.0/3.1 and Swagger 2.0, Protobuf, AsyncAPI 2 and 3, Avro and JSON Schema, Flyway and Liquibase migrations, Spring annotations in Java and Kotlin. Contracts are found in the clone automatically.
- Operation registry with merge on rescan (new, changed, removed, restored; manual fields are kept), scan snapshots and a diff between any two of them; manual operations, including SQL.
- Excel production-load template with locked IDs; import with a preview of filled, empty, unknown and invalid rows and a diff before writing `load/prod-load.yaml`.
- Load profile: coefficients with overrides by service and operation, profiles of ramp, steady and step phases, targets and shares, operations not covered, a timeline chart.
- Languages: English, Russian. Built-in guide (F1). Administrator policy: `updatesEnabled`, `updateUrl`, `language`.
