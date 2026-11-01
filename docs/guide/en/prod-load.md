# Production load

The load profile starts from real figures: how many requests per second each operation gets in production. Glafira
exports the operations to an Excel template, the team that has access to production monitoring fills it in, and you
import it back. The figures are kept in `load/prod-load.yaml`. Open the tab with [[ui:sidebar.prodLoad]] in the left
panel.

## Template {#template}

Click [[ui:prodLoad.export]] and choose where to save the file; the suggested name is
`prod-load-<project>-<date>.xlsx`. With [[ui:prodLoad.prefill]] turned on, the template already holds the figures of
the last import, so the team only updates them. The template is written in the interface language of the moment.

The template has three sheets. **Operations** has a row for every active operation of the registry. Columns A–E (ID,
service, kind, operation, description) are locked: the sheet is protected without a password, to prevent accidental
edits. Fill in the columns F–L:

| Column | What to enter |
|---|---|
| F, average RPS | a number ≥ 0; required as soon as the row has any figure |
| G, peak RPS | a number not less than the average; empty means the same as the average |
| H, errors, % | from 0 to 100 |
| I, request size, bytes | an integer ≥ 0 |
| J, response size, bytes | an integer ≥ 0; leave empty for Kafka rows |
| K, p95 in production, ms | a number ≥ 0 |
| L, comment | any text |

For `kafka_consume` rows the figures mean something else: F and G are messages per second, I is the message size, and K
is the allowed consumer lag in seconds. Excel checks the numbers as you type them; the column headers have notes with these rules.
In versions 0.1.0 and 0.2.0 column J is not locked for Kafka rows: leave it empty, because a value you enter there is imported as
is.

**Parameters** holds a marker of the project in cell A1 (do not change it) and the fields about the figures: the
production stand, the observation period, the monitoring system they come from and who filled them in. **Help**
repeats these rules for the person who fills the template.

## Import {#import}

Click [[ui:prodLoad.import]] and choose the filled file (up to 32 MiB). Glafira first checks the marker on the Parameters sheet: a
template of another project or format is rejected with the expected and the found marker. Then it shows a preview:

- **filled** — rows with figures, matched to the registry by ID;
- **empty** — rows without figures; these operations keep their previous figures;
- **unknown IDs** — rows whose ID is not in the registry, for example from an older template; they are skipped;
- **invalid** — the row, column, value and problem: not a number, negative, not an integer, a percentage outside
  0–100, a peak below the average, or a missing average.

The preview also shows the values of the Parameters sheet and a diff for every operation: [[ui:prodLoad.before]] and
[[ui:prodLoad.after]]. While there is at least one invalid row, [[ui:prodLoad.apply]] is disabled: fix the file and import
it again. [[ui:prodLoad.apply]] writes `load/prod-load.yaml` and saves a copy of the imported file to
`load/imports/<date-time>.xlsx`, so you can always see where the figures came from.

After the import the tab shows the current figures with the date and the stand of the import, and how many active
operations have no figures yet.

## File load/prod-load.yaml {#file}

```yaml
format: 1
source:
  stand: prod-msk
  period_from: 2026-10-01
  period_to: 2026-10-14
  system: Grafana
  filled_by: Ivanov
  imported_at: 2026-10-31T12:00:00Z
  file: load/imports/2026-10-31-120000.xlsx
operations:
  "http:orders:POST /v1/orders": { avg: 120, peak: 340, error_pct: 0.2, req_bytes: 850, resp_bytes: 420, p95_ms: 180 }
  "kafka_consume:orders.created": { avg: 110, peak: 300, req_bytes: 1200, lag_max_s: 5 }
```

`avg` and `peak` are requests per second (messages per second for `kafka_consume`); `error_pct`, `req_bytes`,
`resp_bytes`, `p95_ms` and `comment` are written when filled in; `lag_max_s` is the allowed lag of `kafka_consume`
operations. You can edit the file by hand; the next import replaces the figures of the rows it fills.
