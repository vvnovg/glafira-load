# Project and services

A load project describes the system under test: its services, their contracts and the stands. This chapter covers the
[[ui:sidebar.project]] and service tabs.

## Project folder {#folder}

[[ui:sidebar.newProject]] creates the project in an empty folder. Glafira does not run `git init`: put the folder into
an existing repository or initialize one yourself. The folder holds:

- `glafira-load.yaml` — the manifest: the project name, services and stands;
- `registry/operations.yaml` — the operation registry ([Operation registry](registry.md));
- `registry/scans/<date-time>.yaml` — a snapshot of every scan, for the diff;
- `registry/tables/<service>.yaml` — tables and columns found in the migrations of a service;
- `load/prod-load.yaml` — the production-load figures ([Production load](prod-load.md));
- `load/imports/<date-time>.xlsx` — a copy of every imported template;
- `load/profile.yaml` — coefficients and load profiles ([Load profile](profile.md));
- `.gitignore` with `runs/` — run results of the next stages stay out of git.

Commit all of these files: they are plain YAML, sorted so that `git diff` shows only real changes. Every file is
written atomically: a crash cannot leave half a file. The list of open projects is kept in the app settings, not in the
project. A project whose folder has gone stays grey in the left panel; [[ui:sidebar.remove]] takes it off the list.

[[ui:project.openFolder]] shows the folder in the file manager.

## Services {#services}

A service is a local git clone of one component of the system under test. Click [[ui:project.addService]] on the
project tab, enter the [[ui:project.serviceId]] (lowercase letters, digits and hyphens, unique in the project) and the
[[ui:project.serviceName]], choose the clone folder and click [[ui:common.save]]. A folder inside the project is written
to the manifest relative to it, any other folder with its absolute path; you can edit the manifest by hand to write,
for example, `../orders-service`.

The service tab shows the version of the clone: `git describe --tags --always --dirty` and the commit. The version
goes into the registry, so you can see in which version an operation appeared. If `git` is not installed or the folder
is not a repository, the version is `unversioned` and a warning is shown; the scan still works.

**Contracts.** If the manifest does not list contracts for the service, Glafira finds them in the clone by masks and
skips `.git`, `node_modules`, `target`, `build`, `dist`, `vendor`, `.idea`, `.gradle` and files ignored by the
`.gitignore` files of the clone (the `.gitignore` of folders above the clone and your global git excludes do not
apply):

| Kind | Masks |
|---|---|
| `openapi` | `**/openapi*.{yaml,yml,json}`, `**/swagger*.{yaml,yml,json}`, `**/api-docs*.json` |
| `proto` | `**/*.proto` (import paths are the folders of the found files) |
| `asyncapi` | `**/asyncapi*.{yaml,yml,json}` |
| `schema` | `**/*.avsc`, `**/*.schema.json` |
| `sql` | `**/db/migration/**/*.sql`, `**/migrations/**/*.sql`, `**/db/changelog/**/*.{xml,yaml,yml,sql}`; rollback migrations are skipped |
| `spring` | `**/src/main/java/**/*.java`, `**/src/main/kotlin/**/*.kt`, only when the clone has `pom.xml` or `build.gradle*` |

Switching a found contract off writes the whole list of contracts into the manifest, with `enabled: false` for that
one: from then on the list is fixed, and new files are added with [[ui:service.addContract]] or found again with
[[ui:service.discover]]. A contract path is relative to the clone and cannot point outside it.

**Kafka topics.** In the [[ui:service.kafkaTopics]] section you map a topic to its schema file (`.avsc`, JSON Schema).
Without a mapping, the topic of a schema file is guessed from its file name, and the registry marks such operations.

**Scan.** [[ui:service.scan]] parses the contracts of the service in the background with progress by file; it can be
cancelled, and a cancelled scan writes nothing. [[ui:project.scanAll]] on the project tab scans every service in turn.
The report shows how many operations were found, how many are new, changed, removed and restored, and the warnings and
errors by file and line; [[ui:service.openFile]] opens the file in the app your system uses for it. An error in one contract does not stop
the others: the service gets the operations of the files that parsed. If contracts were found but none of them parsed,
the registry is left unchanged (otherwise every operation of the service would be marked removed) and the report says
so. A contract file larger than 32 MiB is reported as an error and skipped. When [[ui:project.scanAll]] fails on one
service, the reports of the other services are kept. One scan runs in a project at a time.

## Stands {#environments}

A stand is a test environment where the load will run: an [[ui:project.envId]], a [[ui:project.envKind]] (`dev`, `test`,
`stage` or `prod`) and addresses: [[ui:project.env.http]], [[ui:project.env.kafka]],
[[ui:project.env.grpc]] and [[ui:project.env.sql]]. The kind is shown as a badge next to its selector, and `prod` is
red so that a production stand is not mistaken for a test one; in version 0.1.0 the badge is shown only on the project
tab. Write addresses without passwords: the manifest is committed to git.
Secrets come with the runner in stage L2; in this version stands are only described.

## Manifest problems {#problems}

If the manifest breaks the rules (an empty name, a bad or repeated service or stand ID, a service without a folder, an
unsupported format), the project opens read-only: a banner on the project tab lists the problems, and saving, scanning,
import and manual edits are disabled. A manifest that is not valid YAML is reported with the file and the error. Fix `glafira-load.yaml` in an editor and open the project again. Unknown fields in the manifest
are kept when Glafira writes it, so the fields of later versions and your own notes are not lost.
