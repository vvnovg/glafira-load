# Operation registry

The operation registry lists every operation of the system under test with its request and response schemas. It is
built from the contracts of the services and kept in `registry/operations.yaml`. Open it with [[ui:sidebar.registry]]
in the left panel.

## What an operation is {#operations}

An operation is one thing the load will call or produce. There are five kinds, and the kind defines its ID:

| Kind | ID | Where it comes from |
|---|---|---|
| HTTP | `http:<service>:<METHOD> <path>`, the path as in the contract, with `{param}` | OpenAPI 3.0/3.1, Swagger 2.0, Spring controllers |
| Kafka produce | `kafka_produce:<topic>` | AsyncAPI `publish`/`send`, `kafkaTemplate.send("topic", …)` in Spring, schema files |
| Kafka consume | `kafka_consume:<topic>` | AsyncAPI `subscribe`/`receive`, `@KafkaListener(topics = …)` in Spring |
| gRPC | `grpc:<service>:<package.Service/Method>` | `rpc` in `.proto` files; streaming is marked `client`, `server` or `bidi` |
| SQL | `sql:<service>:<name>` | added by hand, the name is yours |

Kafka IDs have no service: a topic is shared by everyone who writes to it or reads from it. The ID is how the
production-load template and the load profile find the operation, so it stays the same as long as the contract does.

The table can be filtered by service, kind, source and status and searched by ID or name. Select a row to see the
address, the source file and line ([[ui:service.openFile]] opens it), the versions where the operation was first and
last seen, and the schemas.

## Schemas {#schemas}

Request and response schemas are shown as a compact tree: field name, type, enum values and an example if the contract
has one. A star `*` marks a required field. The tree is limited to 6 levels and 200 fields per level; anything beyond is
cut and marked [[ui:registry.truncated]]. A type that refers to itself is cut at the repetition and shown as
`ref:<Type>`. A `$ref` that cannot be found becomes the type `unresolved`, and the scan report has a warning about it.

For HTTP, the response is the first 2xx answer; other codes are listed by number. A fingerprint of the schemas (without
examples and descriptions) tells the registry whether an operation changed.

## Scanning again {#rescan}

Each [[ui:service.scan]] merges the new result into the registry:

- an operation with a new ID is **new**;
- an operation whose schemas or address changed is **changed**; its schemas and source are replaced;
- an operation of the service that is no longer in the contracts is **removed**: it stays in the registry with the
  status [[ui:registry.status.removed]] and the version where it was last seen;
- a removed operation that appears again is **restored** and becomes [[ui:registry.status.active]].

Your [[ui:registry.manual]] — tags, owner and notes — are never touched by a scan. A removed operation is deleted for
good with [[ui:registry.deleteRemoved]]. Scanning again without changes in the contracts leaves the file byte for byte
the same, and operations are sorted by service and ID, so `git diff` stays readable.

## Diff {#diff}

Every scan writes a snapshot to `registry/scans/`. Turn on [[ui:registry.diffToggle]] to mark the rows that differ
between the last two snapshots: new, changed or removed. In the scans section under the table, choose any two snapshots
with [[ui:registry.diffFrom]] and [[ui:registry.diffTo]]. The snapshot also records which fields changed (added,
removed, changed type or required); [[ui:service.openScan]] in the scan report on the service tab opens it.

## Manual operations {#manual}

Some operations have no contract: SQL queries, an endpoint of an external system, a topic nobody described. Add them
with [[ui:registry.addManual]]: choose the service and the kind, fill in the address (for SQL, an operation name such as
`find-order-by-id`) and a description. The form shows the resulting ID. Manual operations are never changed or removed
by a scan; [[ui:common.delete]] in the operation panel removes one.

For SQL, [[ui:registry.tables]] lists the tables and columns that Glafira found in the migrations of the services
(Flyway, Liquibase XML and formatted SQL, golang-migrate). Migrations do not create operations: they help you name the
queries your load will run.

## Importer warnings {#warnings}

Glafira reads contracts without compiling or running anything, so some things cannot be computed. They become warnings
with the file and line in the scan report; the rest of the file is still parsed.

- **Spring.** A path given by a constant (`@GetMapping(Paths.SEARCH)`) is not computed and the operation is skipped;
  `@RequestMapping` without a method is skipped; a topic that is not a string literal is skipped. Several paths in one
  mapping: the first is taken. Request bodies are taken by the type name of `@RequestBody`, without fields.
- **OpenAPI.** `callbacks` and `links` are not read; a `$ref` to another file is followed only inside the clone, and a
  `$ref` to a URL is not followed.
- **AsyncAPI.** A message with `oneOf` takes the first variant. Servers with a protocol other than Kafka are reported,
  and their channels are still read as Kafka topics.
- **Schema files.** Without a mapping in [[ui:service.kafkaTopics]], the topic is guessed from the file name. Such rows
  carry the mark "[[ui:registry.topicGuessed]]": check the topic or add the mapping on the service tab.
- **Protobuf.** An `import` that cannot be found and unknown types are reported; `google/protobuf/*` imports are known.
- **SQL.** Statements that cannot be parsed are reported; Liquibase YAML and JSON changelogs are not supported yet.
