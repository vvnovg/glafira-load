# Suites

A suite is the script of a load test: scenarios of steps that call the operations of the registry with the data,
checks and extracted values you describe. Suites live in `suites/<name>/suite.yaml` and are edited by hand in a text
editor; Glafira shows them, checks them, computes the coverage of the load profile and creates a blank suite from the
registry. Suites appear in the left panel under [[ui:sidebar.suites]], one node per folder.

## File suites/name/suite.yaml {#file}

```yaml
format: 1
name: orders
secrets: [API_TOKEN]                      # must exist on the agents as GLAFIRA_LOAD_SECRET_API_TOKEN
data:
  users: { path: data/users.csv, mode: slice }   # slice — agents share the rows; shared — every agent gets the file
  skus:  data/skus.jsonl                         # short form, mode: shared
defaults:
  http: { timeout_ms: 5000, headers: { Accept: application/json } }
  kafka: { acks: all }                           # rdkafka settings of the producer and the consumer
  arrival: poisson                               # poisson | constant
  max_in_flight: 1000                            # per agent
scenarios:
  - id: order-flow
    drives: "http:orders:POST /v1/orders"        # the operation of the first step by default
    on_fail: stop                                # stop | continue
    steps:
      - id: create
        http:
          operation: "http:orders:POST /v1/orders"
          headers: { Authorization: "Bearer {{secret API_TOKEN}}" }
          body: { customerId: "{{users.id}}", items: [{ sku: "{{pick skus.sku}}", qty: "{{int 1 5}}" }] }
        expect: { status: 201, jsonpath: { "$.id": exists } }
        extract: { order_id: "$.id" }
      - id: get
        http: { operation: "http:orders:GET /v1/orders/{id}", path: { id: "{{steps.create.order_id}}" } }
        expect: { status: 200, schema: response, max_ms: 800 }
      - kafka_consume:
          operation: "kafka_consume:orders.created"
          match: { key: "{{steps.create.order_id}}" }
          timeout_ms: 10000
      - kafka_produce:
          operation: "kafka_produce:orders.paid"
          key: "{{steps.create.order_id}}"
          value: { orderId: "{{steps.create.order_id}}", paidAt: "{{now iso}}" }
          encoding: json                              # json | avro
```

