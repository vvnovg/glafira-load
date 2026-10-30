# Glafira for Load Testing — этап L1 «Каркас и реестр операций»

**Дата:** 2026-10-31
**Статус:** согласована в обсуждении, на ревью текста
**Концепция:** `2026-10-31-glafira-load-concept-design.md`
**Образцы:** Glafira for Apache Kafka (`~/projects/glafira-kafka`), Glafira for PostgreSQL
(`~/projects/glafira-postgres`, спека P1 как образец заимствования модулей)

---

## 1. Объём

В L1 входит:

1. Репозиторий `~/projects/last-test` и Tauri-каркас, скопированный из Glafira for Kafka без Kafka-кода (§2).
2. Хранилище настроек, политика администратора, i18n (`ru`, `en`), «Настройки», «О программе», «Справка».
3. Проект НТ как папка в git: манифест, сервисы СУТ, стенды (§3).
4. Версия клона сервиса через системный `git` (§4).
5. Поиск контрактов в клонах и импортёры: OpenAPI/Swagger, Protobuf, AsyncAPI, схемы сообщений, SQL-миграции,
   сканер аннотаций Spring (§5).
6. Реестр операций: модель, слияние при повторном разборе, снимки, дифф (§6).
7. Excel-шаблон нагрузки ПРОМ: выгрузка и импорт с предпросмотром (§7).
8. Профиль нагрузки: коэффициенты, фазы, целевые RPS, график фаз (§8).
9. Экраны: Проект, Сервис, Реестр операций, Нагрузка ПРОМ, Профиль нагрузки (§9).
10. IPC-контракт и ошибки (§10, §11), иконка и идентификаторы продукта (§2.1), тесты и CI (§13), документация (§14).

Не входит: раннер и формат сьюта (L2), результаты прогонов (L3), моки (L4), MCP (L5), секреты стендов (L2;
в L1 у стендов только адреса без паролей), Schema Registry по URL (L2), разбор кода кроме Spring (L5).

---

## 2. Что берём из Glafira for Kafka

| Модуль Glafira for Kafka | В L1 | Изменения |
|---|---|---|
| `src-tauri/src/{product, error, logging, settings, state, i18n, sync}.rs`, `policy/`, `storage/fs.rs` | Да | Идентификаторы продукта (§2.1); новые коды ошибок (§11); ключи политики `language`, `updatesEnabled`, `updateUrl` |
| `storage/{connections, tree, exchange}.rs`, `secrets.rs` | Нет | Подключений в L1 нет; список открытых проектов хранится в `settings` (§3.4) |
| `kafka/`, `browse/`, `consume/`, `produce/`, `tls/`, `mcp/`, `guard.rs`, `health/`, `migrate.rs`, `dock_name.rs` | Нет | MCP — L5 |
| `commands/{app, settings}.rs` | Да | `check_update` / `install_update` остаются, обновления включаются в L7 |
| `src/components/{Layout, Sidebar, TabContent}` | Да | Новые виды вкладок (§9) |
| `src/components/common/{ErrorBanner, Hint, EnvBadge, ConfirmDialog}` | Да | `ConfigTable` не нужен |
| `src/components/{settings, about, help}` | Да | Без раздела MCP и уведомления об обновлении |
| `src/lib/*`, `src/state/workspace.ts`, `src/api/client.ts`, `src/i18n/*`, `src/guide/*`, `src/styles.css` | Да | Новые `TabSpec`; языки `en`, `ru` |
| `scripts/{i18n-check, guide-publish, third-party, changelog, check-dist-compat}.mjs` | Да | Обязательные языки `en`, `ru` |
| `.github/workflows/ci.yml`, `deny.toml`, `LICENSE`, `NOTICE`, `TRADEMARK.md`, `SECURITY.md`, `CONTRIBUTING.md` | Да | Имена продукта |

Скопированные модули сохраняют имена файлов и публичные функции, чтобы позже вынести их в общий код механически.
Изменение делается параметром, а не правкой внутри.

### 2.1 Идентификаторы продукта (`product.rs`, `tauri.conf.json`)

| Константа | Значение |
|---|---|
| `PRODUCT_NAME` | `Glafira for Load Testing` |
| `IDENTIFIER` | `io.github.vvnovg.glafira.load` |
| `mainBinaryName` | `glafira-load-testing` |
| `KEYCHAIN_SERVICE` | `glafira-load` (используется с L2) |
| `POLICY_VENDOR` / `POLICY_PRODUCT` | `Glafira` / `LoadTesting` |
| Окно | `Glafira for Load Testing`, 1400×900, минимум 900×600 |

Иконка — `public/app-icon.svg` (спидометр и пульс), набор для платформ — `pnpm tauri icon public/app-icon.svg`.

---

## 3. Проект НТ

### 3.1 Папка

```
<проект>/
  glafira-load.yaml
  registry/operations.yaml
  registry/scans/<ГГГГ-ММ-ДД-ЧЧММСС>.yaml
  load/prod-load.yaml
  load/imports/<ГГГГ-ММ-ДД-ЧЧММСС>.xlsx
  load/profile.yaml
  .gitignore                      создаётся с `runs/`
```

Приложение создаёт папку командой «Создать проект» (манифест, пустой реестр, `.gitignore`) и не делает
`git init`: проект кладут в существующий репозиторий или инициализируют его сами. Все записи атомарны: файл
пишется во временный рядом и переименовывается. Пути в манифесте — относительно папки проекта, если лежат внутри
неё, иначе абсолютные.

### 3.2 Манифест `glafira-load.yaml`

```yaml
format: 1
name: orders-platform
services:
  - id: orders                      # [a-z0-9-]+, уникален в проекте
    name: Orders Service
    path: ../orders-service         # клон git
    contracts:                      # пусто — автопоиск по маскам §5.1
      - kind: openapi
        path: api/openapi.yaml
      - kind: proto
        path: proto
        include: [proto, third_party]
      - kind: spring
        path: src/main
    kafka:                          # необязательно: сопоставление имён файлов схем топикам
      topics:
        orders.created: schemas/orders-created.avsc
environments:
  - id: test
    kind: test                      # dev | test | stage | prod — как у подключений Glafira
    http: https://orders.test.local
    kafka: kafka-1.test.local:9092,kafka-2.test.local:9092
    grpc: orders.test.local:9090
    sql: postgres://app@pg.test.local:5432/orders   # без пароля; пароль — L2
```

Неизвестные поля сохраняются при перезаписи (serde `flatten` в `extra`), чтобы будущие этапы и ручные правки
не терялись. Ошибка чтения — `PROJECT_INVALID` с путём и строкой; проект открывается в режиме «только чтение»
с баннером.

### 3.3 Проверки

- `services[].id` и `environments[].id` уникальны; `path` существует и является каталогом; `contracts[].path`
  внутри клона (`PATH_OUTSIDE_PROJECT` иначе).
- `environments[].kind: prod` показывается с красным бейджем как в семействе; в L1 действий со стендами нет.

### 3.4 Открытые проекты

Список путей открытых проектов и последний активный — в `settings.json` приложения (`projects.recent`,
`projects.active`), не в проекте. Пропавший путь показывается в дереве серым с кнопкой «Убрать».

---

## 4. Версия сервиса

`git -C <клон> describe --tags --always --dirty` через `std::process::Command` с таймаутом 5 с. Нет `git` в
`PATH` или папка не репозиторий — `GIT_UNAVAILABLE`, версия `unversioned`, предупреждение на вкладке сервиса,
разбор продолжается. Версия пишется в снимок разбора и в поля `first_seen`/`last_seen` операций. Вдобавок
хранится `git rev-parse HEAD` как `commit`.

---

## 5. Контракты и импортёры

### 5.1 Поиск

Если `contracts` сервиса пуст, приложение ищет по маскам, пропуская `node_modules`, `target`, `build`, `dist`,
`.git`, `vendor`:

| kind | Маски |
|---|---|
| `openapi` | `**/openapi*.{yaml,yml,json}`, `**/swagger*.{yaml,yml,json}`, `**/api-docs*.json` |
| `proto` | `**/*.proto` (include — родители найденных файлов) |
| `asyncapi` | `**/asyncapi*.{yaml,yml,json}` |
| `schema` | `**/*.avsc`, `**/*.schema.json` |
| `sql` | `**/db/migration/**/*.sql`, `**/migrations/**/*.sql`, `**/db/changelog/**/*.{xml,yaml,yml,sql}` |
| `spring` | `**/src/main/{java,kotlin}/**/*.{java,kt}` при наличии `pom.xml` или `build.gradle*` |