The suite is identified by its folder; `name` is `[a-z0-9-]`, up to 64 characters, and should be the same as the
folder (Glafira writes it so). Every step is bound to an operation of the registry by its ID: the method and path of an
HTTP operation and the topic of a Kafka operation come from the registry, the base URL and the Kafka bootstrap from
the stand ([Agents and stands](agents.md#stand)). An unknown field anywhere in the file is an error, not a silent
skip: a typo in a field name would change the load.

Data files are CSV with a header or JSON Lines, `path` relative to the project folder and inside it. Each iteration of
a scenario takes the next row of every data set, round-robin. With `mode: slice` the controller divides the rows
between the agents by their weights, with `shared` every agent gets the whole file.

Defaults: `timeout_ms` 5000, `arrival` `poisson`, `max_in_flight` 1000, `mode` `shared`, `on_fail` `stop`,
`kafka_consume.timeout_ms` 10000, `encoding` `json`, `drives` — the operation of the first step. A step without `id`
is called `step<N>` by its position, starting from 1. Scenario and step IDs, data set and secret names are
`[A-Za-z0-9_-]`, up to 64 characters.

## Steps {#steps}

| Step | Fields |
|---|---|
| `http` | `operation`, `path` (path parameters), `query`, `headers`, `body` (YAML, sent as JSON) or `body_text` (as is), `timeout_ms` |
| `kafka_produce` | `operation`, `key`, `value` (YAML → JSON) or `value_text`, `headers`, `encoding`: `json` or `avro` (the schema of the contract the operation points to, the schema ID from the Schema Registry of the stand) |
| `kafka_consume` | `operation`, `match`: `key` and/or `jsonpath` (a map of JSONPath → expected value), `timeout_ms` |

A scenario drives one operation (`drives`, which must be the operation of one of its steps): its rate is the target
RPS of that operation from the profile divided by the number of steps with that operation in the scenario. The other
operations of the scenario get load as a consequence. Two scenarios driving the same operation in one suite are an
error. `on_fail: continue` lets an iteration run the remaining steps after a failed check; by default the iteration
stops.

## Expressions {#expressions}

`{{…}}` inside any string of the suite. No operators and no nesting. A string that is exactly one expression keeps the
type of the value: a number stays a number in the JSON body.

| Expression | Value |
|---|---|
| `users.id` | the column of the current row of data set `users` |
| `steps.create.order_id` | a value extracted by a previous step of the iteration |
| `uuid` | UUID v4 |
| `int a b`, `float a b` | a random number in `[a, b]` |
| `now iso`, `now unix` | the time of the agent: RFC 3339 in UTC, or seconds |
| `str n` | a random string of `n` Latin letters and digits |
| `pick skus.sku` | the column of a random row of a data set |
| `secret API_TOKEN` | the variable `GLAFIRA_LOAD_SECRET_API_TOKEN` of the agent |
| `env.http`, `env.kafka`, `env.schema_registry` | the addresses of the stand from the manifest |
| `run.id`, `agent.id` | the identifiers of the run and the agent |

An unknown form, data set, column or step is an error of the check, with the file and the line. The names `steps`,
`env`, `run` and `agent` cannot be used for data sets.

## Checks and extraction {#expect}

| Key of `expect` | Meaning |
|---|---|
| `status` | a number, a list of numbers or a class: `2xx`, `4xx` |
| `jsonpath` | JSONPath → matcher: `exists`, `{ equals: … }`, `{ matches: "regex" }`, `{ gt: n }`, `{ lt: n }` |
| `schema: response` | the response matches the compact response schema of the registry: field types and required fields, nesting up to 6 levels |
| `max_ms` | the latency of the step is not above the limit |

A failed check is a step error with the code `EXPECT_STATUS`, `EXPECT_JSONPATH`, `EXPECT_SCHEMA` or `EXPECT_MAX_MS`.
For `kafka_consume` the same checks apply to the value of the matched message. `extract` is a map of name → JSONPath
over the body of the response or the message; a missing path is `EXTRACT_MISSING`. Later steps read the values as
`{{steps.<step id>.<name>}}`.

## Check {#check}

[[ui:suite.check]] on the suite tab, or `glafira-load check` in the terminal, works without the network and reports
problems of the levels [[ui:suite.level.error]] and [[ui:suite.level.warning]] with the file and the line. Clicking the
file and line of a problem, or [[ui:suite.openFile]] in the toolbar, opens the file in the editor assigned to it in the
system. The tab rereads the file on [[ui:suite.check]] and when the window regains focus, so edits made in the editor
show up without reopening the tab.

The check covers the YAML schema (unknown fields, types), the existence of the operations in the registry (a removed
operation is a warning, a missing one an error), the parsing of every expression, the data files and the columns they
are referenced by, the secrets (every `secret X` is listed in `secrets`), the uniqueness of scenario and step IDs,
`steps.<id>` referring only to previous steps, one driving scenario per operation, and `encoding: avro` requiring the
[[ui:project.env.schema_registry]] of the stand (checked when a run starts, because the tab does not know the stand).
Data sets and secrets that are declared but not used are warnings. The run window runs the check before starting and
does not start with errors.

## Profile coverage {#coverage}

The coverage table at the bottom of the tab compares the load profile with the suite: for every operation the
[[ui:suite.column.target]], the [[ui:suite.column.planned]] from all scenarios (the rate of a scenario × the number of
its steps with the operation) and the [[ui:suite.column.delta]]; a delta over 10 % of the target is highlighted.
Active operations with figures but without a step are listed in the "Not covered" line. Coverage is computed from the
saved files with the profile `nominal`, or the first profile by name when there is no `nominal`; targets come from the
[Load profile](profile.md#targets).

## Creating a suite from the registry {#create}

The "+" button ([[ui:sidebar.createSuite]]) next to [[ui:sidebar.suites]] in the left panel, or [[ui:suite.create]] in the toolbar of a suite tab,
opens a dialog: enter the [[ui:suite.dialog.name]], tick the [[ui:suite.dialog.operations]] (the active operations of
the registry, with a search field) and click [[ui:suite.dialog.create]]. Glafira writes `suites/<name>/suite.yaml` with
one scenario per operation and opens it. The body of an HTTP step and the value of a Kafka message are generated from
the request schema of the registry: `integer` → `{{int 1 100}}`, `number` → `{{float 0 1}}`, `boolean` → `true`, an
enumeration → its first value, a string → `{{uuid}}` for fields named like `id`, `…_id` or `…Id`, `{{now iso}}` for
fields named like `…_at`, `…At`, `date` or `time`, otherwise `{{str 8}}`. Path parameters get `{{str 6}}`, required
query parameters and headers a value of their type. `expect.status` is the 2xx code of the operation when the registry
knows it, otherwise the class `2xx`. A `kafka_produce` step gets the key `{{uuid}}`, a `kafka_consume` step no `match`.
gRPC and SQL operations cannot be used in a suite in this version.

The file is a starting point: open it in the editor and replace the generated values with data sets and extracted
values.

## Check in the terminal {#cli}

```bash
glafira-load check <project> [--suite <name>]
```

Without `--suite` all suites of the project are checked. Exit code 0 when there are no errors (warnings are printed
but do not fail), 4 when there are errors, 3 when the project cannot be read or has no suites. Run it in CI next to
the tests of the services; the installation of the CLI is described in [Installing an agent](agents.md#install).