Найденные файлы показываются на вкладке сервиса с переключателями; выключенные запоминаются в манифесте как
`contracts[].enabled: false`.

### 5.2 Крейт `glafira-load-contracts`

Чистые функции `parse_<kind>(input: &ContractInput) -> ParseResult`, где `ContractInput` — байты файла, его
путь и функция чтения соседних файлов для `$ref` и `import` (замыкание, которое проверяет границы клона).
`ParseResult { operations: Vec<OperationDraft>, tables: Vec<Table>, warnings: Vec<Diagnostic> }`.
`Diagnostic { path, line, message }`. Крейт не ходит в сеть и не читает диск сам.

| Импортёр | Что даёт | Ограничения L1 |
|---|---|---|
| OpenAPI 3.0/3.1, Swagger 2.0 | Операция на пару путь + метод; `operationId`, `summary`, параметры пути/запроса/заголовков, тело запроса и ответы по кодам в сжатую схему; `$ref` на файлы рядом | Без `callbacks`, `links`, внешних URL |
| Protobuf 3 | `grpc` на каждый `rpc`; сообщения в сжатую схему; стриминг помечается `streaming: client\|server\|bidi` | Разбор через `protobuf-parse`; proto2 — предупреждение |
| AsyncAPI 2.x/3.x | `kafka_produce` для `publish`/`send`, `kafka_consume` для `subscribe`/`receive`; схема сообщения; имя топика из канала | Только привязка `kafka`; остальные протоколы — предупреждение |
| Схемы (`.avsc`, JSON Schema, proto для Kafka) | Сжатая схема сообщения; топик из `kafka.topics` манифеста, иначе из имени файла без расширения с пометкой `topic_guessed: true` | |
| SQL-миграции (Flyway, Liquibase, golang-migrate) | Таблицы и колонки (`CREATE TABLE`, `ALTER TABLE ADD`) в справочник `tables`; операций не создаёт | Диалект PostgreSQL и MySQL через `sqlparser`; нераспознанные выражения — предупреждение |
| Spring MVC/WebFlux | `http` по `@RequestMapping`, `@GetMapping` и остальным, с путём класса; тело из типа параметра `@RequestBody` по имени, без разбора полей; `kafka_consume` по `@KafkaListener(topics)`; `kafka_produce` по `kafkaTemplate.send("topic"` с литералом | Текстовые правила без компиляции; пути через константы — предупреждение «путь не вычислен» с файлом и строкой |

**Сжатая схема** (`SchemaSummary`): дерево `{ name, type, required, enum?, example?, children? }` глубиной до 6,
массивы — `type: array` с `items`. Поля сверх 200 на уровне обрезаются с пометкой `truncated`. Из неё считается
`fingerprint` (SHA-256 нормализованного JSON без примеров и описаний).

### 5.3 Разбор в приложении (`scan/`)

«Разобрать» на сервисе или «Разобрать все» на проекте: в фоне, с прогрессом по файлам через `Channel`, с отменой.
Итог: `ScanReport { service, version, commit, operations_found, warnings, errors, merged: MergeSummary }`.
Отчёт показывается на вкладке сервиса; ошибки — по файлу и строке с кнопкой «Открыть файл» (системный opener).

---

## 6. Реестр операций

### 6.1 Модель (`registry/operations.yaml`)

```yaml
format: 1
operations:
  - id: "http:orders:POST /v1/orders"
    service: orders
    kind: http                       # http | kafka_produce | kafka_consume | grpc | sql
    name: createOrder
    summary: Создание заказа
    address:                         # зависит от kind
      method: POST
      path: /v1/orders
    request: { ...SchemaSummary... }
    response: { ...SchemaSummary... }   # для http — по коду 2xx; остальные коды в responses
    source: { kind: openapi, path: api/openapi.yaml, line: 120 }
    fingerprint: "sha256:…"
    status: active                   # active | removed
    first_seen: { version: v1.4.0, commit: abc123, at: 2026-10-31T10:00:00Z }
    last_seen:  { version: v1.6.2, commit: def456, at: 2026-11-10T09:30:00Z }
    manual:                          # переживает переразбор
      tags: [critical]
      notes: "Идёт через API-gateway, лимит 100 rps"
      owner: team-orders
```

Правила `id`: `http:<service>:<METHOD> <path>` (путь как в контракте, с `{param}`), `kafka_produce:<topic>`,
`kafka_consume:<topic>`, `grpc:<service>:<package.Service/Method>`, `sql:<service>:<name>` (имя задаёт человек).
Операции `source.kind: manual` добавляются формой и не трогаются слиянием.

### 6.2 Слияние

Вход: текущий реестр и `Vec<OperationDraft>` одного сервиса. Чистая функция в `glafira-load-model`:

1. Черновик с `id`, которого нет, — новая операция `active`, `first_seen = last_seen = версия`.
2. `id` есть: `last_seen` обновляется; если `fingerprint` изменился — в `MergeSummary.changed` с диффом сжатых
   схем (добавленные, удалённые, изменившие тип поля); `request`, `response`, `address`, `source` заменяются,
   `manual` сохраняется.
3. Операции сервиса со статусом `active` и `source.kind != manual`, не пришедшие в черновиках, — `removed`,
   `last_seen` не меняется. Удалить окончательно можно вручную. Появившаяся снова `removed` операция
   возвращается в `active`.
4. Порядок в файле — по `service`, затем `id`, чтобы дифф в git был читаемым.

Слияние идемпотентно: повторный разбор без изменений даёт пустой `MergeSummary` и файл байт в байт.

### 6.3 Снимки и дифф

Каждый разбор пишет `registry/scans/<метка>.yaml` с версией, коммитом, `MergeSummary` и списком `id` с
`fingerprint`. Вкладка реестра показывает «Дифф с прошлым разбором» (последние два снимка) и позволяет выбрать
любые два снимка. Пометка затронутых сценариев — L6.

---

## 7. Нагрузка ПРОМ

### 7.1 Шаблон (`glafira-load-xlsx`, `rust_xlsxwriter`)

Файл `prod-load-<проект>-<ГГГГ-ММ-ДД>.xlsx`, язык листов — язык интерфейса на момент выгрузки.

**Лист «Операции».** По строке на операцию `active`. Колонки A–E защищены (лист защищён, разрешено только
редактирование незащищённых ячеек; пароль защиты пустой, цель — уберечь от случайной правки, не от умысла):

| Колонка | Содержимое | Правило |
|---|---|---|
| A `ID` | `id` операции | защищена, серый фон |
| B `Сервис`, C `Тип`, D `Операция`, E `Описание` | из реестра | защищены |
| F `Среднее RPS` | число ≥ 0 | для `kafka_consume` — «сообщений/с» |
| G `Пиковое RPS` | число ≥ F | |
| H `Доля ошибок, %` | 0–100 | |
| I `Размер запроса, байт` | целое ≥ 0 | для `kafka_consume` — размер сообщения |
| J `Размер ответа, байт` | целое ≥ 0 | для Kafka — пусто и защищена |
| K `p95 ПРОМ, мс` | число ≥ 0 | для `kafka_consume` — «допустимый лаг, с» |
| L `Комментарий` | текст | |

Проверка данных Excel на F–K (число, границы, подсказка в ячейке), заголовки с комментариями-подсказками,
закреплённая первая строка, автофильтр.

**Лист «Параметры».** A1 — `glafira-load:prod-load:1:<project.name>` (маркер формата и проекта). Поля:
стенд ПРОМ (имя), период наблюдения с/по, источник цифр (система мониторинга), кто заполнил, дата.

**Лист «Справка».** Как заполнять, что значат колонки, что ID менять нельзя; текст из i18n.

### 7.2 Импорт (`calamine`)

1. Читается A1 листа «Параметры»: нет маркера или другой проект — `TEMPLATE_MISMATCH` (ожидаемый/найденный).
2. Строки листа «Операции» сопоставляются по `ID`. Результат `ImportPreview`:
   - `filled` — строки с числами в F или G;
   - `empty` — строки без цифр (пропускаются);
   - `unknown_ids` — `ID` не из реестра (строки из старого шаблона), пропускаются с предупреждением;
   - `invalid` — нечисловое значение, отрицательное, `G < F`, `H > 100`: номер строки, колонка, значение;
     при наличии хотя бы одной строки — `TEMPLATE_INVALID`, кнопка «Применить» недоступна;
   - `diff` — по каждой операции старые и новые значения против текущего `load/prod-load.yaml`;
   - `params` — поля листа «Параметры».
3. «Применить»: пишет `load/prod-load.yaml`, копирует файл в `load/imports/<метка>.xlsx`.

### 7.3 Файл `load/prod-load.yaml`

```yaml
format: 1
source: { stand: prod-msk, period: { from: 2026-10-01, to: 2026-10-14 }, system: Grafana, filled_by: Иванов, imported_at: 2026-10-31T12:00:00Z, file: imports/2026-10-31-120000.xlsx }
operations:
  "http:orders:POST /v1/orders": { rps_avg: 120, rps_peak: 340, error_pct: 0.2, req_bytes: 850, resp_bytes: 420, p95_ms: 180, comment: "" }
  "kafka_consume:orders.created": { rate_avg: 110, rate_peak: 300, msg_bytes: 1200, lag_max_s: 5 }
```

---

## 8. Профиль нагрузки

### 8.1 Файл `load/profile.yaml`

```yaml
format: 1
coefficients:
  k_env: 0.5                       # мощность стенда к ПРОМ
  k_growth: 1.3                    # запас роста
  overrides:
    services: { billing: { k_env: 0.3 } }
    operations: { "http:orders:POST /v1/orders": { k_growth: 2.0 } }
profiles:
  nominal:
    basis: avg                     # avg | peak
    phases:
      - { kind: ramp, minutes: 10, to_pct: 100 }
      - { kind: steady, minutes: 60 }
      - { kind: ramp, minutes: 5, to_pct: 0 }
  peak:
    basis: peak
    phases: [ { kind: ramp, minutes: 5, to_pct: 100 }, { kind: steady, minutes: 30 } ]
  stress:
    basis: peak
    phases: [ { kind: steps, step_pct: 25, step_minutes: 10, max_pct: 300 } ]
```

### 8.2 Расчёт (`glafira-load-model`)

- `target(op) = basis(op) × k_env(op) × k_growth(op)`, где коэффициент операции = переопределение операции,
  иначе сервиса, иначе общий. Для `kafka_consume` основа — `rate_*`.
- `share(op) = target(op) / Σ target` по операциям с цифрами. Операции `active` без цифр — список
  «не покрыто», в профиль не входят; `removed` не участвуют.
- `timeline(profile)` — ряд точек (минута, % от цели) для графика и для раннера: `ramp` — линейно от
  предыдущего значения к `to_pct`, `steady` — удержание, `steps` — ступени от `step_pct` до `max_pct`.
- Свойства, проверяемые тестами: сумма долей равна 1 (с точностью 1e-9) при непустом списке; увеличение любого
  коэффициента не уменьшает ни одной цели; `timeline` непрерывна в начале каждой фазы.

### 8.3 Вкладка

Коэффициенты формой, переопределения таблицей, выбор профиля, фазы списком с добавлением и перестановкой,
таблица «операция — основа — цель RPS — доля» с итогом, список «не покрыто», график `timeline` на uPlot
(ось X — минуты, ось Y — % и абсолютный суммарный RPS второй осью). Сохранение по кнопке с диффом файла.

---

## 9. Экраны

Дерево слева (как в семействе): Проекты НТ → проект → Сервисы (по одному узлу на сервис), Реестр операций,
Нагрузка ПРОМ, Профиль нагрузки, Стенды. Вкладки справа по `TabSpec`:

| Вкладка | Содержимое |
|---|---|
| Проект | Манифест формой: имя, путь, сервисы (id, имя, путь с выбором папки через `plugin-dialog`), стенды (id, вид с бейджем, адреса). «Создать проект», «Открыть проект», «Открыть папку». «Разобрать все» с прогрессом |
| Сервис | Путь, версия и коммит, предупреждение `GIT_UNAVAILABLE`; найденные контракты с переключателями и видом; «Разобрать»; последний `ScanReport`: найдено, новых, изменённых, удалённых, предупреждения и ошибки по файлу и строке с «Открыть файл» |
| Реестр операций | Таблица с виртуализацией: id, сервис, вид, имя, источник, статус, последняя версия; фильтры по сервису, виду, источнику, статусу; поиск по id и имени; панель справа: схемы запроса и ответа деревом, адрес, источник с «Открыть файл», ручные поля с сохранением; переключатель «Дифф с прошлым разбором» (новые зелёным, удалённые серым, изменённые жёлтым с диффом полей); «Добавить вручную» (форма для `sql` и любых других) |
| Нагрузка ПРОМ | «Выгрузить шаблон» (диалог сохранения), «Импортировать» (диалог открытия → предпросмотр §7.2 → «Применить»), таблица текущих цифр с датой импорта и источником, список незаполненных операций |
| Профиль нагрузки | §8.3 |
| Настройки, О программе, Справка | Как в семействе; разделы MCP и обновлений появятся в L5/L7 |

Подсказки «?» на коэффициентах, колонках шаблона, полях стендов. Всё с клавиатуры; фокус-ловушки в диалогах.

---

## 10. IPC-контракт

| Команда | Параметры | Ответ |
|---|---|---|
| `project_create` | `path, name` | `Project` |
| `project_open` / `project_close` | `path` | `Project` / — |
| `project_save` | `Project` | `Project` (после нормализации) |
| `project_recent` | — | `[{ path, name, missing }]` |
| `service_discover_contracts` | `project, service_id` | `[ContractRef]` |
| `service_scan` | `project, service_id?` (все, если пусто) | `Channel<ScanProgress>` → `ScanReport[]` |
| `scan_cancel` | `scan_id` | — |
| `registry_list` | `project, filter` | `[OperationRow]` |
| `registry_get` | `project, id` | `Operation` |
| `registry_update_manual` | `project, id, manual` | `Operation` |
| `registry_add_manual` / `registry_delete` | `project, Operation` / `project, id` | — |
| `registry_scans` / `registry_diff` | `project` / `project, from, to` | `[ScanMeta]` / `RegistryDiff` |
| `prod_load_export_template` | `project, path` | `{ rows }` |
| `prod_load_preview_import` | `project, path` | `ImportPreview` |
| `prod_load_apply_import` | `project, path` | `ProdLoad` |
| `prod_load_get` | `project` | `ProdLoad` |
| `profile_get` / `profile_save` | `project` / `project, Profile` | `Profile` |
| `profile_compute` | `project, profile_name` | `{ targets, shares, uncovered, timeline }` |
| `open_path` | `path` | — (только внутри проекта или клонов) |

Типы в `src/api/types.ts` зеркалят Rust-структуры с `serde(rename_all = "camelCase")`.

---

## 11. Ошибки

`AppError { code, message, details, retryable }`. Коды семейства (`IO_ERROR`, `INVALID_INPUT`, `CANCELLED`,
`POLICY_DENIED`) и новые:

| code | Причина | Поведение UI |
|---|---|---|
| `PROJECT_INVALID` | Манифест или файл проекта не читается или не проходит §3.3 | Баннер с путём, строкой и причиной; проект только для чтения |
| `CONTRACT_PARSE` | Файл контракта не разобран | Строка в отчёте разбора с файлом и строкой; остальные файлы разбираются |
| `GIT_UNAVAILABLE` | Нет `git`, не репозиторий, таймаут | Предупреждение на сервисе, версия `unversioned` |
| `TEMPLATE_MISMATCH` | Маркер A1 другого проекта или формата | Диалог с ожидаемым и найденным |
| `TEMPLATE_INVALID` | Есть строки `invalid` | Предпросмотр со списком, «Применить» недоступна |
| `PATH_OUTSIDE_PROJECT` | Путь выходит за проект или клон | Отказ с путём |
| `SCAN_BUSY` | Разбор сервиса уже идёт | Кнопка заблокирована, показан прогресс |

Частичные результаты — норма: сервис с ошибкой в одном контракте получает операции из остальных.

---

## 12. Структура репозитория

```
last-test/
  package.json, pnpm-lock.yaml, pnpm-workspace.yaml, vite.config.ts, vitest.config.ts, tsconfig*.json, index.html
  public/app-icon.svg
  src/
    api/{client.ts, types.ts}
    components/{Layout, Sidebar, TabContent}.tsx
    components/{project, services, registry, load, profile, settings, about, help, common}/
    lib/ state/workspace.ts i18n/ locales/{en,ru}.json guide/
  src-tauri/
    src/  main.rs lib.rs product.rs error.rs logging.rs settings.rs state.rs i18n.rs sync.rs
          policy/ storage/fs.rs project/{manifest, paths, recent}.rs git/version.rs
          scan/{discover, run, progress}.rs load/{template, import, profile}.rs commands/
    tauri.conf.json, capabilities/, icons/
  crates/
    glafira-load-model/      model/{project, registry, prod_load, profile}.rs merge.rs diff.rs schema.rs
    glafira-load-contracts/  openapi.rs proto.rs asyncapi.rs schema.rs sql.rs spring.rs summary.rs
    glafira-load-xlsx/       export.rs import.rs
  dev/fixtures/              orders-openapi/ orders-proto/ orders-asyncapi/ schemas/ flyway/ liquibase/ spring-java/ spring-kotlin/
  docs/                      admin.md guide/{en,ru}/ superpowers/{specs,plans}/
  scripts/                   i18n-check.mjs guide-publish.mjs third-party.mjs changelog.mjs check-dist-compat.mjs
  .github/workflows/ci.yml   deny.toml LICENSE NOTICE TRADEMARK.md SECURITY.md CONTRIBUTING.md README.md README.ru.md
```

Корневой `Cargo` workspace: `src-tauri` и `crates/*`. Крейты `glafira-load-*` не зависят от `tauri`.

---

## 13. Тестирование

### 13.1 Автотесты

- `glafira-load-contracts`: по фикстуре на импортёр и на каждое ограничение (`$ref` на соседний файл, proto с
  `import`, AsyncAPI 2 и 3, Avro с вложенными записями, Flyway и Liquibase, Spring на Java и Kotlin, путь через
  константу → предупреждение). Снимки ожидаемых `OperationDraft` в `tests/snapshots/` (`insta`).
- `glafira-load-model`: слияние (новая, изменённая, удалённая, вернувшаяся, ручная), идемпотентность, порядок;
  дифф снимков; расчёт профиля и его свойства (§8.2); чтение манифеста с неизвестными полями без потерь.
- `glafira-load-xlsx`: выгрузка → программное заполнение (`umya-spreadsheet` или правка XML в тесте) → импорт;
  маркер другого проекта; строки `invalid`, `unknown_ids`, `empty`.
- `src-tauri`: команды проекта на временной папке; `git/version.rs` с поддельным `git` в `PATH`
  (`examples/fake_git.rs`, как `fake_ssh` в PostgreSQL); атомарная запись; `PATH_OUTSIDE_PROJECT`.
- Фронтенд (vitest + Testing Library): форма проекта, вкладка сервиса с отчётом, таблица реестра с фильтрами и
  диффом, предпросмотр импорта с блокировкой «Применить», вкладка профиля и расчёт итога; `i18n-check`.

### 13.2 CI

`cargo test --workspace`, `cargo deny check`, `pnpm test`, `pnpm i18n:check`, `pnpm build`, `node --test scripts`,
сборка установщиков на macOS (arm64, x86_64), Windows (msi), Linux (deb, rpm, AppImage), как в семействе.

---

## 14. Документация

`README.md` (en) и `README.ru.md`; `docs/admin.md` (политика, где лежат данные); руководство в приложении
`docs/guide/{en,ru}/`: «Начало работы», «Проект и сервисы», «Реестр операций», «Нагрузка ПРОМ», «Профиль
нагрузки». `CHANGELOG.md` с `0.1.0`.

---

## 15. Критерии готовности

1. Создать проект, добавить сервис из `dev/fixtures`, разобрать контракты всех шести видов, увидеть операции со
   схемами и ссылками на источник.
2. Изменить фикстуру, разобрать снова, увидеть дифф (новая, изменённая, удалённая) и снимки.
3. Выгрузить шаблон, заполнить в Excel/LibreOffice, импортировать: предпросмотр показывает заполненные, пустые,
   неизвестные и ошибочные строки; после исправления «Применить» пишет `prod-load.yaml` и копию xlsx.
4. Задать коэффициенты и фазы, получить цели, доли, «не покрыто» и график; `profile.yaml` в git читаем.
5. Всё с клавиатуры, на `ru` и `en`; CI зелёный, установщики трёх ОС собираются.
