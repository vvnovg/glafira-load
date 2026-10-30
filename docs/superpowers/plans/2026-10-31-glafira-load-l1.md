# Glafira for Load Testing — L1 «Каркас и реестр операций»: план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Desktop-приложение Glafira for Load Testing, которое открывает проект НТ (папку в git), разбирает контракты сервисов СУТ из локальных клонов в реестр операций с диффом между разборами, выгружает Excel-шаблон нагрузки ПРОМ, импортирует заполненный с предпросмотром и считает профиль нагрузки с графиком фаз.

**Architecture:** Tauri 2. Каркас (политика, настройки, i18n, справка, оболочка с деревом и вкладками) переносится из Glafira for LLM Inference — это те же модули с теми же именами, что в Glafira for Apache Kafka (спека L1 §2), но уже без Kafka, MCP и секретов. Вся логика без I/O — в трёх крейтах workspace: `glafira-load-model` (файлы проекта, реестр, слияние, дифф, нагрузка, профиль), `glafira-load-contracts` (импортёры «байты → операции»), `glafira-load-xlsx` (шаблон ПРОМ). `src-tauri` добавляет чтение и запись папки проекта, вызов `git`, обход клонов, прогресс разбора и IPC. Фронтенд React: дерево проектов и пять вкладок.

**Tech Stack:** Rust stable (edition 2024), Tauri 2, serde + serde_yaml_ng, sha2, globset + ignore, roxmltree, sqlparser, rust_xlsxwriter + calamine, tokio (process); React 19, TypeScript 6, Vite 8, TanStack Query 5 / Virtual 3, Zustand 5, i18next, uPlot 1.6, Vitest + Testing Library.

**Спецификация:** `docs/superpowers/specs/2026-10-31-glafira-load-l1-design.md`. Концепция: `docs/superpowers/specs/2026-10-31-glafira-load-concept-design.md`.

**Образцы кода:** Glafira for LLM Inference в `/Users/vvnovg/projects/glafira-inference-llm` — ниже `$G` (каркас); Glafira for Kubernetes в `/Users/vvnovg/projects/glafira-openshift` — `$O` (обёртка uPlot `src/components/resources/UsageChart.tsx`). Корень нового репозитория — `$P` (`/Users/vvnovg/projects/last-test`). Команды шагов выполняются из `$P`, если не сказано иное.

## Global Constraints

- Платформы: macOS 11+ (arm64, x64), Windows x64, Linux x64. Node 22, pnpm 11, Rust stable.
- Идентификаторы: `PRODUCT_NAME = "Glafira for Load Testing"`, `IDENTIFIER = "io.github.vvnovg.glafira.load"`, бинарь приложения `glafira-load-testing`, раннер `glafira-load` (L2), `KEYCHAIN_SERVICE = "glafira-load"`, `POLICY_VENDOR = "Glafira"`, `POLICY_PRODUCT = "LoadTesting"`. Окно 1400×900, минимум 900×600.
- Файлы проекта: `glafira-load.yaml`, `registry/operations.yaml`, `registry/scans/<ГГГГ-ММ-ДД-ЧЧММСС>.yaml`, `load/prod-load.yaml`, `load/imports/<ГГГГ-ММ-ДД-ЧЧММСС>.xlsx`, `load/profile.yaml`. Все записи атомарны (временный файл + rename). Ключи YAML — snake_case; те же структуры уходят в IPC без переименования.
- Пути в манифесте — относительно папки проекта, если лежат внутри неё, иначе абсолютные. Любой путь контракта проверяется на выход за пределы клона, любой путь записи — за пределы проекта (`PATH_OUTSIDE_PROJECT`).
- Правила `id` операции: `http:<service>:<METHOD> <path>`, `kafka_produce:<topic>`, `kafka_consume:<topic>`, `grpc:<service>:<package.Service/Method>`, `sql:<service>:<name>`. `id` сервиса и стенда — `[a-z0-9-]{1,64}`.
- Сжатая схема: глубина ≤ 6, ≤ 200 полей на уровне (`truncated: true`), отпечаток `sha256:<hex>` без примеров и описаний.
- `git describe --tags --always --dirty` и `git rev-parse HEAD` через системный `git`, таймаут 5 с; без git — `unversioned` и `GIT_UNAVAILABLE` как предупреждение, не ошибка разбора.
- Все команды Tauri возвращают `Result<T, AppError>`; `AppError { code, key, params, message, details?, retryable }` сериализуется в camelCase, `code` — в SCREAMING_SNAKE_CASE. Коды L1: `INVALID_INPUT`, `NOT_FOUND`, `TIMEOUT`, `CANCELLED`, `INTERNAL`, `PROJECT_INVALID`, `CONTRACT_PARSE`, `GIT_UNAVAILABLE`, `TEMPLATE_MISMATCH`, `TEMPLATE_INVALID`, `PATH_OUTSIDE_PROJECT`, `SCAN_BUSY`.
- Пользовательские тексты из Rust — только через `ltext!` / `app_err!` с английским шаблоном; переводы — `src/locales/{en,ru}.json`. Кириллица в пользовательских строках Rust запрещена (`pnpm i18n:check`). Комментарии в коде — по-русски, как во всём семействе.
- Языки L1: `en`, `ru`. Главы руководства: `getting-started`, `project`, `registry`, `prod-load`, `profile`, `updates`.
- Ни одна операция не блокирует UI: файлы и `git` — в `spawn_blocking` или `tokio::process`, разбор — фоновая задача с прогрессом по `Channel` и отменой.
- Нет телеметрии и удалённого контента; CSP — только `self` и IPC. Обновления (`check_update` / `install_update`) остаются в коде с заглушкой ключа `REPLACE_WITH_UPDATER_PUBKEY`, как в `$G`; включаются в L7.
- Сообщения коммитов — по-русски, в стиле `feat(registry): …`, с последней строкой `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Отступления от спецификации (решения плана)

1. **Образец каркаса — `$G` (Glafira for LLM Inference), а не Glafira for Kafka.** Модули те же и с теми же именами (`product`, `error`, `i18n`, `logging`, `policy`, `settings`, `state`, `storage/fs`, `sync`, `dock_name`, оболочка React), но в `$G` уже нет Kafka, MCP и лицензий, а код свежее. Таблица заимствований спеки §2 выполняется.
2. **Свой разбор подмножества proto3** вместо крейта `protobuf-parse`: тот читает файлы с диска сам, а крейт импортёров по спеке §5.2 диск не читает. Подмножество: `syntax`, `package`, `import`, `option`, `message` (вложенные, `repeated`, `optional`, `map<K,V>`, `oneof`, `reserved`), `enum`, `service` с `rpc` и `stream`. Всё остальное — предупреждение с номером строки.
3. **Поля цифр ПРОМ унифицированы:** в `prod-load.yaml` у всех операций `avg` и `peak` (RPS для HTTP/gRPC/SQL/produce, сообщений/с для `kafka_consume`), остальные поля необязательны: `error_pct`, `req_bytes`, `resp_bytes`, `p95_ms`, `lag_max_s`, `comment`. Вид операции известен из реестра, отдельные имена `rps_*`/`rate_*` не нужны.
4. **Шаблон можно выгрузить с текущими цифрами** (`prefill`): команда ПРОМ обновляет, а не заполняет с нуля; это же даёт round-trip-тест «выгрузить с цифрами → импортировать» без правки XML.
5. **Подписи листов и колонок шаблона приходят из фронтенда** (`TemplateLabels` в аргументах команды): словари лежат в `src/locales`, Rust их не читает — как и везде в семействе.
6. **Схемы без AsyncAPI получают направление:** в манифесте `kafka.topics` значение — путь к файлу схемы (направление `produce`) или объект `{ schema, direction: produce | consume }`. Файл без записи в манифесте — `kafka_produce` с топиком из имени файла и `topic_guessed: true`.
7. **Снимки ожидаемых результатов — обычные `assert_eq!`**, без `insta`: меньше инструментов, дифф в тесте читается прямо.
8. **`PROJECT_INVALID` в двух видах:** YAML не читается — команда `project_open` возвращает ошибку (показывать нечего); манифест прочитан, но не проходит проверки §3.3 — проект открывается с `read_only: true` и списком `problems` в баннере.
9. **Типы модели в TypeScript — snake_case** (`first_seen`, `k_env`), потому что это те же структуры, что в файлах проекта, и фронтенд показывает их диффом; DTO приложения (`AppInfo`, `LanguageSettings`, `ScanReport`…) — camelCase, как в семействе.
10. **Liquibase:** XML-ченджлоги (`createTable`, `addColumn`) и formatted SQL разбираются; YAML/JSON-ченджлоги — предупреждение «формат не поддерживается».

---

## Структура файлов L1

```
last-test/
├── Cargo.toml                                  # workspace: src-tauri, crates/*
├── package.json, pnpm-lock.yaml, pnpm-workspace.yaml, index.html
├── vite.config.ts, vitest.config.ts, tsconfig.json, tsconfig.node.json
├── public/app-icon.svg                         # уже в репозитории
├── .github/workflows/ci.yml, deny.toml
├── LICENSE, NOTICE, TRADEMARK.md, SECURITY.md, CONTRIBUTING.md, README.md, README.ru.md, CHANGELOG.md, THIRD_PARTY.md
├── docs/admin.md, docs/admin/{policy.json,policy.reg,policy.mobileconfig}
├── docs/guide/{en,ru}/{getting-started,project,registry,prod-load,profile,updates}.md
├── scripts/{i18n-check,check-dist-compat,third-party,changelog,guide-publish}.mjs (+ .test.mjs)
├── dev/fixtures/
│   ├── orders-openapi/openapi.yaml, orders-openapi/schemas/order.yaml
│   ├── orders-swagger/swagger.json
│   ├── orders-proto/proto/orders/v1/orders.proto, orders-proto/proto/common/money.proto
│   ├── orders-asyncapi/asyncapi-v2.yaml, orders-asyncapi/asyncapi-v3.yaml
│   ├── schemas/orders-created.avsc, schemas/payment.schema.json
│   ├── flyway/db/migration/V1__init.sql, flyway/db/migration/V2__add_column.sql
│   ├── liquibase/db/changelog/changelog.xml, liquibase/db/changelog/002.sql
│   ├── spring-java/pom.xml, spring-java/src/main/java/demo/OrderController.java, .../OrderListener.java
│   └── spring-kotlin/build.gradle.kts, spring-kotlin/src/main/kotlin/demo/PaymentController.kt
├── crates/
│   ├── glafira-load-model/src/{lib,project,schema,registry,merge,diff,prod_load,profile}.rs
│   ├── glafira-load-contracts/src/{lib,summary,openapi,proto,asyncapi,schema,sql,spring}.rs
│   └── glafira-load-xlsx/src/{lib,export,import}.rs
├── src-tauri/
│   ├── Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/
│   └── src/
│       ├── main.rs lib.rs product.rs error.rs i18n.rs logging.rs settings.rs state.rs sync.rs dock_name.rs
│       ├── policy/{mod,sources}.rs storage/{mod,fs}.rs
│       ├── project/{mod,manifest,paths}.rs            # Task 12
│       ├── git/{mod,version}.rs                       # Task 12
│       ├── scan/{mod,discover,run,registry}.rs        # Task 13
│       ├── load/{mod,template,import,profile}.rs      # Task 14
│       └── commands/{mod,app,settings,project,scan,registry,load}.rs
└── src/
    ├── main.tsx App.tsx styles.css polyfills.ts vite-env.d.ts
    ├── api/{client,types}.ts  i18n/{index,languages,text}.ts  locales/{en,ru}.json  guide/{chapters,parse}.ts
    ├── lib/{env,focusTrap,format,links,plainInputs,tabActive,useAppMenu,useDebouncedValue,useHelpShortcut,useLanguageSync}.ts
    ├── state/{workspace,helpContext}.ts  test/{setup.ts,render.tsx}
    └── components/
        ├── Layout.tsx Sidebar.tsx TabContent.tsx
        ├── common/{ErrorBanner,Hint,ExternalLink,ChunkBoundary}.tsx
        ├── settings/SettingsPanel.tsx about/{AboutPanel,ReleaseNotes,UpdateNotice}.tsx help/{HelpPanel.tsx,helpChapterFor.ts}
        ├── projects/ProjectTree.tsx                   # Task 15
        ├── project/ProjectPanel.tsx                   # Task 15
        ├── services/ServicePanel.tsx                  # Task 16
        ├── registry/{RegistryPanel,OperationDetails,SchemaTree,ManualOperationForm}.tsx   # Task 17
        ├── load/{ProdLoadPanel,ImportPreview}.tsx     # Task 18
        └── profile/{ProfilePanel,PhaseEditor,TimelineChart}.tsx                          # Task 19
```

Ответственность крейтов:

| Крейт | Делает | Не делает |
|---|---|---|
| `glafira-load-model` | Типы файлов проекта, чтение и запись YAML из строки, проверка манифеста, `id` операций, сжатая схема и отпечаток, слияние реестра, дифф снимков, предпросмотр импорта, расчёт профиля | Не читает диск, не знает про Tauri |
| `glafira-load-contracts` | Шесть импортёров: байты контракта → `OperationDraft`, таблицы, предупреждения; чтение соседних файлов — через замыкание от вызывающего | Не обходит каталоги, не знает про git |
| `glafira-load-xlsx` | Запись шаблона в файл, чтение заполненного в строки | Не решает, что валидно (это `prod_load::preview_import` в модели) |
| `src-tauri` | Папка проекта, атомарная запись, `git`, обход клонов, прогресс и отмена, настройки, IPC | Не содержит логики, которую можно проверить без диска |

---

### Task 1: Каркас репозитория и Rust-ядро из `$G`

**Files:**
- Create: `Cargo.toml` (workspace), `crates/glafira-load-model/{Cargo.toml,src/lib.rs}`, `crates/glafira-load-contracts/{Cargo.toml,src/lib.rs}`, `crates/glafira-load-xlsx/{Cargo.toml,src/lib.rs}`, `deny.toml`, `LICENSE`, `NOTICE`, `TRADEMARK.md`, `SECURITY.md`, `CONTRIBUTING.md`
- Create (копии из `$G` с правками): `src-tauri/{Cargo.toml,build.rs,tauri.conf.json,capabilities/default.json}`, `src-tauri/src/{main,lib,product,error,i18n,logging,settings,state,sync,dock_name}.rs`, `src-tauri/src/policy/{mod,sources}.rs`, `src-tauri/src/storage/{mod,fs}.rs`, `src-tauri/src/commands/{mod,app,settings}.rs`
- Test: unit-тесты внутри перечисленных файлов (`cargo test --workspace`)

**Interfaces:**
- Produces: `crate::error::{AppError, AppResult, ErrorCode}` с кодами из Global Constraints; макросы `ltext!`, `app_err!`; `crate::storage::fs::{atomic_write, read_json, read_json_or_quarantine, write_json}`; `crate::settings::SettingsStore` с `projects()`, `remember_project(&str)`, `forget_project(&str)`, `set_active_project(Option<&str>)`; `crate::state::AppState { policy, settings }`; `crate::commands::blocking`.

- [ ] **Step 1: Workspace и пустые крейты**

```bash
cat > Cargo.toml <<'EOF'
[workspace]
resolver = "2"
members = ["src-tauri", "crates/glafira-load-model", "crates/glafira-load-contracts", "crates/glafira-load-xlsx"]

[workspace.package]
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/vvnovg/glafira-load"

[profile.release]
codegen-units = 1
lto = true
opt-level = 3
panic = "abort"
strip = true
EOF
for c in model contracts xlsx; do
  mkdir -p crates/glafira-load-$c/src
  cat > crates/glafira-load-$c/Cargo.toml <<EOF
[package]
name = "glafira-load-$c"
version = "0.1.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "Glafira for Load Testing: $c"

[dependencies]
EOF
  printf '//! Glafira for Load Testing: крейт `%s`. Заполняется в следующих задачах.\n' "$c" > crates/glafira-load-$c/src/lib.rs
done
```

- [ ] **Step 2: Скопировать ядро из `$G`**

```bash
G=/Users/vvnovg/projects/glafira-inference-llm
mkdir -p src-tauri/src/{policy,storage,commands} src-tauri/capabilities
cp $G/src-tauri/build.rs src-tauri/
cp $G/src-tauri/capabilities/default.json src-tauri/capabilities/
cp $G/src-tauri/tauri.conf.json src-tauri/
for f in main lib product error i18n logging settings state sync dock_name; do cp $G/src-tauri/src/$f.rs src-tauri/src/; done
cp $G/src-tauri/src/policy/{mod,sources}.rs src-tauri/src/policy/
cp $G/src-tauri/src/storage/fs.rs src-tauri/src/storage/
printf 'pub mod fs;\n' > src-tauri/src/storage/mod.rs
cp $G/src-tauri/src/commands/{app,settings}.rs src-tauri/src/commands/
cp $G/deny.toml $G/LICENSE $G/NOTICE $G/SECURITY.md $G/CONTRIBUTING.md $G/TRADEMARK.md .
```

В `TRADEMARK.md`, `SECURITY.md`, `CONTRIBUTING.md` заменить «Glafira for LLM Inference» на «Glafira for Load Testing» и адреса репозиториев на `vvnovg/glafira-load` / `vvnovg/glafira-load-releases`; убрать строки про vLLM/SGLang. В `TRADEMARK.md` упомянуть, что имя «Glafira», «Glafira for Load Testing» и иконка — товарные знаки.

- [ ] **Step 3: `src-tauri/Cargo.toml`**

```toml
[package]
name = "glafira-load-testing"
version = "0.1.0"
description = "Glafira for Load Testing — desktop-приложение для подготовки и сопровождения нагрузочного тестирования"
authors = ["Glafira for Load Testing contributors"]
edition = "2024"
license = "Apache-2.0"
repository = "https://github.com/vvnovg/glafira-load"

[lib]
# Суффикс _lib нужен, чтобы имя библиотеки не совпало с именем бинаря (Windows, cargo#8519).
name = "glafira_load_testing_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
tauri-plugin-dialog = "2"
tauri-plugin-updater = { version = "2.14", default-features = false, features = ["rustls-tls", "system-proxy"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "registry", "fmt"] }
plist = "1"
time = { version = "0.3", features = ["formatting", "parsing", "macros"] }
tokio = { version = "1", features = ["rt", "macros", "time", "process", "sync", "io-util"] }
sys-locale = "0.3"
glafira-load-model = { path = "../crates/glafira-load-model" }
glafira-load-contracts = { path = "../crates/glafira-load-contracts" }
glafira-load-xlsx = { path = "../crates/glafira-load-xlsx" }

[target.'cfg(windows)'.dependencies]
winreg = "0.55"

[dev-dependencies]
tempfile = "3"
```

`[profile.release]` в `src-tauri/Cargo.toml` не нужен: профиль задан в workspace.

- [ ] **Step 4: `product.rs`**

```rust
//! Имя продукта и идентификаторы — в одном месте. Переименование меняет только этот файл
//! и `tauri.conf.json`; тест ниже следит, чтобы они совпадали.

pub const PRODUCT_NAME: &str = "Glafira for Load Testing";
pub const IDENTIFIER: &str = "io.github.vvnovg.glafira.load";
/// Бинарь настольного приложения (`mainBinaryName`).
pub const MAIN_BINARY: &str = "glafira-load-testing";
/// Бинарь CLI-раннера (этап L2): его имя печатается в подсказках уже сейчас.
pub const RUNNER_BINARY: &str = "glafira-load";
pub const KEYCHAIN_SERVICE: &str = "glafira-load";
pub const POLICY_VENDOR: &str = "Glafira";
pub const POLICY_PRODUCT: &str = "LoadTesting";

#[cfg(test)]
mod tests {
    #[test]
    fn matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], super::IDENTIFIER);
        assert_eq!(conf["productName"], super::PRODUCT_NAME);
        assert_eq!(conf["mainBinaryName"], super::MAIN_BINARY);
        assert_eq!(conf["app"]["windows"][0]["title"], super::PRODUCT_NAME);
    }
}
```

- [ ] **Step 5: `tauri.conf.json`** — заменить значения (структура файла из `$G` сохраняется):

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Glafira for Load Testing",
  "mainBinaryName": "glafira-load-testing",
  "version": "0.1.0",
  "identifier": "io.github.vvnovg.glafira.load",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1450",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      { "title": "Glafira for Load Testing", "width": 1400, "height": 900, "minWidth": 900, "minHeight": 600, "dragDropEnabled": false }
    ],
    "security": {
      "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["app", "dmg", "msi", "deb", "rpm", "appimage"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"],
    "createUpdaterArtifacts": true,
    "publisher": "Glafira for Load Testing",
    "category": "DeveloperTool",
    "shortDescription": "Glafira for Load Testing — desktop tool for preparing and maintaining load tests",
    "copyright": "© 2026 Viacheslav Novgorodtsev",
    "linux": { "deb": { "depends": ["libsecret-1-0"] }, "rpm": { "depends": ["libsecret"] } },
    "macOS": { "minimumSystemVersion": "11.0" },
    "windows": { "wix": { "upgradeCode": "<НОВЫЙ GUID: uuidgen | tr A-Z a-z>", "language": ["en-US"] } }
  },
  "plugins": {
    "updater": {
      "pubkey": "REPLACE_WITH_UPDATER_PUBKEY",
      "endpoints": ["https://github.com/vvnovg/glafira-load-releases/releases/latest/download/latest.json"]
    }
  }
}
```

Порт `1450` — свой, чтобы `pnpm tauri dev` трёх Глафир работал одновременно. `upgradeCode` сгенерировать один раз командой `uuidgen | tr A-Z a-z` и не менять.

- [ ] **Step 6: `error.rs` — коды L1**

Заменить `enum ErrorCode` и `retryable_code`:

```rust
/// Коды ошибок (спека L1 §11). Фронтенд выбирает поведение по коду, текст — по `key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Timeout,
    Cancelled,
    Internal,
    /// Манифест или файл проекта не читается или не проходит проверки.
    ProjectInvalid,
    /// Файл контракта не разобран (остальные контракты разбираются).
    ContractParse,
    /// Нет `git`, папка не репозиторий, таймаут.
    GitUnavailable,
    /// Шаблон другого проекта или формата.
    TemplateMismatch,
    /// В шаблоне есть строки с ошибками.
    TemplateInvalid,
    /// Путь выходит за проект или клон.
    PathOutsideProject,
    /// Разбор сервиса уже идёт.
    ScanBusy,
}
```

```rust
    fn retryable_code(code: ErrorCode) -> bool {
        matches!(code, ErrorCode::Timeout)
    }
```

Тест `codes_serialize_and_only_network_ones_are_retryable` переписать под новый список:

```rust
    #[test]
    fn codes_serialize_and_only_timeout_is_retryable() {
        for (code, name, retryable) in [
            (ErrorCode::InvalidInput, "INVALID_INPUT", false),
            (ErrorCode::NotFound, "NOT_FOUND", false),
            (ErrorCode::Timeout, "TIMEOUT", true),
            (ErrorCode::Cancelled, "CANCELLED", false),
            (ErrorCode::Internal, "INTERNAL", false),
            (ErrorCode::ProjectInvalid, "PROJECT_INVALID", false),
            (ErrorCode::ContractParse, "CONTRACT_PARSE", false),
            (ErrorCode::GitUnavailable, "GIT_UNAVAILABLE", false),
            (ErrorCode::TemplateMismatch, "TEMPLATE_MISMATCH", false),
            (ErrorCode::TemplateInvalid, "TEMPLATE_INVALID", false),
            (ErrorCode::PathOutsideProject, "PATH_OUTSIDE_PROJECT", false),
            (ErrorCode::ScanBusy, "SCAN_BUSY", false),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), name);
            assert_eq!(
                AppError::localized(code, LocalizedText::raw("x")).retryable,
                retryable,
                "{name}"
            );
        }
    }
```

В тесте `serializes_for_frontend` ключ `errors.connections.emptyName` заменить на `errors.project.emptyName` с текстом `Project name cannot be empty`.

- [ ] **Step 7: `settings.rs` — без MCP, с открытыми проектами**

Удалить всё про MCP: `ApprovedClient`, `McpSettings`, поле `mcp` в `SettingsFile`, функции `mcp_enabled_in`, `mcp`, `set_mcp_enabled`, `is_client_approved`, `remember_client`, `touch_client`, `revoke_client` и тесты `mcp_settings_persist_next_to_language`, `unreadable_settings_mean_mcp_off_for_the_shim` и любые другие, упоминающие `mcp`. Заголовок модуля: `//! Пользовательские настройки (язык интерфейса, открытые проекты) и выбор действующего языка.` Добавить:

```rust
/// Сколько последних проектов помнит приложение.
pub const MAX_RECENT_PROJECTS: usize = 20;

/// Открытые проекты НТ (спека L1 §3.4): пути к папкам и последний активный.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsSettings {
    #[serde(default)]
    pub recent: Vec<String>,
    #[serde(default)]
    pub active: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    schema_version: u32,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    projects: ProjectsSettings,
}
```

и методы `SettingsStore`:

```rust
    pub fn projects(&self) -> ProjectsSettings {
        self.data.read_safe().projects.clone()
    }

    /// Блокирующий вызов. Проект поднимается в начало списка; список обрезается до `MAX_RECENT_PROJECTS`.
    pub fn remember_project(&self, path: &str) -> AppResult<()> {
        self.update(|d| {
            d.projects.recent.retain(|p| p != path);
            d.projects.recent.insert(0, path.to_string());
            d.projects.recent.truncate(MAX_RECENT_PROJECTS);
            d.projects.active = Some(path.to_string());
        })
    }

    /// Блокирующий вызов. Убирает проект из списка; активный проект сбрасывается, если это он.
    pub fn forget_project(&self, path: &str) -> AppResult<()> {
        self.update(|d| {
            d.projects.recent.retain(|p| p != path);
            if d.projects.active.as_deref() == Some(path) {
                d.projects.active = None;
            }
        })
    }

    /// Блокирующий вызов.
    pub fn set_active_project(&self, path: Option<&str>) -> AppResult<()> {
        self.update(|d| d.projects.active = path.map(String::from))
    }
```

Тесты (в `mod tests` файла):

```rust
    #[test]
    fn recent_projects_are_ordered_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let s = SettingsStore::open(dir.path());
        for i in 0..25 {
            s.remember_project(&format!("/p/{i}")).unwrap();
        }
        let p = s.projects();
        assert_eq!(p.recent.len(), MAX_RECENT_PROJECTS);
        assert_eq!(p.recent[0], "/p/24");
        assert_eq!(p.active.as_deref(), Some("/p/24"));
        s.remember_project("/p/10").unwrap();
        assert_eq!(s.projects().recent[0], "/p/10");
        s.forget_project("/p/10").unwrap();
        let p = SettingsStore::open(dir.path()).projects();
        assert!(!p.recent.contains(&"/p/10".to_string()));
        assert_eq!(p.active, None, "активный сброшен вместе с удалением");
    }

    #[test]
    fn language_and_projects_do_not_reset_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let s = SettingsStore::open(dir.path());
        s.set_language(Some("ru")).unwrap();
        s.remember_project("/p/a").unwrap();
        let again = SettingsStore::open(dir.path());
        assert_eq!(again.language().as_deref(), Some("ru"));
        assert_eq!(again.projects().recent, vec!["/p/a".to_string()]);
    }
```

- [ ] **Step 8: `policy/mod.rs` — без MCP и TLS**

Удалить константы `KEY_MCP_ENABLED` и `KEY_ALLOW_INSECURE_TLS`, поля `mcp_enabled` и `allow_insecure_tls` из `PolicyStatus`, их чтение в `resolve()` и все тесты, которые их упоминают. Остаются ключи `updatesEnabled`, `updateUrl`, `language`. В `policy/sources.rs` ничего не меняется (пути строятся из `product::IDENTIFIER`, `POLICY_VENDOR`, `POLICY_PRODUCT`).

- [ ] **Step 9: `state.rs`, `commands/mod.rs`, `lib.rs`, `main.rs`**

`state.rs`:

```rust
use std::path::Path;
use std::sync::Arc;

use crate::error::AppResult;

/// Общее состояние приложения: политика и настройки. Клонируется дёшево (всё за `Arc`).
/// Реестр идущих разборов добавляется в Task 13.
#[derive(Clone)]
pub struct AppState {
    pub policy: Arc<crate::policy::Policy>,
    pub settings: Arc<crate::settings::SettingsStore>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> AppResult<Self> {
        Ok(Self {
            policy: Arc::new(crate::policy::Policy::default()),
            settings: Arc::new(crate::settings::SettingsStore::open(data_dir)),
        })
    }

    /// Политика администратора (читается при запуске в lib.rs).
    pub fn with_policy(mut self, policy: crate::policy::Policy) -> Self {
        self.policy = Arc::new(policy);
        self
    }
}
```

`commands/mod.rs`:

```rust
pub mod app;
pub mod settings;

use crate::error::{AppResult, ErrorCode};

/// Блокирующие вызовы (файлы, git) не выполняются на async-потоках Tauri.
pub(crate) async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        app_err!(
            ErrorCode::Internal,
            "errors.app.taskPanicked",
            "Background task failed: {{error}}",
            error = e
        )
    })?
}
```

`lib.rs`:

```rust
#[macro_use]
pub mod i18n;
pub mod commands;
mod dock_name;
pub mod error;
pub mod logging;
pub mod policy;
pub mod product;
pub mod settings;
pub mod state;
pub mod storage;
pub mod sync;

use tauri::Manager;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            crate::logging::init(&app.path().app_log_dir()?);
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");
            if !crate::dock_name::set(crate::product::PRODUCT_NAME) {
                tracing::warn!("dock name was not set: LaunchServices refused");
            }
            let dir = app.path().app_data_dir()?;
            let policy = crate::policy::sources::load();
            tracing::info!(managed = policy.status.managed, "policy");
            let st = AppState::new(&dir)?.with_policy(policy);
            app.manage(st);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::get_app_info,
            commands::app::check_update,
            commands::app::install_update,
            commands::settings::get_language_settings,
            commands::settings::set_language,
            commands::settings::get_policy_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
```

`main.rs`:

```rust
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    glafira_load_testing_lib::run()
}
```

В `commands/settings.rs` тесты создают `AppState::new(dir, Arc::new(MemoryStore::default()))` — заменить на `AppState::new(dir)` и убрать `use crate::secrets::MemoryStore`. В `commands/app.rs` и `settings.rs` убрать упоминания `secrets`, если `cargo check` их найдёт. В `logging.rs` и `i18n.rs` править нечего.

- [ ] **Step 10: Проверка**

Run: `cargo test --workspace`
Expected: все тесты `product`, `error`, `settings`, `policy`, `storage::fs`, `i18n`, `sync`, `logging` проходят; пустые крейты компилируются. `cargo clippy --workspace --all-targets -- -D warnings` без замечаний. Чтобы `tauri::generate_context!` нашёл иконки, временно выполнить `pnpm tauri icon` нельзя (фронтенда ещё нет): до Task 2 скопировать `$G/src-tauri/icons` в `src-tauri/icons` как заглушку.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "feat(core): workspace, пустые крейты и Rust-ядро из Glafira for LLM Inference без MCP и секретов

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Каркас фронтенда — оболочка, вкладки, i18n en/ru, справка, иконка

**Files:**
- Create (копии из `$G`): `package.json`, `pnpm-workspace.yaml`, `index.html`, `vite.config.ts`, `vitest.config.ts`, `tsconfig.json`, `tsconfig.node.json`, `src/{main.tsx,App.tsx,styles.css,app.css,polyfills.ts,vite-env.d.ts}`, `src/api/{client,types}.ts`, `src/i18n/*`, `src/guide/*`, `src/lib/*`, `src/state/*`, `src/test/*`, `src/components/{Layout,Sidebar,TabContent}.tsx`, `src/components/common/{ErrorBanner,Hint,ExternalLink,ChunkBoundary}.tsx`, `src/components/{settings,about,help}/*`, `src/locales/{en,ru}.json`, `scripts/*`, `docs/guide/{en,ru}/*.md`
- Create: `src-tauri/icons/*` (генерация), `.github/workflows/ci.yml`
- Test: `src/state/workspace.test.ts`, `src/components/Layout.test.tsx`, существующие тесты скопированных модулей

**Interfaces:**
- Produces: `TabSpec` с видами `project | service | registry | prod-load | profile | about | help | settings`; `tabId`, `tabProject`, `useWorkspace.closeForProject`; `api.getAppInfo/checkUpdate/installUpdate/getLanguageSettings/setLanguage/getPolicyStatus`; словари с ветками `common`, `errors`, `sidebar`, `tabs`, `layout`, `settings`, `about`, `help`, `support`, `menu`, `hints`; `GUIDE_CHAPTERS`.

- [ ] **Step 1: Скопировать фронтенд и скрипты**

```bash
G=/Users/vvnovg/projects/glafira-inference-llm
cp $G/package.json $G/pnpm-workspace.yaml $G/index.html $G/vite.config.ts $G/vitest.config.ts $G/tsconfig.json $G/tsconfig.node.json .
mkdir -p src/{api,i18n,guide,lib,state,test,locales,components/{common,settings,about,help}} scripts docs/guide/{en,ru} .github/workflows
cp $G/src/{main.tsx,App.tsx,styles.css,app.css,polyfills.ts,vite-env.d.ts} src/
cp $G/src/api/client.ts $G/src/api/types.ts src/api/
cp $G/src/i18n/*.ts src/i18n/
cp $G/src/guide/*.ts src/guide/
cp $G/src/lib/{env,focusTrap,format,formatAgo,links,plainInputs,tabActive,useAppMenu,useDebouncedValue,useHelpShortcut,useLanguageSync}.ts src/lib/
cp $G/src/lib/*.test.ts* src/lib/ 2>/dev/null
cp $G/src/state/{workspace,helpContext}.ts src/state/
cp $G/src/test/* src/test/
cp $G/src/components/{Layout,Sidebar,TabContent}.tsx src/components/
cp $G/src/components/common/{ErrorBanner,Hint,ExternalLink,ChunkBoundary}.tsx $G/src/components/common/*.test.tsx src/components/common/
cp $G/src/components/settings/SettingsPanel*.tsx src/components/settings/
cp $G/src/components/about/*.tsx src/components/about/
cp $G/src/components/help/* src/components/help/
cp $G/scripts/{i18n-check,check-dist-compat,third-party,changelog,guide-publish}.mjs $G/scripts/*.test.mjs $G/scripts/guide-publish.d.mts scripts/
rm -f scripts/{mock-server,merge-locales,make-latest-json,check-release,macos-notarize}*.mjs scripts/*.sh
cp $G/.github/workflows/ci.yml .github/workflows/
```

В `package.json`: `"name": "glafira-load-testing"`, `"version": "0.1.0"`, убрать скрипт `mock-server`; в `dependencies` добавить `"@tanstack/react-virtual": "^3.14.18"` и `"uplot": "1.6.32"` (как в `$O/package.json`); скрипт `i18n:check` оставить `node scripts/i18n-check.mjs --rust-cyrillic src-tauri/src crates`. В `vite.config.ts` порт `1450` и `hmr.port: 1451`. Затем `pnpm install` (создаёт `pnpm-lock.yaml`).

- [ ] **Step 2: Иконки**

Run: `pnpm tauri icon public/app-icon.svg`
Expected: в `src-tauri/icons/` появляются `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`, `icon.png` и наборы Square*; заглушка из Task 1 перезаписана. `git status` показывает изменённые иконки.

- [ ] **Step 3: Тест на `workspace.ts`**

`src/state/workspace.test.ts`:

```ts
import { tabId, tabProject, useWorkspace } from "./workspace";

describe("workspace tabs", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("ids include the project path and the service id", () => {
    expect(tabId({ kind: "project", path: "/p/a" })).toBe("project:/p/a");
    expect(tabId({ kind: "project", path: null })).toBe("project:new");
    expect(tabId({ kind: "service", path: "/p/a", serviceId: "orders" })).toBe("service:/p/a:orders");
    expect(tabId({ kind: "registry", path: "/p/a" })).toBe("registry:/p/a");
    expect(tabId({ kind: "prod-load", path: "/p/a" })).toBe("prod-load:/p/a");
    expect(tabId({ kind: "profile", path: "/p/a" })).toBe("profile:/p/a");
    expect(tabId({ kind: "help" })).toBe("help");
  });

  it("closeForProject closes every tab of the project and keeps the rest", () => {
    const { open, closeForProject } = useWorkspace.getState();
    open({ kind: "registry", path: "/p/a" });
    open({ kind: "profile", path: "/p/a" });
    open({ kind: "registry", path: "/p/b" });
    closeForProject("/p/a");
    const { tabs, activeId } = useWorkspace.getState();
    expect(tabs.map((t) => t.id)).toEqual(["registry:/p/b"]);
    expect(activeId).toBe("registry:/p/b");
    expect(tabProject({ kind: "about" })).toBeNull();
  });
});
```

- [ ] **Step 4: Run test to verify it fails**

Run: `pnpm vitest run src/state/workspace.test.ts`
Expected: FAIL — `tabProject` не экспортируется, `kind: "project"` не входит в `TabSpec`.

- [ ] **Step 5: `src/state/workspace.ts`**

```ts
import { create } from "zustand";

export type TabSpec =
  /** Проект: манифест, сервисы, стенды. `path: null` — форма нового проекта. */
  | { kind: "project"; path: string | null }
  | { kind: "service"; path: string; serviceId: string }
  | { kind: "registry"; path: string }
  | { kind: "prod-load"; path: string }
  | { kind: "profile"; path: string }
  | { kind: "about" }
  /** Справка: глава и раздел руководства (F1, ссылки). */
  | { kind: "help"; chapter?: string; anchor?: string }
  | { kind: "settings" };

export type Tab = TabSpec & { id: string };

export function tabId(spec: TabSpec): string {
  switch (spec.kind) {
    case "project":
      return `project:${spec.path ?? "new"}`;
    case "service":
      return `service:${spec.path}:${spec.serviceId}`;
    case "registry":
    case "prod-load":
    case "profile":
      return `${spec.kind}:${spec.path}`;
    default:
      return spec.kind;
  }
}

/** Путь проекта вкладки или `null`, если вкладка не про проект. */
export function tabProject(tab: TabSpec): string | null {
  return "path" in tab ? tab.path : null;
}

interface WorkspaceState {
  tabs: Tab[];
  activeId: string | null;
  open: (spec: TabSpec) => void;
  close: (id: string) => void;
  /** Закрывает все вкладки проекта (после «Убрать» в дереве). */
  closeForProject: (path: string) => void;
  activate: (id: string) => void;
}

export const useWorkspace = create<WorkspaceState>((set) => ({
  tabs: [],
  activeId: null,
  open: (spec) =>
    set((s) => {
      const id = tabId(spec);
      const tab = { ...spec, id } as Tab;
      // Уже открытая вкладка получает новый spec (например, главу справки).
      if (s.tabs.some((t) => t.id === id)) return { tabs: s.tabs.map((t) => (t.id === id ? tab : t)), activeId: id };
      return { tabs: [...s.tabs, tab], activeId: id };
    }),
  close: (id) =>
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      if (idx < 0) return s;
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeId = s.activeId !== id ? s.activeId : (tabs[idx] ?? tabs[idx - 1])?.id ?? null;
      return { tabs, activeId };
    }),
  closeForProject: (path) =>
    set((s) => {
      const firstIdx = s.tabs.findIndex((t) => tabProject(t) === path);
      if (firstIdx < 0) return s;
      const tabs = s.tabs.filter((t) => tabProject(t) !== path);
      const activeId = tabs.some((t) => t.id === s.activeId)
        ? s.activeId
        : (tabs[firstIdx] ?? tabs[tabs.length - 1])?.id ?? null;
      return { tabs, activeId };
    }),
  activate: (id) => set({ activeId: id }),
}));
```

- [ ] **Step 6: `api/types.ts` и `api/client.ts` — только общие типы**

В `types.ts` оставить `ErrorCode` (новый список), `LocalizedText`, `AppError`, `Environment`, `AppInfo`, `UpdateCheck`, `LanguageSettings`, `PolicyStatus`; всё про инсталляции, серверы, папки, экспорт удалить. `ErrorCode`:

```ts
export type ErrorCode =
  | "INVALID_INPUT"
  | "NOT_FOUND"
  | "TIMEOUT"
  | "CANCELLED"
  | "INTERNAL"
  | "PROJECT_INVALID"
  | "CONTRACT_PARSE"
  | "GIT_UNAVAILABLE"
  | "TEMPLATE_MISMATCH"
  | "TEMPLATE_INVALID"
  | "PATH_OUTSIDE_PROJECT"
  | "SCAN_BUSY";
```

Из `PolicyStatus` убрать `mcpEnabled` и `allowInsecureTls`. В `client.ts` оставить `isAppError`, `toAppError`, `call` и шесть методов: `getAppInfo`, `checkUpdate`, `installUpdate`, `getLanguageSettings`, `setLanguage`, `getPolicyStatus`. Типы и методы команд проекта добавляются в Task 15.

- [ ] **Step 7: Оболочка**

`src/components/Sidebar.tsx` (дерево появится в Task 15):

```tsx
import { useTranslation } from "react-i18next";
import { useWorkspace } from "../state/workspace";
import { openSupport } from "../lib/links";

export function Sidebar() {
  const { t } = useTranslation();
  const open = useWorkspace((s) => s.open);

  return (
    <aside className="sidebar">
      <div className="tree" data-testid="project-tree">
        <p className="empty">{t("sidebar.emptyHint")}</p>
      </div>
      <div className="sidebar-footer">
        <button type="button" className="link" onClick={() => open({ kind: "settings" })}>
          {t("sidebar.settings")}
        </button>
        <button type="button" className="link" onClick={openSupport}>
          {t("support.title")}
        </button>
        <button type="button" className="link" onClick={() => open({ kind: "help" })}>
          {t("sidebar.help")}
        </button>
        <button type="button" className="link" onClick={() => open({ kind: "about" })}>
          {t("sidebar.about")}
        </button>
      </div>
    </aside>
  );
}
```

`src/components/Layout.tsx` — из `$G` убрать `EnvBadge`, `env-stripe`, запрос `installations` и `tabInstallation`; заголовки:

```tsx
import { useTranslation } from "react-i18next";
import { type Tab, useWorkspace } from "../state/workspace";
import { Sidebar } from "./Sidebar";
import { i18n } from "../i18n";
import { TabContent } from "./TabContent";
import { UpdateNotice } from "./about/UpdateNotice";
import { TabActiveContext } from "../lib/tabActive";
import { useHelpShortcut } from "../lib/useHelpShortcut";

/** Имя папки проекта: заголовки вкладок до загрузки манифеста (и для тестов). */
export function projectLabel(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
}

function tabTitle(tab: Tab): string {
  switch (tab.kind) {
    case "settings":
      return i18n.t("tabs.settings");
    case "about":
      return i18n.t("tabs.about");
    case "help":
      return i18n.t("tabs.help");
    case "project":
      return tab.path ? i18n.t("tabs.project", { name: projectLabel(tab.path) }) : i18n.t("tabs.newProject");
    case "service":
      return i18n.t("tabs.service", { name: projectLabel(tab.path), service: tab.serviceId });
    case "registry":
      return i18n.t("tabs.registry", { name: projectLabel(tab.path) });
    case "prod-load":
      return i18n.t("tabs.prodLoad", { name: projectLabel(tab.path) });
    case "profile":
      return i18n.t("tabs.profile", { name: projectLabel(tab.path) });
  }
}

export function Layout() {
  const { t } = useTranslation(); // заодно перерисовывает заголовки вкладок при смене языка
  const { tabs, activeId, activate, close } = useWorkspace();
  useHelpShortcut();

  return (
    <div className="layout">
      <Sidebar />
      <main className="workspace">
        <UpdateNotice />
        <nav className="tabbar" role="tablist">
          {tabs.map((tab) => {
            const isActive = tab.id === activeId;
            return (
              <div key={tab.id} className={isActive ? "tab active" : "tab"}>
                <button type="button" role="tab" aria-selected={isActive} onClick={() => activate(tab.id)}>
                  {tabTitle(tab)}
                </button>
                <button
                  type="button"
                  className="icon"
                  aria-label={t("layout.closeTab")}
                  onClick={(e) => {
                    e.stopPropagation();
                    close(tab.id);
                  }}
                >
                  ×
                </button>
              </div>
            );
          })}
        </nav>
        {tabs.length === 0 && <p className="empty">{t("layout.empty")}</p>}
        {/* Неактивные вкладки скрыты, но не размонтированы — состояние форм и прогресс разбора сохраняются. */}
        {tabs.map((tab) => (
          <section key={tab.id} className="tab-content" hidden={tab.id !== activeId}>
            <TabActiveContext.Provider value={tab.id === activeId}>
              <TabContent tab={tab} />
            </TabActiveContext.Provider>
          </section>
        ))}
      </main>
    </div>
  );
}
```

`src/components/TabContent.tsx` — вкладки проекта пока заглушки (их заменяют Tasks 15–19):

```tsx
import { lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import type { Tab } from "../state/workspace";
import { AboutPanel } from "./about/AboutPanel";
import { ChunkBoundary } from "./common/ChunkBoundary";
import { SettingsPanel } from "./settings/SettingsPanel";

// Справка (react-markdown и главы) грузится отдельным чанком при первом открытии.
const HelpPanel = lazy(() => import("./help/HelpPanel").then((m) => ({ default: m.HelpPanel })));

function LazyHelp({ tab }: { tab: Extract<Tab, { kind: "help" }> }) {
  const { t } = useTranslation();
  return (
    <ChunkBoundary fallback={<p className="empty">{t("help.loadFailed")}</p>}>
      <Suspense fallback={<p className="empty">{t("common.loading")}</p>}>
        <HelpPanel spec={tab} />
      </Suspense>
    </ChunkBoundary>
  );
}

export function TabContent({ tab }: { tab: Tab }) {
  const { t } = useTranslation();
  switch (tab.kind) {
    case "about":
      return <AboutPanel />;
    case "help":
      return <LazyHelp tab={tab} />;
    case "settings":
      return <SettingsPanel />;
    default:
      // Экраны проекта появляются в Tasks 15–19.
      return <p className="empty">{t("common.loading")}</p>;
  }
}
```

`src/components/help/helpChapterFor.ts`:

```ts
import type { ChapterId } from "../../guide/chapters";
import type { TabSpec } from "../../state/workspace";

/** Глава руководства для экрана (F1). */
export function helpChapterFor(tab: TabSpec | null): ChapterId {
  switch (tab?.kind) {
    case "project":
    case "service":
      return "project";
    case "registry":
      return "registry";
    case "prod-load":
      return "prod-load";
    case "profile":
      return "profile";
    case "about":
      return "updates";
    default:
      return "getting-started";
  }
}
```

`src/guide/chapters.ts`: `export const GUIDE_CHAPTERS = ["getting-started", "project", "registry", "prod-load", "profile", "updates"] as const;` — тот же список в `scripts/i18n-check.mjs` (`export const GUIDE_CHAPTERS = [...]`). `src/lib/links.ts`: адреса `https://github.com/vvnovg/glafira-load-releases/blob/main/SUPPORT.md` и `https://github.com/vvnovg/glafira-load/blob/main/LICENSE`. `src/components/about/AboutPanel.tsx` править не нужно (использует только `about.*`, `support.*`).

- [ ] **Step 8: Словари**

`src/locales/en.json` — ветки `common`, `errors.{app,settings,internal,io,json,generic,policy,storage,updates}`, `settings`, `about` (кроме `summary` и `trademarks`), `help`, `support`, `menu`, `layout.closeTab`, `hints.label` скопировать из `$G/src/locales/en.json` без изменений; ветки `installations`, `sidebar.*` кроме трёх ссылок, `errors.{installations,secrets,server,import,folders}`, `hints.installations` удалить. Добавить и заменить:

```json
{
  "sidebar": {
    "settings": "Settings",
    "help": "Help",
    "about": "About",
    "projects": "Load projects",
    "openProject": "Open project…",
    "newProject": "New project…",
    "emptyHint": "Open a load project folder or create a new one.",
    "remove": "Remove from the list",
    "missing": "Folder not found",
    "services": "Services",
    "registry": "Operation registry",
    "prodLoad": "Production load",
    "profile": "Load profile",
    "environments": "Environments"
  },
  "tabs": {
    "settings": "Settings",
    "about": "About",
    "help": "Help",
    "project": "{{name}}",
    "newProject": "New project",
    "service": "{{name}} · {{service}}",
    "registry": "{{name}} · Registry",
    "prodLoad": "{{name}} · Production load",
    "profile": "{{name}} · Load profile"
  },
  "layout": { "closeTab": "Close tab", "empty": "Open a project on the left or create a new one." },
  "about": {
    "summary": "Glafira for Load Testing is a desktop tool for preparing and maintaining load tests. This version (stage L1) builds the registry of operations from the contracts of your services, exports the production-load template to Excel, imports the filled template and computes the load profile. The CLI runner, suites, mocks and reports come in the next stages.",
    "trademarks": "Apache Kafka is a trademark of the Apache Software Foundation. Other product names are trademarks of their respective owners. This project is not affiliated with them."
  },
  "errors": {
    "project": { "emptyName": "Project name cannot be empty" }
  }
}
```

`ru.json` — те же ключи по-русски (ветки из `$G/src/locales/ru.json` копируются так же). `sidebar`: «Настройки», «Справка», «О программе», «Проекты НТ», «Открыть проект…», «Новый проект…», «Откройте папку проекта НТ или создайте новый.», «Убрать из списка», «Папка не найдена», «Сервисы», «Реестр операций», «Нагрузка ПРОМ», «Профиль нагрузки», «Стенды». `tabs`: «{{name}}», «Новый проект», «{{name}} · {{service}}», «{{name}} · Реестр», «{{name}} · Нагрузка ПРОМ», «{{name}} · Профиль». `layout.empty`: «Откройте проект слева или создайте новый.» `about.summary`: «Glafira for Load Testing — настольный инструмент подготовки и сопровождения нагрузочного тестирования. Эта версия (этап L1) строит реестр операций по контрактам сервисов, выгружает шаблон нагрузки ПРОМ в Excel, импортирует заполненный шаблон и считает профиль нагрузки. CLI-раннер, сьюты, моки и отчёты появятся в следующих этапах.» `about.trademarks`: «Apache Kafka — товарный знак Apache Software Foundation. Остальные названия продуктов — товарные знаки их владельцев. Проект с ними не связан.» `errors.project.emptyName`: «Имя проекта не может быть пустым».

- [ ] **Step 9: Главы руководства-заглушки**

Для каждой главы из `GUIDE_CHAPTERS` создать `docs/guide/en/<id>.md` и `docs/guide/ru/<id>.md`. Содержимое `getting-started.md`, `updates.md` взять из `$G` и заменить имя продукта и описание (Task 20 допишет остальные главы). Остальные четыре — по одному абзацу:

`docs/guide/en/project.md`:
```markdown
# Project and services

A load project is a folder in git: the manifest `glafira-load.yaml`, the operation registry, the production-load figures and the load profile. This chapter is completed at the end of stage L1.
```

`docs/guide/ru/project.md`:
```markdown
# Проект и сервисы

Проект НТ — папка в git: манифест `glafira-load.yaml`, реестр операций, цифры нагрузки ПРОМ и профиль нагрузки. Глава дописывается в конце этапа L1.
```

Аналогично `registry.md` («Operation registry» / «Реестр операций»), `prod-load.md` («Production load» / «Нагрузка ПРОМ»), `profile.md` («Load profile» / «Профиль нагрузки»).

- [ ] **Step 10: Тест оболочки**

`src/components/Layout.test.tsx`:

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../test/render";
import { useWorkspace } from "../state/workspace";
import { Layout, projectLabel } from "./Layout";

vi.mock("../api/client", () => ({
  api: {
    getAppInfo: vi.fn(async () => ({ name: "Glafira", version: "0.1.0", commit: "x", os: "macos", arch: "aarch64", dataDir: "/d", logDir: "/l" })),
    getLanguageSettings: vi.fn(async () => ({ effective: "ru", chosen: null, policy: null, systemMatch: null, reason: "default", languages: ["en", "ru"] })),
    getPolicyStatus: vi.fn(async () => ({ managed: false, source: null, updatesEnabled: false, updateUrl: null, language: null, errors: [] })),
    checkUpdate: vi.fn(async () => ({ status: "disabled", reason: { key: "errors.updates.notConfigured", params: {}, text: "x" } })),
  },
  toAppError: (e: unknown) => e,
  isAppError: () => false,
}));

describe("Layout", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("opens the registry tab with the project folder name and closes it", async () => {
    renderWithClient(<Layout />);
    useWorkspace.getState().open({ kind: "registry", path: "/home/u/orders-load" });
    expect(await screen.findByRole("tab", { name: "orders-load · Реестр" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Закрыть вкладку" }));
    expect(screen.queryByRole("tab")).toBeNull();
    expect(projectLabel("C:\\work\\p\\")).toBe("p");
  });
});
```

- [ ] **Step 11: Проверка и CI**

Run: `pnpm test && pnpm i18n:check && pnpm build && cargo test --workspace`
Expected: все тесты проходят; `i18n:check` не находит расхождений ключей en/ru и глав; `pnpm build` собирает `dist/`. `pnpm tauri dev` открывает окно «Glafira for Load Testing» с пустым деревом, вкладками «Настройки», «Справка», «О программе».

`.github/workflows/ci.yml` — из `$G` с правками: `workspaces: |` → `. -> target`; шаги `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; `cargo deny --manifest-path src-tauri/Cargo.toml --config deny.toml check licenses advisories` (workspace-граф общий); убрать комментарий про серверы инференса. Job `platforms` (macos-15, windows-latest) — те же правки.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "feat(ui): оболочка приложения, вкладки проекта, словари en/ru, справка и иконки

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Крейт `glafira-load-model` — манифест, сжатая схема, реестр операций

**Files:**
- Modify: `crates/glafira-load-model/Cargo.toml`
- Create: `crates/glafira-load-model/src/{lib,project,schema,registry}.rs`
- Test: unit-тесты в тех же файлах

**Interfaces:**
- Produces: `glafira_load_model::{from_yaml, to_yaml, ModelError}`; `project::{Manifest, Service, ContractRef, ContractKind, KafkaMap, TopicSchema, Direction, Environment, EnvKind, ManifestProblem, validate, MANIFEST_FILE}`; `schema::{SchemaSummary, fingerprint, fingerprint_of, MAX_DEPTH, MAX_CHILDREN}`; `registry::{OpKind, Address, Source, SourceKind, Status, Streaming, Seen, Manual, OperationDraft, Operation, Registry, operation_id, address_label, REGISTRY_FILE, SCANS_DIR}`.

- [ ] **Step 1: Зависимости**

`crates/glafira-load-model/Cargo.toml`, секция `[dependencies]`:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml_ng = "0.10"
sha2 = "0.11"
hex = "0.4"
thiserror = "2"
```

- [ ] **Step 2: Тесты манифеста** — `crates/glafira-load-model/src/project.rs`, в конце файла:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
format: 1
name: orders-platform
services:
  - id: orders
    name: Orders Service
    path: ../orders-service
    contracts:
      - kind: openapi
        path: api/openapi.yaml
      - kind: proto
        path: proto
        include: [proto, third_party]
        enabled: false
    kafka:
      topics:
        orders.created: schemas/orders-created.avsc
        orders.paid: { schema: schemas/paid.avsc, direction: consume }
    team: platform
environments:
  - id: test
    kind: test
    http: https://orders.test.local
    kafka: kafka-1.test.local:9092
"#;

    #[test]
    fn reads_manifest_keeps_unknown_fields_and_writes_them_back() {
        let m: Manifest = crate::from_yaml(SAMPLE).unwrap();
        assert_eq!(m.name, "orders-platform");
        assert_eq!(m.services[0].contracts[1].kind, ContractKind::Proto);
        assert!(!m.services[0].contracts[1].enabled);
        assert!(m.services[0].contracts[0].enabled, "enabled по умолчанию true");
        assert_eq!(
            m.services[0].kafka.topics["orders.paid"],
            TopicSchema::Full { schema: "schemas/paid.avsc".into(), direction: Direction::Consume }
        );
        assert_eq!(m.services[0].extra["team"], serde_json::json!("platform"));
        assert_eq!(m.environments[0].kind, EnvKind::Test);
        let again: Manifest = crate::from_yaml(&crate::to_yaml(&m)).unwrap();
        assert_eq!(again, m, "неизвестное поле team пережило запись");
        assert!(validate(&m).is_empty());
    }

    #[test]
    fn validation_finds_bad_and_duplicate_ids() {
        let mut m: Manifest = crate::from_yaml(SAMPLE).unwrap();
        m.services.push(Service { id: "orders".into(), name: String::new(), path: "x".into(), contracts: vec![], kafka: KafkaMap::default(), extra: Default::default() });
        m.services.push(Service { id: "Bad Id".into(), name: String::new(), path: String::new(), contracts: vec![], kafka: KafkaMap::default(), extra: Default::default() });
        m.environments.push(Environment { id: "test".into(), kind: EnvKind::Prod, http: None, kafka: None, grpc: None, sql: None, extra: Default::default() });
        m.name = "  ".into();
        m.format = 2;
        let problems = validate(&m);
        assert_eq!(
            problems,
            vec![
                ManifestProblem::BadFormat { format: 2 },
                ManifestProblem::EmptyName,
                ManifestProblem::DuplicateServiceId { id: "orders".into() },
                ManifestProblem::BadServiceId { id: "Bad Id".into() },
                ManifestProblem::EmptyServicePath { id: "Bad Id".into() },
                ManifestProblem::DuplicateEnvironmentId { id: "test".into() },
            ]
        );
    }

    #[test]
    fn topic_schema_direction_defaults_to_produce() {
        let t: TopicSchema = crate::from_yaml("schemas/a.avsc").unwrap();
        assert_eq!(t.schema(), "schemas/a.avsc");
        assert_eq!(t.direction(), Direction::Produce);
    }

    #[test]
    fn bad_yaml_reports_a_line() {
        let e = crate::from_yaml::<Manifest>("format: 1\nname: x\nservices: [\n").unwrap_err();
        assert!(e.line.is_some(), "{e:?}");
    }
}
```

- [ ] **Step 3: `lib.rs` и `project.rs`**

`crates/glafira-load-model/src/lib.rs`:

```rust
//! Модель файлов проекта НТ: типы и чистые функции без I/O (спека L1 §3, §6–§8).
//! Диск читает и пишет приложение; раннер (L2) использует те же типы.

pub mod project;
pub mod registry;
pub mod schema;

/// Ошибка чтения файла проекта: текст для `details` и строка, если YAML её знает.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ModelError {
    pub message: String,
    pub line: Option<u32>,
}

impl From<serde_yaml_ng::Error> for ModelError {
    fn from(e: serde_yaml_ng::Error) -> Self {
        Self {
            message: e.to_string(),
            line: e.location().map(|l| l.line() as u32),
        }
    }
}

pub fn from_yaml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ModelError> {
    Ok(serde_yaml_ng::from_str(text)?)
}

/// Файлы проекта пишутся только из согласованных структур: ошибка сериализации — баг, не ввод.
pub fn to_yaml<T: serde::Serialize>(value: &T) -> String {
    serde_yaml_ng::to_string(value).expect("project files serialize")
}
```

`crates/glafira-load-model/src/project.rs`:

```rust
//! Манифест проекта `glafira-load.yaml` (спека L1 §3.2–§3.3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "glafira-load.yaml";
pub const FORMAT: u32 = 1;
/// Максимальная длина `id` сервиса и стенда.
pub const MAX_ID_LEN: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    Openapi,
    Proto,
    Asyncapi,
    Schema,
    Sql,
    Spring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractRef {
    pub kind: ContractKind,
    /// Относительно клона сервиса, разделитель `/`.
    pub path: String,
    /// Каталоги для `import` (proto), относительно клона.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Produce,
    Consume,
}

/// Схема топика без AsyncAPI (решение плана 6): путь к файлу или путь с направлением.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TopicSchema {
    Path(String),
    Full {
        schema: String,
        #[serde(default)]
        direction: Direction,
    },
}

impl TopicSchema {
    pub fn schema(&self) -> &str {
        match self {
            TopicSchema::Path(p) => p,
            TopicSchema::Full { schema, .. } => schema,
        }
    }
    pub fn direction(&self) -> Direction {
        match self {
            TopicSchema::Path(_) => Direction::Produce,
            TopicSchema::Full { direction, .. } => *direction,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KafkaMap {
    #[serde(default)]
    pub topics: BTreeMap<String, TopicSchema>,
}

impl KafkaMap {
    pub fn is_empty(&self) -> bool {
        self.topics.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Service {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Клон git: относительно проекта или абсолютный.
    pub path: String,
    /// Пусто — автопоиск по маскам (спека §5.1).
    #[serde(default)]
    pub contracts: Vec<ContractRef>,
    #[serde(default, skip_serializing_if = "KafkaMap::is_empty")]
    pub kafka: KafkaMap,
    /// Неизвестные поля переживают перезапись.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvKind {
    Dev,
    Test,
    Stage,
    Prod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Environment {
    pub id: String,
    pub kind: EnvKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kafka: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc: Option<String>,
    /// Строка подключения без пароля; пароль — L2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub services: Vec<Service>,
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Manifest {
    pub fn new(name: &str) -> Self {
        Self { format: FORMAT, name: name.into(), services: vec![], environments: vec![], extra: BTreeMap::new() }
    }

    pub fn service(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.id == id)
    }
}

/// Нарушение правил §3.3. Отдаётся фронтенду как есть: текст подбирается по `kind`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestProblem {
    BadFormat { format: u32 },
    EmptyName,
    BadServiceId { id: String },
    DuplicateServiceId { id: String },
    EmptyServicePath { id: String },
    BadEnvironmentId { id: String },
    DuplicateEnvironmentId { id: String },
}

/// `[a-z0-9-]{1,64}`.
pub fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LEN
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Проверки без обращения к диску; существование путей проверяет приложение.
pub fn validate(m: &Manifest) -> Vec<ManifestProblem> {
    let mut out = Vec::new();
    if m.format != FORMAT {
        out.push(ManifestProblem::BadFormat { format: m.format });
    }
    if m.name.trim().is_empty() {
        out.push(ManifestProblem::EmptyName);
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in &m.services {
        if !seen.insert(s.id.as_str()) {
            out.push(ManifestProblem::DuplicateServiceId { id: s.id.clone() });
        } else if !is_id(&s.id) {
            out.push(ManifestProblem::BadServiceId { id: s.id.clone() });
        }
        if s.path.trim().is_empty() {
            out.push(ManifestProblem::EmptyServicePath { id: s.id.clone() });
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for e in &m.environments {
        if !seen.insert(e.id.as_str()) {
            out.push(ManifestProblem::DuplicateEnvironmentId { id: e.id.clone() });
        } else if !is_id(&e.id) {
            out.push(ManifestProblem::BadEnvironmentId { id: e.id.clone() });
        }
    }
    out
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p glafira-load-model project`
Expected: 4 теста PASS. Если `serde_yaml_ng` при `flatten` вернёт неизвестное поле как число/булево, `extra` хранит `serde_json::Value` — это ожидаемо.

- [ ] **Step 5: Тесты и код сжатой схемы** — `crates/glafira-load-model/src/schema.rs`:

```rust
//! Сжатая схема запроса и ответа и её отпечаток (спека L1 §5.2).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_DEPTH: usize = 6;
pub const MAX_CHILDREN: usize = 200;

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SchemaSummary {
    pub name: String,
    /// `object`, `array`, `string`, `integer`, `number`, `boolean`, `bytes`, `enum`, `one_of`, `any`, имя message у proto.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, rename = "enum", skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// Поля объекта, единственный элемент `items` у массива, варианты у `one_of`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SchemaSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

impl SchemaSummary {
    pub fn leaf(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self { name: name.into(), kind: kind.into(), ..Default::default() }
    }

    pub fn object(name: impl Into<String>, children: Vec<SchemaSummary>) -> Self {
        Self { name: name.into(), kind: "object".into(), children, ..Default::default() }
    }

    pub fn array(name: impl Into<String>, mut items: SchemaSummary) -> Self {
        items.name = "items".into();
        Self { name: name.into(), kind: "array".into(), children: vec![items], ..Default::default() }
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn with_example(mut self, example: impl Into<String>) -> Self {
        self.example = Some(example.into());
        self
    }

    /// Обрезает дерево до `MAX_DEPTH` уровней и `MAX_CHILDREN` полей на уровне, помечая обрезанное.
    pub fn limit(&mut self) {
        self.limit_at(1);
    }

    fn limit_at(&mut self, depth: usize) {
        if self.children.len() > MAX_CHILDREN {
            self.children.truncate(MAX_CHILDREN);
            self.truncated = true;
        }
        if depth >= MAX_DEPTH && !self.children.is_empty() {
            self.children.clear();
            self.truncated = true;
            return;
        }
        for c in &mut self.children {
            c.limit_at(depth + 1);
        }
    }

    /// Каноническая форма для отпечатка: без примеров, поля объекта по имени.
    pub fn canonical(&self) -> serde_json::Value {
        let mut children: Vec<&SchemaSummary> = self.children.iter().collect();
        if self.kind == "object" {
            children.sort_by(|a, b| a.name.cmp(&b.name));
        }
        serde_json::json!({
            "n": self.name,
            "t": self.kind,
            "r": self.required,
            "e": self.enum_values,
            "c": children.iter().map(|c| c.canonical()).collect::<Vec<_>>(),
        })
    }
}

/// `sha256:<hex>` канонического JSON (ключи у `serde_json::Map` упорядочены — результат стабилен).
pub fn fingerprint_of(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).expect("json value serializes");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

pub fn fingerprint(s: &SchemaSummary) -> String {
    fingerprint_of(&s.canonical())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> SchemaSummary {
        SchemaSummary::object(
            "body",
            vec![
                SchemaSummary::leaf("id", "string").required(true).with_example("A-1"),
                SchemaSummary::array("items", SchemaSummary::object("x", vec![SchemaSummary::leaf("sku", "string")])),
            ],
        )
    }

    #[test]
    fn fingerprint_ignores_examples_and_field_order() {
        let a = order();
        let mut b = order();
        b.children.reverse();
        b.children[1].example = None;
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut c = order();
        c.children[0].kind = "integer".into();
        assert_ne!(fingerprint(&a), fingerprint(&c));
        assert!(fingerprint(&a).starts_with("sha256:"));
    }

    #[test]
    fn array_items_keep_their_order_in_the_fingerprint() {
        let a = SchemaSummary { name: "v".into(), kind: "one_of".into(), children: vec![SchemaSummary::leaf("a", "string"), SchemaSummary::leaf("b", "integer")], ..Default::default() };
        let mut b = a.clone();
        b.children.reverse();
        assert_ne!(fingerprint(&a), fingerprint(&b), "варианты one_of не объект: порядок значим");
    }

    #[test]
    fn limit_cuts_depth_and_width() {
        let mut deep = SchemaSummary::leaf("l", "string");
        for i in 0..10 {
            deep = SchemaSummary::object(format!("o{i}"), vec![deep]);
        }
        deep.limit();
        let mut depth = 1;
        let mut node = &deep;
        while let Some(c) = node.children.first() {
            node = c;
            depth += 1;
        }
        assert_eq!(depth, MAX_DEPTH);
        assert!(node.truncated);

        let wide = SchemaSummary::object("w", (0..250).map(|i| SchemaSummary::leaf(format!("f{i}"), "string")).collect());
        let mut w = wide;
        w.limit();
        assert_eq!(w.children.len(), MAX_CHILDREN);
        assert!(w.truncated);
    }

    #[test]
    fn yaml_round_trip_skips_defaults() {
        let s = SchemaSummary::leaf("id", "string");
        let text = crate::to_yaml(&s);
        assert_eq!(text.trim(), "name: id\ntype: string");
        assert_eq!(crate::from_yaml::<SchemaSummary>(&text).unwrap(), s);
    }
}
```

Run: `cargo test -p glafira-load-model schema` — Expected: 4 PASS.

- [ ] **Step 6: Реестр** — `crates/glafira-load-model/src/registry.rs`:

```rust
//! Реестр операций `registry/operations.yaml` (спека L1 §6.1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::{SchemaSummary, fingerprint_of};

pub const REGISTRY_FILE: &str = "registry/operations.yaml";
pub const SCANS_DIR: &str = "registry/scans";
pub const FORMAT: u32 = 1;

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Http,
    KafkaProduce,
    KafkaConsume,
    Grpc,
    Sql,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OpKind::Http => "http",
            OpKind::KafkaProduce => "kafka_produce",
            OpKind::KafkaConsume => "kafka_consume",
            OpKind::Grpc => "grpc",
            OpKind::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Address {
    Http { method: String, path: String },
    Kafka { topic: String },
    Grpc { service: String, method: String },
    Sql { name: String },
}

/// Адрес одной строкой: для `id`, диффов и заголовков.
pub fn address_label(a: &Address) -> String {
    match a {
        Address::Http { method, path } => format!("{} {path}", method.to_ascii_uppercase()),
        Address::Kafka { topic } => topic.clone(),
        Address::Grpc { service, method } => format!("{service}/{method}"),
        Address::Sql { name } => name.clone(),
    }
}

/// Устойчивый `id` (Global Constraints). Пара вид/адрес, которой нет в правилах, даёт общий вид
/// `<kind>:<service>:<адрес>` — ошибка импортёра не должна ронять разбор.
pub fn operation_id(service: &str, kind: OpKind, address: &Address) -> String {
    match (kind, address) {
        (OpKind::Http, Address::Http { .. }) => format!("http:{service}:{}", address_label(address)),
        (OpKind::KafkaProduce, Address::Kafka { topic }) => format!("kafka_produce:{topic}"),
        (OpKind::KafkaConsume, Address::Kafka { topic }) => format!("kafka_consume:{topic}"),
        (OpKind::Grpc, Address::Grpc { .. }) => format!("grpc:{service}:{}", address_label(address)),
        (OpKind::Sql, Address::Sql { name }) => format!("sql:{service}:{name}"),
        (kind, address) => format!("{}:{service}:{}", kind.as_str(), address_label(address)),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Openapi,
    Proto,
    Asyncapi,
    Schema,
    Sql,
    Spring,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub kind: SourceKind,
    /// Относительно клона сервиса; у `manual` — пусто.
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl Source {
    pub fn manual() -> Self {
        Self { kind: SourceKind::Manual, path: String::new(), line: None }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Removed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Streaming {
    Client,
    Server,
    Bidi,
}

/// Версия сервиса, в которой операция замечена.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Seen {
    pub version: String,
    pub commit: String,
    /// RFC 3339, UTC.
    pub at: String,
}

/// Ручные поля: переживают переразбор.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Manual {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
}

impl Manual {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.notes.is_empty() && self.owner.is_empty()
    }
}

/// Операция из импортёра: всё, что известно из контракта. Остальное добавляет слияние.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationDraft {
    pub id: String,
    pub service: String,
    pub kind: OpKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub address: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<SchemaSummary>,
    /// Ответ 2xx у HTTP, ответ rpc у gRPC, сообщение у Kafka.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<SchemaSummary>,
    /// Остальные ответы HTTP по коду.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub responses: BTreeMap<String, SchemaSummary>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<Streaming>,
    /// Топик взят из имени файла схемы, а не из манифеста.
    #[serde(default, skip_serializing_if = "is_false")]
    pub topic_guessed: bool,
}

impl OperationDraft {
    /// Черновик с `id` по правилам и именем из адреса; импортёр уточняет имя и схемы.
    pub fn new(service: &str, kind: OpKind, address: Address, source: Source) -> Self {
        Self {
            id: operation_id(service, kind, &address),
            service: service.into(),
            kind,
            name: address_label(&address),
            summary: String::new(),
            address,
            request: None,
            response: None,
            responses: BTreeMap::new(),
            source,
            streaming: None,
            topic_guessed: false,
        }
    }

    /// Отпечаток: адрес, стриминг и канонические схемы. Имя, описание и источник не входят.
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&serde_json::json!({
            "a": self.address,
            "s": self.streaming,
            "q": self.request.as_ref().map(SchemaSummary::canonical),
            "p": self.response.as_ref().map(SchemaSummary::canonical),
            "r": self.responses.iter().map(|(k, v)| (k.clone(), v.canonical())).collect::<BTreeMap<_, _>>(),
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    #[serde(flatten)]
    pub draft: OperationDraft,
    pub fingerprint: String,
    pub status: Status,
    pub first_seen: Seen,
    pub last_seen: Seen,
    #[serde(default, skip_serializing_if = "Manual::is_empty")]
    pub manual: Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub format: u32,
    #[serde(default)]
    pub operations: Vec<Operation>,
}

impl Default for Registry {
    fn default() -> Self {
        Self { format: FORMAT, operations: Vec::new() }
    }
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&Operation> {
        self.operations.iter().find(|o| o.draft.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Operation> {
        self.operations.iter_mut().find(|o| o.draft.id == id)
    }

    /// Порядок файла — по сервису, затем по `id`: дифф в git читается.
    pub fn sort(&mut self) {
        self.operations.sort_by(|a, b| (a.draft.service.as_str(), a.draft.id.as_str()).cmp(&(b.draft.service.as_str(), b.draft.id.as_str())));
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.operations.len();
        self.operations.retain(|o| o.draft.id != id);
        before != self.operations.len()
    }

    /// Отпечатки активных операций: для снимка разбора.
    pub fn fingerprints(&self) -> BTreeMap<String, String> {
        self.operations.iter().filter(|o| o.status == Status::Active).map(|o| (o.draft.id.clone(), o.fingerprint.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen() -> Seen {
        Seen { version: "v1".into(), commit: "abc".into(), at: "2026-10-31T10:00:00Z".into() }
    }

    #[test]
    fn ids_follow_the_rules() {
        assert_eq!(operation_id("orders", OpKind::Http, &Address::Http { method: "post".into(), path: "/v1/orders".into() }), "http:orders:POST /v1/orders");
        assert_eq!(operation_id("orders", OpKind::KafkaProduce, &Address::Kafka { topic: "orders.created".into() }), "kafka_produce:orders.created");
        assert_eq!(operation_id("orders", OpKind::KafkaConsume, &Address::Kafka { topic: "orders.created".into() }), "kafka_consume:orders.created");
        assert_eq!(operation_id("orders", OpKind::Grpc, &Address::Grpc { service: "orders.v1.OrderService".into(), method: "Create".into() }), "grpc:orders:orders.v1.OrderService/Create");
        assert_eq!(operation_id("billing", OpKind::Sql, &Address::Sql { name: "invoices.insert".into() }), "sql:billing:invoices.insert");
        assert_eq!(operation_id("x", OpKind::Sql, &Address::Kafka { topic: "t".into() }), "sql:x:t", "несогласованная пара не паникует");
    }

    #[test]
    fn fingerprint_depends_on_schemas_and_address_only() {
        let mut a = OperationDraft::new("orders", OpKind::Http, Address::Http { method: "POST".into(), path: "/v1/orders".into() }, Source { kind: SourceKind::Openapi, path: "api.yaml".into(), line: Some(3) });
        a.request = Some(SchemaSummary::leaf("body", "object"));
        let mut b = a.clone();
        b.name = "createOrder".into();
        b.summary = "Create".into();
        b.source.line = Some(99);
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.request = Some(SchemaSummary::leaf("body", "string"));
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn registry_round_trips_through_yaml_flat() {
        let draft = OperationDraft::new("orders", OpKind::Http, Address::Http { method: "GET".into(), path: "/v1/orders/{id}".into() }, Source { kind: SourceKind::Openapi, path: "api.yaml".into(), line: None });
        let fp = draft.fingerprint();
        let mut r = Registry::default();
        r.operations.push(Operation { draft, fingerprint: fp, status: Status::Active, first_seen: seen(), last_seen: seen(), manual: Manual { tags: vec!["critical".into()], ..Default::default() } });
        let text = crate::to_yaml(&r);
        assert!(text.contains("- id: 'http:orders:GET /v1/orders/{id}'") || text.contains("- id: \"http:orders:GET /v1/orders/{id}\""), "{text}");
        assert!(text.contains("\n    address:\n      type: http\n"), "поля черновика лежат плоско: {text}");
        assert!(!text.contains("summary:"), "пустые поля не пишутся");
        let again: Registry = crate::from_yaml(&text).unwrap();
        assert_eq!(again, r);
        assert_eq!(again.fingerprints().len(), 1);
    }

    #[test]
    fn sort_is_by_service_then_id() {
        let mk = |service: &str, path: &str| {
            let d = OperationDraft::new(service, OpKind::Http, Address::Http { method: "GET".into(), path: path.into() }, Source::manual());
            Operation { fingerprint: d.fingerprint(), draft: d, status: Status::Active, first_seen: seen(), last_seen: seen(), manual: Manual::default() }
        };
        let mut r = Registry { format: FORMAT, operations: vec![mk("b", "/z"), mk("a", "/y"), mk("b", "/a")] };
        r.sort();
        assert_eq!(r.operations.iter().map(|o| o.draft.id.as_str()).collect::<Vec<_>>(), ["http:a:GET /y", "http:b:GET /a", "http:b:GET /z"]);
    }
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p glafira-load-model`
Expected: все тесты трёх модулей PASS. `cargo clippy -p glafira-load-model --all-targets -- -D warnings` без замечаний.

- [ ] **Step 8: Commit**

```bash
git add crates/glafira-load-model
git commit -m "feat(model): манифест проекта, сжатая схема с отпечатком и реестр операций

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Слияние реестра, снимки разборов и дифф

**Files:**
- Create: `crates/glafira-load-model/src/{merge,diff}.rs`
- Modify: `crates/glafira-load-model/src/lib.rs` (добавить `pub mod diff; pub mod merge;`)
- Test: unit-тесты в тех же файлах

**Interfaces:**
- Consumes: `registry::{Registry, Operation, OperationDraft, Seen, Status, SourceKind, address_label}`, `schema::SchemaSummary`.
- Produces: `merge::{merge, MergeSummary, ChangedOperation}`; `diff::{FieldChange, schema_diff, Scan, RegistryDiff, registry_diff, SCAN_FORMAT}`.

- [ ] **Step 1: `diff.rs`**

```rust
//! Дифф сжатых схем и снимков разбора (спека L1 §6.2–§6.3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::merge::MergeSummary;
use crate::schema::SchemaSummary;

pub const SCAN_FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum FieldChange {
    Added { path: String, kind: String },
    Removed { path: String, kind: String },
    TypeChanged { path: String, from: String, to: String },
    RequiredChanged { path: String, required: bool },
    AddressChanged { from: String, to: String },
}

/// Изменения между двумя схемами; `prefix` — `request`, `response`, `responses.404`.
pub fn schema_diff(prefix: &str, old: Option<&SchemaSummary>, new: Option<&SchemaSummary>) -> Vec<FieldChange> {
    let mut out = Vec::new();
    match (old, new) {
        (None, None) => {}
        (None, Some(n)) => out.push(FieldChange::Added { path: prefix.into(), kind: n.kind.clone() }),
        (Some(o), None) => out.push(FieldChange::Removed { path: prefix.into(), kind: o.kind.clone() }),
        (Some(o), Some(n)) => diff_node(prefix, o, n, &mut out),
    }
    out
}

fn kind_label(s: &SchemaSummary) -> String {
    if s.enum_values.is_empty() { s.kind.clone() } else { format!("{}[{}]", s.kind, s.enum_values.join("|")) }
}

fn diff_node(path: &str, o: &SchemaSummary, n: &SchemaSummary, out: &mut Vec<FieldChange>) {
    if o.kind != n.kind || o.enum_values != n.enum_values {
        out.push(FieldChange::TypeChanged { path: path.into(), from: kind_label(o), to: kind_label(n) });
        if o.kind != n.kind {
            return;
        }
    }
    if o.required != n.required {
        out.push(FieldChange::RequiredChanged { path: path.into(), required: n.required });
    }
    let old: BTreeMap<&str, &SchemaSummary> = o.children.iter().map(|c| (c.name.as_str(), c)).collect();
    let new: BTreeMap<&str, &SchemaSummary> = n.children.iter().map(|c| (c.name.as_str(), c)).collect();
    for (name, c) in &new {
        let child = format!("{path}.{name}");
        match old.get(name) {
            Some(oc) => diff_node(&child, oc, c, out),
            None => out.push(FieldChange::Added { path: child, kind: c.kind.clone() }),
        }
    }
    for (name, c) in &old {
        if !new.contains_key(name) {
            out.push(FieldChange::Removed { path: format!("{path}.{name}"), kind: c.kind.clone() });
        }
    }
}

/// Снимок разбора `registry/scans/<метка>.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scan {
    pub format: u32,
    /// RFC 3339, UTC.
    pub at: String,
    pub service: String,
    pub version: String,
    pub commit: String,
    pub summary: MergeSummary,
    /// Отпечатки всех активных операций реестра после слияния (всех сервисов).
    pub fingerprints: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RegistryDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

/// Что изменилось между двумя снимками по отпечаткам.
pub fn registry_diff(from: &Scan, to: &Scan) -> RegistryDiff {
    let mut d = RegistryDiff::default();
    for (id, fp) in &to.fingerprints {
        match from.fingerprints.get(id) {
            None => d.added.push(id.clone()),
            Some(old) if old != fp => d.changed.push(id.clone()),
            Some(_) => {}
        }
    }
    d.removed = from.fingerprints.keys().filter(|id| !to.fingerprints.contains_key(*id)).cloned().collect();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_diff_reports_added_removed_type_and_required() {
        let old = SchemaSummary::object("body", vec![
            SchemaSummary::leaf("id", "string").required(true),
            SchemaSummary::leaf("qty", "integer"),
            SchemaSummary::array("tags", SchemaSummary::leaf("x", "string")),
        ]);
        let new = SchemaSummary::object("body", vec![
            SchemaSummary::leaf("id", "string"),
            SchemaSummary::leaf("qty", "number"),
            SchemaSummary::array("tags", SchemaSummary::leaf("x", "integer")),
            SchemaSummary::leaf("note", "string"),
        ]);
        let d = schema_diff("request", Some(&old), Some(&new));
        assert_eq!(d, vec![
            FieldChange::RequiredChanged { path: "request.id".into(), required: false },
            FieldChange::Added { path: "request.note".into(), kind: "string".into() },
            FieldChange::TypeChanged { path: "request.qty".into(), from: "integer".into(), to: "number".into() },
            FieldChange::TypeChanged { path: "request.tags.items".into(), from: "string".into(), to: "integer".into() },
        ]);
        assert_eq!(schema_diff("response", None, Some(&new)), vec![FieldChange::Added { path: "response".into(), kind: "object".into() }]);
        assert_eq!(schema_diff("response", Some(&old), None), vec![FieldChange::Removed { path: "response".into(), kind: "object".into() }]);
    }

    #[test]
    fn registry_diff_compares_fingerprints() {
        let scan = |pairs: &[(&str, &str)]| Scan {
            format: SCAN_FORMAT, at: String::new(), service: "s".into(), version: "v".into(), commit: "c".into(),
            summary: MergeSummary::default(),
            fingerprints: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        };
        let d = registry_diff(&scan(&[("a", "1"), ("b", "1"), ("c", "1")]), &scan(&[("a", "1"), ("b", "2"), ("d", "1")]));
        assert_eq!(d, RegistryDiff { added: vec!["d".into()], removed: vec!["c".into()], changed: vec!["b".into()] });
    }
}
```

- [ ] **Step 2: `merge.rs`**

```rust
//! Слияние результата разбора с реестром (спека L1 §6.2).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::diff::{FieldChange, schema_diff};
use crate::registry::{Manual, Operation, OperationDraft, Registry, Seen, SourceKind, Status, address_label};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangedOperation {
    pub id: String,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MergeSummary {
    pub added: Vec<String>,
    pub changed: Vec<ChangedOperation>,
    pub removed: Vec<String>,
    pub restored: Vec<String>,
    pub unchanged: usize,
}

impl MergeSummary {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty() && self.restored.is_empty()
    }
}

/// Сливает черновики одного сервиса в реестр. Черновик чужого сервиса и повтор `id` в одном
/// разборе пропускаются (первый побеждает). Идемпотентно: повторное слияние тех же черновиков
/// даёт пустую сводку и тот же файл.
pub fn merge(registry: &mut Registry, service: &str, drafts: Vec<OperationDraft>, seen: &Seen) -> MergeSummary {
    let mut summary = MergeSummary::default();
    let mut ids = BTreeSet::new();
    for draft in drafts {
        if draft.service != service || !ids.insert(draft.id.clone()) {
            continue;
        }
        let id = draft.id.clone();
        let fp = draft.fingerprint();
        match registry.get_mut(&id) {
            Some(op) => {
                let was_removed = op.status == Status::Removed;
                let changed = op.fingerprint != fp;
                let mut changes = Vec::new();
                if changed {
                    changes.extend(schema_diff("request", op.draft.request.as_ref(), draft.request.as_ref()));
                    changes.extend(schema_diff("response", op.draft.response.as_ref(), draft.response.as_ref()));
                    let codes: BTreeSet<&String> = op.draft.responses.keys().chain(draft.responses.keys()).collect();
                    for code in codes {
                        changes.extend(schema_diff(&format!("responses.{code}"), op.draft.responses.get(code), draft.responses.get(code)));
                    }
                    if op.draft.address != draft.address {
                        changes.push(FieldChange::AddressChanged { from: address_label(&op.draft.address), to: address_label(&draft.address) });
                    }
                }
                op.draft = draft;
                op.fingerprint = fp;
                op.last_seen = seen.clone();
                op.status = Status::Active;
                if was_removed {
                    summary.restored.push(id);
                } else if changed {
                    summary.changed.push(ChangedOperation { id, changes });
                } else {
                    summary.unchanged += 1;
                }
            }
            None => {
                summary.added.push(id);
                registry.operations.push(Operation {
                    draft,
                    fingerprint: fp,
                    status: Status::Active,
                    first_seen: seen.clone(),
                    last_seen: seen.clone(),
                    manual: Manual::default(),
                });
            }
        }
    }
    for op in registry.operations.iter_mut().filter(|o| {
        o.draft.service == service && o.status == Status::Active && o.draft.source.kind != SourceKind::Manual && !ids.contains(&o.draft.id)
    }) {
        op.status = Status::Removed;
        summary.removed.push(op.draft.id.clone());
    }
    registry.sort();
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Address, OpKind, Source};
    use crate::schema::SchemaSummary;

    fn seen(v: &str) -> Seen {
        Seen { version: v.into(), commit: format!("c-{v}"), at: "2026-10-31T10:00:00Z".into() }
    }

    fn http(path: &str, body: Option<SchemaSummary>) -> OperationDraft {
        let mut d = OperationDraft::new("orders", OpKind::Http, Address::Http { method: "POST".into(), path: path.into() }, Source { kind: SourceKind::Openapi, path: "api.yaml".into(), line: Some(1) });
        d.request = body;
        d
    }

    #[test]
    fn add_change_remove_restore_and_manual_fields_survive() {
        let mut r = Registry::default();
        let s1 = merge(&mut r, "orders", vec![http("/a", None), http("/b", None)], &seen("v1"));
        assert_eq!(s1.added, vec!["http:orders:POST /a", "http:orders:POST /b"]);
        r.get_mut("http:orders:POST /a").unwrap().manual.notes = "keep me".into();

        let s2 = merge(&mut r, "orders", vec![http("/a", Some(SchemaSummary::leaf("body", "object")))], &seen("v2"));
        assert_eq!(s2.changed[0].id, "http:orders:POST /a");
        assert_eq!(s2.changed[0].changes, vec![FieldChange::Added { path: "request".into(), kind: "object".into() }]);
        assert_eq!(s2.removed, vec!["http:orders:POST /b"]);
        let a = r.get("http:orders:POST /a").unwrap();
        assert_eq!(a.manual.notes, "keep me");
        assert_eq!((a.first_seen.version.as_str(), a.last_seen.version.as_str()), ("v1", "v2"));
        let b = r.get("http:orders:POST /b").unwrap();
        assert_eq!((b.status, b.last_seen.version.as_str()), (Status::Removed, "v1"));

        let s3 = merge(&mut r, "orders", vec![http("/a", Some(SchemaSummary::leaf("body", "object"))), http("/b", None)], &seen("v3"));
        assert_eq!(s3.restored, vec!["http:orders:POST /b"]);
        assert_eq!(s3.unchanged, 1);
        assert_eq!(r.get("http:orders:POST /b").unwrap().status, Status::Active);
    }

    #[test]
    fn manual_and_foreign_operations_are_not_removed() {
        let mut r = Registry::default();
        merge(&mut r, "billing", vec![{
            let mut d = http("/pay", None);
            d.service = "billing".into();
            d.id = "http:billing:POST /pay".into();
            d
        }], &seen("v1"));
        let mut manual = OperationDraft::new("orders", OpKind::Sql, Address::Sql { name: "x".into() }, Source::manual());
        manual.name = "x".into();
        r.operations.push(Operation { fingerprint: manual.fingerprint(), draft: manual, status: Status::Active, first_seen: seen("v1"), last_seen: seen("v1"), manual: Manual::default() });
        let s = merge(&mut r, "orders", vec![], &seen("v2"));
        assert!(s.removed.is_empty());
        assert_eq!(r.operations.len(), 2);
        assert!(r.operations.iter().all(|o| o.status == Status::Active));
    }

    #[test]
    fn merge_is_idempotent_and_sorted() {
        let mut r = Registry::default();
        merge(&mut r, "orders", vec![http("/z", None), http("/a", None)], &seen("v1"));
        let text1 = crate::to_yaml(&r);
        let s = merge(&mut r, "orders", vec![http("/a", None), http("/z", None), http("/a", None)], &seen("v1"));
        assert!(s.is_empty());
        assert_eq!(s.unchanged, 2, "дубликат в разборе не считается");
        assert_eq!(crate::to_yaml(&r), text1);
        assert_eq!(r.operations[0].draft.id, "http:orders:POST /a");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p glafira-load-model`
Expected: тесты `diff` и `merge` PASS; `cargo clippy -p glafira-load-model --all-targets -- -D warnings` чисто.

- [ ] **Step 4: Commit**

```bash
git add crates/glafira-load-model
git commit -m "feat(model): слияние реестра с диффом схем, снимки разборов и их сравнение

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Нагрузка ПРОМ, предпросмотр импорта и расчёт профиля

**Files:**
- Create: `crates/glafira-load-model/src/{prod_load,profile}.rs`
- Modify: `crates/glafira-load-model/src/lib.rs` (добавить `pub mod prod_load; pub mod profile;`)
- Test: unit-тесты в тех же файлах

**Interfaces:**
- Consumes: `registry::{Registry, OpKind, Status}`.
- Produces: `prod_load::{ProdLoad, SourceInfo, Figures, Cell, TemplateRow, InvalidCell, InvalidReason, FiguresDiff, ImportPreview, preview_import, PROD_LOAD_FILE, IMPORTS_DIR}`; `profile::{ProfileFile, Coefficients, Overrides, Override, Basis, Phase, Profile, Target, Point, Computed, ProfileError, compute, timeline, PROFILE_FILE}`.

- [ ] **Step 1: `prod_load.rs`**

```rust
//! Цифры нагрузки ПРОМ `load/prod-load.yaml` и предпросмотр импорта шаблона (спека L1 §7.2–§7.3,
//! решение плана 3: у всех операций `avg` и `peak`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::registry::OpKind;

pub const PROD_LOAD_FILE: &str = "load/prod-load.yaml";
pub const IMPORTS_DIR: &str = "load/imports";
pub const FORMAT: u32 = 1;

/// Лист «Параметры» шаблона.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceInfo {
    #[serde(default)]
    pub stand: String,
    #[serde(default)]
    pub period_from: String,
    #[serde(default)]
    pub period_to: String,
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub filled_by: String,
    /// RFC 3339, UTC.
    #[serde(default)]
    pub imported_at: String,
    /// Копия шаблона относительно проекта: `load/imports/<метка>.xlsx`.
    #[serde(default)]
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Figures {
    /// Среднее RPS (сообщений/с у `kafka_consume`).
    pub avg: f64,
    pub peak: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub req_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resp_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,
    /// Только `kafka_consume`: допустимый лаг, с.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag_max_s: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProdLoad {
    pub format: u32,
    #[serde(default)]
    pub source: SourceInfo,
    #[serde(default)]
    pub operations: BTreeMap<String, Figures>,
}

impl Default for ProdLoad {
    fn default() -> Self {
        Self { format: FORMAT, source: SourceInfo::default(), operations: BTreeMap::new() }
    }
}

/// Ячейка из шаблона: пустая, число или текст, который числом не является.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cell {
    Empty,
    Number { value: f64 },
    Text { value: String },
}

impl Cell {
    pub fn is_empty(&self) -> bool {
        matches!(self, Cell::Empty)
    }
}

/// Строка листа «Операции» как её прочитал `glafira-load-xlsx`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateRow {
    /// Номер строки в листе, с 1.
    pub row: u32,
    pub id: String,
    pub avg: Cell,
    pub peak: Cell,
    pub error_pct: Cell,
    pub req_bytes: Cell,
    pub resp_bytes: Cell,
    /// p95 (мс) у всех, кроме `kafka_consume`, где это допустимый лаг (с).
    pub p95_ms: Cell,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvalidReason {
    NotANumber,
    Negative,
    NotAnInteger,
    PercentOutOfRange,
    PeakBelowAvg,
    AvgMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvalidCell {
    pub row: u32,
    pub id: String,
    /// `avg`, `peak`, `error_pct`, `req_bytes`, `resp_bytes`, `p95_ms`.
    pub column: String,
    pub value: String,
    pub reason: InvalidReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FiguresDiff {
    pub id: String,
    pub old: Option<Figures>,
    pub new: Figures,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportPreview {
    pub filled: Vec<String>,
    pub empty: Vec<String>,
    pub unknown_ids: Vec<String>,
    pub invalid: Vec<InvalidCell>,
    pub diff: Vec<FiguresDiff>,
    pub source: SourceInfo,
    /// Будущий `prod-load.yaml`; `None`, пока есть `invalid`.
    pub result: Option<ProdLoad>,
}

fn number(cell: &Cell, column: &str, row: &TemplateRow, invalid: &mut Vec<InvalidCell>) -> Option<f64> {
    let bad = |value: String, reason| InvalidCell { row: row.row, id: row.id.clone(), column: column.into(), value, reason };
    match cell {
        Cell::Empty => None,
        Cell::Text { value } => {
            invalid.push(bad(value.clone(), InvalidReason::NotANumber));
            None
        }
        Cell::Number { value } if *value < 0.0 => {
            invalid.push(bad(value.to_string(), InvalidReason::Negative));
            None
        }
        Cell::Number { value } => Some(*value),
    }
}

fn integer(cell: &Cell, column: &str, row: &TemplateRow, invalid: &mut Vec<InvalidCell>) -> Option<u64> {
    let v = number(cell, column, row, invalid)?;
    if v.fract() != 0.0 {
        invalid.push(InvalidCell { row: row.row, id: row.id.clone(), column: column.into(), value: v.to_string(), reason: InvalidReason::NotAnInteger });
        return None;
    }
    Some(v as u64)
}

/// Предпросмотр импорта. Пустые строки пропускаются (прежние цифры остаются), неизвестные `id`
/// пропускаются с предупреждением, строки с ошибками блокируют результат.
pub fn preview_import(rows: &[TemplateRow], source: SourceInfo, known: &BTreeMap<String, OpKind>, current: Option<&ProdLoad>) -> ImportPreview {
    let mut p = ImportPreview { filled: vec![], empty: vec![], unknown_ids: vec![], invalid: vec![], diff: vec![], source: source.clone(), result: None };
    let mut result = current.cloned().unwrap_or_default();
    result.source = source;
    for row in rows {
        let Some(kind) = known.get(&row.id) else {
            p.unknown_ids.push(row.id.clone());
            continue;
        };
        if [&row.avg, &row.peak, &row.error_pct, &row.req_bytes, &row.resp_bytes, &row.p95_ms].iter().all(|c| c.is_empty()) {
            p.empty.push(row.id.clone());
            continue;
        }
        let before = p.invalid.len();
        let avg = number(&row.avg, "avg", row, &mut p.invalid);
        let peak = number(&row.peak, "peak", row, &mut p.invalid);
        let error_pct = number(&row.error_pct, "error_pct", row, &mut p.invalid);
        if let Some(e) = error_pct
            && e > 100.0
        {
            p.invalid.push(InvalidCell { row: row.row, id: row.id.clone(), column: "error_pct".into(), value: e.to_string(), reason: InvalidReason::PercentOutOfRange });
        }
        let req_bytes = integer(&row.req_bytes, "req_bytes", row, &mut p.invalid);
        let resp_bytes = integer(&row.resp_bytes, "resp_bytes", row, &mut p.invalid);
        let p95 = number(&row.p95_ms, "p95_ms", row, &mut p.invalid);
        let Some(avg) = avg else {
            if row.avg.is_empty() {
                p.invalid.push(InvalidCell { row: row.row, id: row.id.clone(), column: "avg".into(), value: String::new(), reason: InvalidReason::AvgMissing });
            }
            continue;
        };
        let peak = peak.unwrap_or(avg);
        if peak < avg {
            p.invalid.push(InvalidCell { row: row.row, id: row.id.clone(), column: "peak".into(), value: peak.to_string(), reason: InvalidReason::PeakBelowAvg });
        }
        if p.invalid.len() != before {
            continue;
        }
        let consume = *kind == OpKind::KafkaConsume;
        let figures = Figures {
            avg,
            peak,
            error_pct,
            req_bytes,
            resp_bytes,
            p95_ms: if consume { None } else { p95 },
            lag_max_s: if consume { p95 } else { None },
            comment: row.comment.trim().to_string(),
        };
        let old = current.and_then(|c| c.operations.get(&row.id)).cloned();
        if old.as_ref() != Some(&figures) {
            p.diff.push(FiguresDiff { id: row.id.clone(), old, new: figures.clone() });
        }
        result.operations.insert(row.id.clone(), figures);
        p.filled.push(row.id.clone());
    }
    if p.invalid.is_empty() {
        p.result = Some(result);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u32, id: &str, avg: Cell, peak: Cell) -> TemplateRow {
        TemplateRow { row: n, id: id.into(), avg, peak, error_pct: Cell::Empty, req_bytes: Cell::Empty, resp_bytes: Cell::Empty, p95_ms: Cell::Empty, comment: String::new() }
    }
    fn num(v: f64) -> Cell {
        Cell::Number { value: v }
    }
    fn known() -> BTreeMap<String, OpKind> {
        [("http:o:POST /a", OpKind::Http), ("kafka_consume:t", OpKind::KafkaConsume)].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn classifies_rows_and_keeps_previous_figures_for_empty_ones() {
        let mut current = ProdLoad::default();
        current.operations.insert("kafka_consume:t".into(), Figures { avg: 5.0, peak: 6.0, ..Default::default() });
        let mut consume = row(3, "kafka_consume:t", Cell::Empty, Cell::Empty);
        consume.p95_ms = Cell::Empty;
        let rows = vec![
            row(2, "http:o:POST /a", num(120.0), num(340.0)),
            consume,
            row(4, "http:o:GET /old", num(1.0), Cell::Empty),
        ];
        let p = preview_import(&rows, SourceInfo { stand: "prod".into(), ..Default::default() }, &known(), Some(&current));
        assert_eq!(p.filled, vec!["http:o:POST /a"]);
        assert_eq!(p.empty, vec!["kafka_consume:t"]);
        assert_eq!(p.unknown_ids, vec!["http:o:GET /old"]);
        assert!(p.invalid.is_empty());
        assert_eq!(p.diff.len(), 1);
        assert_eq!(p.diff[0].old, None);
        let r = p.result.unwrap();
        assert_eq!(r.operations["kafka_consume:t"].avg, 5.0, "пустая строка не стирает прежние цифры");
        assert_eq!(r.operations["http:o:POST /a"].peak, 340.0);
        assert_eq!(r.source.stand, "prod");
    }

    #[test]
    fn invalid_cells_block_the_result() {
        let mut r1 = row(2, "http:o:POST /a", Cell::Text { value: "many".into() }, num(1.0));
        r1.error_pct = num(150.0);
        r1.req_bytes = num(1.5);
        let r2 = row(3, "kafka_consume:t", num(10.0), num(5.0));
        let mut r3 = row(4, "http:o:POST /a", Cell::Empty, num(5.0));
        r3.row = 4;
        let p = preview_import(&[r1, r2, r3], SourceInfo::default(), &known(), None);
        let reasons: Vec<(u32, &str, InvalidReason)> = p.invalid.iter().map(|i| (i.row, i.column.as_str(), i.reason)).collect();
        assert_eq!(reasons, vec![
            (2, "avg", InvalidReason::NotANumber),
            (2, "error_pct", InvalidReason::PercentOutOfRange),
            (2, "req_bytes", InvalidReason::NotAnInteger),
            (3, "peak", InvalidReason::PeakBelowAvg),
            (4, "avg", InvalidReason::AvgMissing),
        ]);
        assert!(p.result.is_none());
        assert!(p.filled.is_empty());
    }

    #[test]
    fn consume_rows_put_the_last_column_into_lag() {
        let mut r = row(2, "kafka_consume:t", num(10.0), Cell::Empty);
        r.p95_ms = num(5.0);
        let p = preview_import(&[r], SourceInfo::default(), &known(), None);
        let f = &p.result.unwrap().operations["kafka_consume:t"];
        assert_eq!((f.peak, f.lag_max_s, f.p95_ms), (10.0, Some(5.0), None));
    }

    #[test]
    fn yaml_round_trip() {
        let mut pl = ProdLoad::default();
        pl.operations.insert("http:o:POST /a".into(), Figures { avg: 120.0, peak: 340.0, error_pct: Some(0.2), req_bytes: Some(850), resp_bytes: None, p95_ms: Some(180.0), lag_max_s: None, comment: "x".into() });
        let text = crate::to_yaml(&pl);
        assert!(!text.contains("resp_bytes"), "{text}");
        assert_eq!(crate::from_yaml::<ProdLoad>(&text).unwrap(), pl);
    }
}
```

- [ ] **Step 2: `profile.rs`**

```rust
//! Профиль нагрузки `load/profile.yaml` и его расчёт (спека L1 §8).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::prod_load::ProdLoad;
use crate::registry::{OpKind, Registry, Status};

pub const PROFILE_FILE: &str = "load/profile.yaml";
pub const FORMAT: u32 = 1;

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_env: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_growth: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, Override>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, Override>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Coefficients {
    /// Мощность стенда относительно ПРОМ.
    #[serde(default = "one")]
    pub k_env: f64,
    /// Запас роста.
    #[serde(default = "one")]
    pub k_growth: f64,
    #[serde(default)]
    pub overrides: Overrides,
}

impl Default for Coefficients {
    fn default() -> Self {
        Self { k_env: 1.0, k_growth: 1.0, overrides: Overrides::default() }
    }
}

impl Coefficients {
    /// Коэффициенты операции: переопределение операции, иначе сервиса, иначе общие.
    pub fn for_operation(&self, service: &str, id: &str) -> (f64, f64) {
        let op = self.overrides.operations.get(id);
        let svc = self.overrides.services.get(service);
        let pick = |f: fn(&Override) -> Option<f64>, base: f64| op.and_then(f).or_else(|| svc.and_then(f)).unwrap_or(base);
        (pick(|o| o.k_env, self.k_env), pick(|o| o.k_growth, self.k_growth))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Avg,
    Peak,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Phase {
    /// Линейно от текущего уровня к `to_pct` за `minutes`.
    Ramp { minutes: f64, to_pct: f64 },
    Steady { minutes: f64 },
    /// Ступени `step_pct`, 2·`step_pct`, … до `max_pct` включительно, каждая по `step_minutes`.
    Steps { step_pct: f64, step_minutes: f64, max_pct: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub basis: Basis,
    #[serde(default)]
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileFile {
    pub format: u32,
    #[serde(default)]
    pub coefficients: Coefficients,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for ProfileFile {
    /// Три профиля из спеки §8.1: nominal, peak, stress.
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert("nominal".into(), Profile { basis: Basis::Avg, phases: vec![Phase::Ramp { minutes: 10.0, to_pct: 100.0 }, Phase::Steady { minutes: 60.0 }, Phase::Ramp { minutes: 5.0, to_pct: 0.0 }] });
        profiles.insert("peak".into(), Profile { basis: Basis::Peak, phases: vec![Phase::Ramp { minutes: 5.0, to_pct: 100.0 }, Phase::Steady { minutes: 30.0 }] });
        profiles.insert("stress".into(), Profile { basis: Basis::Peak, phases: vec![Phase::Steps { step_pct: 25.0, step_minutes: 10.0, max_pct: 300.0 }] });
        Self { format: FORMAT, coefficients: Coefficients::default(), profiles }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub id: String,
    pub service: String,
    pub kind: OpKind,
    pub basis_value: f64,
    pub k_env: f64,
    pub k_growth: f64,
    pub target: f64,
    pub share: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub minute: f64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Computed {
    pub profile: String,
    pub basis: Basis,
    pub targets: Vec<Target>,
    pub total: f64,
    /// Активные операции без цифр ПРОМ.
    pub uncovered: Vec<String>,
    pub timeline: Vec<Point>,
    pub total_minutes: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("profile not found: {0}")]
    UnknownProfile(String),
}

/// Ряд «минута → % цели». Начинается с (0, 0); у ступеней скачок — две точки с одной минутой.
pub fn timeline(phases: &[Phase]) -> Vec<Point> {
    let mut out = vec![Point { minute: 0.0, pct: 0.0 }];
    let (mut t, mut pct) = (0.0, 0.0);
    for phase in phases {
        match *phase {
            Phase::Ramp { minutes, to_pct } => {
                t += minutes;
                pct = to_pct;
                out.push(Point { minute: t, pct });
            }
            Phase::Steady { minutes } => {
                t += minutes;
                out.push(Point { minute: t, pct });
            }
            Phase::Steps { step_pct, step_minutes, max_pct } => {
                if step_pct <= 0.0 || step_minutes <= 0.0 {
                    continue;
                }
                let mut level = step_pct;
                while level <= max_pct + 1e-9 {
                    out.push(Point { minute: t, pct: level });
                    t += step_minutes;
                    out.push(Point { minute: t, pct: level });
                    level += step_pct;
                }
                pct = out.last().map(|p| p.pct).unwrap_or(pct);
            }
        }
    }
    out
}

pub fn compute(file: &ProfileFile, name: &str, registry: &Registry, prod: &ProdLoad) -> Result<Computed, ProfileError> {
    let profile = file.profiles.get(name).ok_or_else(|| ProfileError::UnknownProfile(name.into()))?;
    let mut targets = Vec::new();
    let mut uncovered = Vec::new();
    for op in registry.operations.iter().filter(|o| o.status == Status::Active) {
        let Some(f) = prod.operations.get(&op.draft.id) else {
            uncovered.push(op.draft.id.clone());
            continue;
        };
        let basis_value = match profile.basis {
            Basis::Avg => f.avg,
            Basis::Peak => f.peak,
        };
        let (k_env, k_growth) = file.coefficients.for_operation(&op.draft.service, &op.draft.id);
        targets.push(Target { id: op.draft.id.clone(), service: op.draft.service.clone(), kind: op.draft.kind, basis_value, k_env, k_growth, target: basis_value * k_env * k_growth, share: 0.0 });
    }
    let total: f64 = targets.iter().map(|t| t.target).sum();
    if total > 0.0 {
        for t in &mut targets {
            t.share = t.target / total;
        }
    }
    let timeline = timeline(&profile.phases);
    let total_minutes = timeline.last().map(|p| p.minute).unwrap_or(0.0);
    Ok(Computed { profile: name.into(), basis: profile.basis, targets, total, uncovered, timeline, total_minutes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::merge;
    use crate::prod_load::Figures;
    use crate::registry::{Address, OperationDraft, Seen, Source};

    fn registry() -> Registry {
        let mut r = Registry::default();
        let seen = Seen::default();
        let mk = |svc: &str, path: &str| OperationDraft::new(svc, OpKind::Http, Address::Http { method: "GET".into(), path: path.into() }, Source::manual());
        merge(&mut r, "orders", vec![mk("orders", "/a"), mk("orders", "/b"), mk("orders", "/none")], &seen);
        merge(&mut r, "billing", vec![mk("billing", "/pay")], &seen);
        r
    }

    fn prod() -> ProdLoad {
        let mut p = ProdLoad::default();
        for (id, avg, peak) in [("http:orders:GET /a", 100.0, 300.0), ("http:orders:GET /b", 50.0, 60.0), ("http:billing:GET /pay", 50.0, 100.0)] {
            p.operations.insert(id.into(), Figures { avg, peak, ..Default::default() });
        }
        p
    }

    #[test]
    fn targets_use_overrides_and_shares_sum_to_one() {
        let mut file = ProfileFile::default();
        file.coefficients.k_env = 0.5;
        file.coefficients.k_growth = 2.0;
        file.coefficients.overrides.services.insert("billing".into(), Override { k_env: Some(0.1), k_growth: None });
        file.coefficients.overrides.operations.insert("http:orders:GET /b".into(), Override { k_env: None, k_growth: Some(4.0) });
        let c = compute(&file, "nominal", &registry(), &prod()).unwrap();
        let by: BTreeMap<&str, &Target> = c.targets.iter().map(|t| (t.id.as_str(), t)).collect();
        assert_eq!(by["http:orders:GET /a"].target, 100.0);
        assert_eq!(by["http:orders:GET /b"].target, 50.0 * 0.5 * 4.0);
        assert_eq!(by["http:billing:GET /pay"].target, 50.0 * 0.1 * 2.0);
        assert!((c.targets.iter().map(|t| t.share).sum::<f64>() - 1.0).abs() < 1e-9);
        assert_eq!(c.uncovered, vec!["http:orders:GET /none"]);
        assert_eq!(c.total_minutes, 75.0);
        let peak = compute(&file, "peak", &registry(), &prod()).unwrap();
        assert_eq!(peak.targets.iter().find(|t| t.id == "http:orders:GET /a").unwrap().basis_value, 300.0);
    }

    #[test]
    fn raising_any_coefficient_never_lowers_a_target() {
        let base = compute(&ProfileFile::default(), "nominal", &registry(), &prod()).unwrap();
        for k in [1.5, 3.0] {
            let mut f = ProfileFile::default();
            f.coefficients.k_growth = k;
            let c = compute(&f, "nominal", &registry(), &prod()).unwrap();
            for (a, b) in base.targets.iter().zip(&c.targets) {
                assert!(b.target >= a.target, "{}: {} < {}", a.id, b.target, a.target);
            }
        }
    }

    #[test]
    fn timeline_is_continuous_at_phase_boundaries() {
        let pts = timeline(&[Phase::Ramp { minutes: 10.0, to_pct: 100.0 }, Phase::Steady { minutes: 20.0 }, Phase::Steps { step_pct: 50.0, step_minutes: 5.0, max_pct: 150.0 }, Phase::Ramp { minutes: 5.0, to_pct: 0.0 }]);
        let m: Vec<(f64, f64)> = pts.iter().map(|p| (p.minute, p.pct)).collect();
        assert_eq!(m, vec![(0.0, 0.0), (10.0, 100.0), (30.0, 100.0), (30.0, 50.0), (35.0, 50.0), (35.0, 100.0), (40.0, 100.0), (40.0, 150.0), (45.0, 150.0), (50.0, 0.0)]);
        assert!(pts.windows(2).all(|w| w[1].minute >= w[0].minute));
        assert_eq!(timeline(&[]), vec![Point { minute: 0.0, pct: 0.0 }]);
    }

    #[test]
    fn unknown_profile_and_empty_prod() {
        assert_eq!(compute(&ProfileFile::default(), "nope", &registry(), &prod()).unwrap_err(), ProfileError::UnknownProfile("nope".into()));
        let c = compute(&ProfileFile::default(), "nominal", &registry(), &ProdLoad::default()).unwrap();
        assert_eq!((c.targets.len(), c.uncovered.len(), c.total), (0, 4, 0.0));
    }

    #[test]
    fn default_file_round_trips() {
        let f = ProfileFile::default();
        let text = crate::to_yaml(&f);
        assert!(text.contains("kind: steps"), "{text}");
        assert_eq!(crate::from_yaml::<ProfileFile>(&text).unwrap(), f);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p glafira-load-model && cargo clippy -p glafira-load-model --all-targets -- -D warnings`
Expected: все тесты PASS (в `prod_load` 4, в `profile` 5), clippy чисто.

- [ ] **Step 4: Commit**

```bash
git add crates/glafira-load-model
git commit -m "feat(model): цифры ПРОМ с предпросмотром импорта и расчёт профиля нагрузки

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Крейт `glafira-load-contracts` — каркас, сжатие JSON Schema, импортёр OpenAPI/Swagger

**Files:**
- Modify: `crates/glafira-load-contracts/Cargo.toml`
- Create: `crates/glafira-load-contracts/src/{lib,summary,openapi}.rs`
- Create: `dev/fixtures/orders-openapi/openapi.yaml`, `dev/fixtures/orders-openapi/schemas/order.yaml`, `dev/fixtures/orders-swagger/swagger.json`
- Test: `crates/glafira-load-contracts/src/openapi.rs` (`mod tests`), `summary.rs` (`mod tests`)

**Interfaces:**
- Consumes: `glafira_load_model::{project::{ContractKind, Direction}, registry::*, schema::SchemaSummary}`.
- Produces: `glafira_load_contracts::{parse, ContractInput, ReadFile, ParseOptions, TopicBinding, ParseResult, Diagnostic, Table, Column, ParseError}`; `summary::{Resolver, summarize}`; `openapi::parse`. Внутренние помощники `parse_document`, `line_of`, `dir_of`, `join_path` (pub(crate)).

- [ ] **Step 1: Зависимости и фикстуры**

`crates/glafira-load-contracts/Cargo.toml`, `[dependencies]`:

```toml
glafira-load-model = { path = "../glafira-load-model" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml_ng = "0.10"
regex = "1"
roxmltree = "0.20"
sqlparser = "0.58"
thiserror = "2"
```

(Если `cargo add sqlparser` ставит более новую версию — взять её; API `Statement::CreateTable` / `AlterTable` см. Task 9.)

`dev/fixtures/orders-openapi/openapi.yaml`:

```yaml
openapi: 3.0.3
info: { title: Orders, version: "1.4.0" }
paths:
  /v1/orders:
    post:
      operationId: createOrder
      summary: Create an order
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "./schemas/order.yaml#/NewOrder"
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Order"
        "400":
          description: Bad request
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Error"
      callbacks:
        onDone: {}
  /v1/orders/{id}:
    parameters:
      - name: id
        in: path
        required: true
        schema: { type: string }
    get:
      operationId: getOrder
      parameters:
        - name: expand
          in: query
          schema: { type: string, enum: [items, customer] }
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Order"
        "404":
          description: Not found
components:
  schemas:
    Order:
      type: object
      required: [id, status]
      properties:
        id: { type: string, example: "A-1" }
        status: { type: string, enum: [NEW, PAID, FAILED] }
        items:
          type: array
          items:
            $ref: "./schemas/order.yaml#/Item"
        customer:
          $ref: "#/components/schemas/Customer"
    Customer:
      type: object
      properties:
        name: { type: string }
        self:
          $ref: "#/components/schemas/Customer"
    Error:
      type: object
      properties:
        code: { type: integer }
```

`dev/fixtures/orders-openapi/schemas/order.yaml`:

```yaml
NewOrder:
  type: object
  required: [items]
  properties:
    items:
      type: array
      items:
        $ref: "#/Item"
    note: { type: [string, "null"] }
Item:
  type: object
  properties:
    sku: { type: string }
    qty: { type: integer, default: 1 }
```

`dev/fixtures/orders-swagger/swagger.json`:

```json
{
  "swagger": "2.0",
  "info": { "title": "Legacy", "version": "1" },
  "paths": {
    "/legacy/pay": {
      "post": {
        "operationId": "pay",
        "parameters": [
          { "name": "X-Request-Id", "in": "header", "type": "string" },
          { "name": "body", "in": "body", "required": true, "schema": { "$ref": "#/definitions/Payment" } }
        ],
        "responses": { "200": { "description": "ok", "schema": { "$ref": "#/definitions/Receipt" } } }
      }
    }
  },
  "definitions": {
    "Payment": { "type": "object", "required": ["amount"], "properties": { "amount": { "type": "number" }, "currency": { "type": "string" } } },
    "Receipt": { "type": "object", "properties": { "id": { "type": "string" } } }
  }
}
```

- [ ] **Step 2: `lib.rs`**

```rust
//! Импортёры контрактов: байты файла → черновики операций (спека L1 §5.2). Крейт не читает диск:
//! соседние файлы (`$ref`, `import`) приходят через замыкание вызывающего, которое проверяет
//! границы клона и возвращает `None` за его пределами.

pub mod asyncapi;
pub mod openapi;
pub mod proto;
pub mod schema;
pub mod spring;
pub mod sql;
pub mod summary;

use std::collections::BTreeMap;

pub use glafira_load_model::project::{ContractKind, Direction};
use glafira_load_model::registry::OperationDraft;
use serde::{Deserialize, Serialize};

/// Чтение файла клона по относительному пути с разделителем `/`.
pub type ReadFile<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

pub struct ContractInput<'a> {
    /// Путь файла относительно клона, разделитель `/`.
    pub path: &'a str,
    pub bytes: &'a [u8],
    pub read: ReadFile<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicBinding {
    pub topic: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    pub service: String,
    /// Путь файла схемы → топик (из `kafka.topics` манифеста).
    pub topics: BTreeMap<String, TopicBinding>,
    /// Каталоги `import` для proto, относительно клона.
    pub include: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub path: String,
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParseResult {
    pub operations: Vec<OperationDraft>,
    pub tables: Vec<Table>,
    pub warnings: Vec<Diagnostic>,
}

impl ParseResult {
    pub fn warn(&mut self, path: &str, line: Option<u32>, message: impl Into<String>) {
        self.warnings.push(Diagnostic { path: path.into(), line, message: message.into() });
    }
}

/// Файл не разобран целиком (синтаксис). Частичные проблемы — `ParseResult::warnings`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    pub message: String,
    pub line: Option<u32>,
}

pub fn parse(kind: ContractKind, input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    match kind {
        ContractKind::Openapi => openapi::parse(input, opts),
        ContractKind::Proto => proto::parse(input, opts),
        ContractKind::Asyncapi => asyncapi::parse(input, opts),
        ContractKind::Schema => schema::parse(input, opts),
        ContractKind::Sql => sql::parse(input, opts),
        ContractKind::Spring => spring::parse(input, opts),
    }
}

/// YAML или JSON → `serde_json::Value` (JSON — подмножество YAML, но свой разбор даёт строку ошибки точнее).
pub(crate) fn parse_document(bytes: &[u8]) -> Result<serde_json::Value, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError { message: format!("not UTF-8: {e}"), line: None })?;
    if text.trim_start().starts_with('{') {
        return serde_json::from_str(text).map_err(|e| ParseError { message: e.to_string(), line: Some(e.line() as u32) });
    }
    serde_yaml_ng::from_str(text).map_err(|e| ParseError { message: e.to_string(), line: e.location().map(|l| l.line() as u32) })
}

/// Строка (с 1) первого вхождения `needle`.
pub(crate) fn line_of(text: &str, needle: &str) -> Option<u32> {
    text.find(needle).map(|pos| text[..pos].matches('\n').count() as u32 + 1)
}

pub(crate) fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// `dir` + `rel` с нормализацией `.` и `..`; выход выше корня даёт пустые сегменты, которые
/// отбрасываются — вызывающий всё равно проверяет границы клона.
pub(crate) fn join_path(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_lines() {
        assert_eq!(join_path("api/v1", "../schemas/x.yaml"), "api/schemas/x.yaml");
        assert_eq!(join_path("", "./a.yaml"), "a.yaml");
        assert_eq!(dir_of("a.yaml"), "");
        assert_eq!(dir_of("api/a.yaml"), "api");
        assert_eq!(line_of("a\nb\n  /v1/x:\n", "/v1/x:"), Some(3));
        assert_eq!(line_of("a", "zzz"), None);
        assert!(parse_document(b"{\"a\": 1}").is_ok());
        assert_eq!(parse_document(b"a: [\n").unwrap_err().line, Some(1));
    }
}
```

- [ ] **Step 3: `summary.rs`**

```rust
//! JSON Schema (OpenAPI, AsyncAPI, JSON Schema) → `SchemaSummary` с разрешением `$ref`.

use std::cell::RefCell;
use std::collections::BTreeMap;

use glafira_load_model::schema::{MAX_DEPTH, SchemaSummary};
use serde_json::Value;

use crate::{Diagnostic, ReadFile, dir_of, join_path, parse_document};

/// Разрешение ссылок: локальные `#/a/b` в текущем документе, внешние `file.yaml#/a` — через `read`.
pub struct Resolver<'a> {
    pub path: &'a str,
    read: ReadFile<'a>,
    docs: RefCell<BTreeMap<String, Value>>,
    warnings: RefCell<Vec<Diagnostic>>,
}

impl<'a> Resolver<'a> {
    pub fn new(path: &'a str, root: &Value, read: ReadFile<'a>) -> Self {
        let mut docs = BTreeMap::new();
        docs.insert(path.to_string(), root.clone());
        Self { path, read, docs: RefCell::new(docs), warnings: RefCell::new(Vec::new()) }
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.warnings.borrow_mut().push(Diagnostic { path: self.path.into(), line: None, message: message.into() });
    }

    pub fn take_warnings(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut *self.warnings.borrow_mut())
    }

    /// Документ по пути (кэш); внешние адреса и отсутствующие файлы — предупреждение и `None`.
    fn document(&self, file: &str) -> Option<Value> {
        if let Some(d) = self.docs.borrow().get(file) {
            return Some(d.clone());
        }
        let bytes = (self.read)(file)?;
        match parse_document(&bytes) {
            Ok(v) => {
                self.docs.borrow_mut().insert(file.into(), v.clone());
                Some(v)
            }
            Err(e) => {
                self.warn(format!("{file}: {}", e.message));
                None
            }
        }
    }

    /// `$ref` относительно документа `doc_file` → (файл, узел).
    pub fn resolve(&self, doc_file: &str, reference: &str) -> Option<(String, Value)> {
        if reference.contains("://") {
            self.warn(format!("external $ref is not supported: {reference}"));
            return None;
        }
        let (file_part, pointer) = reference.split_once('#').unwrap_or((reference, ""));
        let file = if file_part.is_empty() { doc_file.to_string() } else { join_path(dir_of(doc_file), file_part) };
        let doc = self.document(&file)?;
        let node = if pointer.is_empty() { Some(&doc) } else { doc.pointer(&pointer.replace("~1", "/").replace("~0", "~")) };
        match node {
            Some(n) => Some((file, n.clone())),
            None => {
                self.warn(format!("$ref not found: {reference}"));
                None
            }
        }
    }
}

fn short(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > 64 { s.chars().take(61).collect::<String>() + "…" } else { s }
}

fn type_of(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).find(|t| *t != "null").map(String::from),
        _ => None,
    }
}

/// Сжатие схемы `schema` из документа `doc_file`. `stack` — цепочка `$ref` против циклов.
pub fn summarize(name: &str, schema: &Value, required: bool, doc_file: &str, r: &Resolver, depth: usize, stack: &mut Vec<String>) -> SchemaSummary {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if stack.iter().any(|s| s == reference) {
            return SchemaSummary::leaf(name, format!("ref:{}", reference.rsplit('/').next().unwrap_or(reference))).required(required);
        }
        let Some((file, node)) = r.resolve(doc_file, reference) else {
            return SchemaSummary::leaf(name, "unresolved").required(required);
        };
        stack.push(reference.to_string());
        let out = summarize(name, &node, required, &file, r, depth, stack);
        stack.pop();
        return out;
    }
    let mut out = SchemaSummary::leaf(name, "any").required(required);
    if depth >= MAX_DEPTH {
        out.truncated = true;
        return out;
    }
    let example = schema.get("example").or_else(|| schema.get("default")).or_else(|| schema.get("examples").and_then(|e| e.get(0)));
    out.example = example.map(short);
    if let Some(variants) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) {
        out.kind = "one_of".into();
        out.children = variants.iter().enumerate().map(|(i, v)| summarize(&format!("variant{}", i + 1), v, false, doc_file, r, depth + 1, stack)).collect();
        return out;
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        out.kind = "object".into();
        for p in parts {
            let s = summarize(name, p, false, doc_file, r, depth, stack);
            out.children.extend(s.children);
        }
        return out;
    }
    let kind = type_of(schema).unwrap_or_else(|| {
        if schema.get("properties").is_some() { "object".into() } else if schema.get("items").is_some() { "array".into() } else if schema.get("enum").is_some() { "string".into() } else { "any".into() }
    });
    out.kind = kind.clone();
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        out.enum_values = e.iter().map(short).collect();
    }
    match kind.as_str() {
        "object" => {
            let required_names: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                out.children = props.iter().map(|(n, s)| summarize(n, s, required_names.contains(&n.as_str()), doc_file, r, depth + 1, stack)).collect();
            }
        }
        "array" => {
            if let Some(items) = schema.get("items") {
                out.children = vec![summarize("items", items, false, doc_file, r, depth + 1, stack)];
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_local_external_and_cyclic_refs() {
        let root: Value = serde_json::json!({
            "components": { "schemas": {
                "Node": { "type": "object", "properties": { "next": { "$ref": "#/components/schemas/Node" }, "ext": { "$ref": "./other.json#/X" } } }
            } }
        });
        let read = |p: &str| (p == "api/other.json").then(|| br#"{"X": {"type": "string", "enum": ["a", "b"], "example": "a"}}"#.to_vec());
        let r = Resolver::new("api/openapi.json", &root, &read);
        let s = summarize("body", &serde_json::json!({ "$ref": "#/components/schemas/Node" }), true, "api/openapi.json", &r, 1, &mut vec![]);
        assert_eq!(s.kind, "object");
        assert!(s.required);
        let next = s.children.iter().find(|c| c.name == "next").unwrap();
        assert_eq!(next.kind, "ref:Node", "цикл обрывается ссылкой на имя");
        let ext = s.children.iter().find(|c| c.name == "ext").unwrap();
        assert_eq!((ext.kind.as_str(), ext.enum_values.clone(), ext.example.as_deref()), ("string", vec!["a".into(), "b".into()], Some("a")));
        assert!(r.take_warnings().is_empty());
    }

    #[test]
    fn unknown_refs_and_urls_warn_but_do_not_fail() {
        let root = serde_json::json!({});
        let read = |_: &str| None;
        let r = Resolver::new("a.yaml", &root, &read);
        let s = summarize("x", &serde_json::json!({ "$ref": "#/nope" }), false, "a.yaml", &r, 1, &mut vec![]);
        assert_eq!(s.kind, "unresolved");
        summarize("y", &serde_json::json!({ "$ref": "https://x/y.json" }), false, "a.yaml", &r, 1, &mut vec![]);
        let w = r.take_warnings();
        assert_eq!(w.len(), 2);
        assert!(w[1].message.contains("external $ref"));
    }

    #[test]
    fn nullable_type_arrays_one_of_and_all_of() {
        let root = serde_json::json!({});
        let read = |_: &str| None;
        let r = Resolver::new("a.yaml", &root, &read);
        let s = summarize("v", &serde_json::json!({ "type": ["string", "null"] }), false, "a.yaml", &r, 1, &mut vec![]);
        assert_eq!(s.kind, "string");
        let s = summarize("v", &serde_json::json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] }), false, "a.yaml", &r, 1, &mut vec![]);
        assert_eq!((s.kind.as_str(), s.children.len()), ("one_of", 2));
        let s = summarize("v", &serde_json::json!({ "allOf": [{ "type": "object", "properties": { "a": { "type": "string" } } }, { "type": "object", "properties": { "b": { "type": "integer" } } }] }), false, "a.yaml", &r, 1, &mut vec![]);
        assert_eq!(s.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }
}
```

- [ ] **Step 4: `openapi.rs`**

```rust
//! OpenAPI 3.0/3.1 и Swagger 2.0 → операции `http` (спека L1 §5.2).

use std::collections::BTreeMap;

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use serde_json::Value;

use crate::summary::{Resolver, summarize};
use crate::{ContractInput, ParseError, ParseOptions, ParseResult, line_of, parse_document};

const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

#[derive(Clone, Copy, PartialEq)]
enum Version {
    V2,
    V3,
}

/// Узел или то, на что он ссылается (параметры, тела и ответы бывают `$ref`).
fn deref<'a>(r: &Resolver, file: &str, node: &'a Value) -> (String, Value) {
    match node.get("$ref").and_then(Value::as_str) {
        Some(reference) => r.resolve(file, reference).unwrap_or_else(|| (file.to_string(), Value::Null)),
        None => (file.to_string(), node.clone()),
    }
}

fn first_media_schema(content: Option<&Value>) -> Option<&Value> {
    content?.as_object()?.values().next()?.get("schema")
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let doc = parse_document(input.bytes)?;
    let text = String::from_utf8_lossy(input.bytes);
    let version = if doc.get("openapi").is_some() {
        Version::V3
    } else if doc.get("swagger").is_some() {
        Version::V2
    } else {
        return Err(ParseError { message: "neither an OpenAPI nor a Swagger document (no openapi/swagger field)".into(), line: None });
    };
    let r = Resolver::new(input.path, &doc, input.read);
    let file = input.path;
    let mut out = ParseResult::default();
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        out.warn(file, None, "no paths");
        return Ok(out);
    };
    for (path, item) in paths {
        let Some(item) = item.as_object() else { continue };
        let line = line_of(&text, &format!("{path}:")).or_else(|| line_of(&text, &format!("\"{path}\"")));
        let shared: Vec<Value> = item.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            let mut draft = OperationDraft::new(&opts.service, OpKind::Http, Address::Http { method: method.to_ascii_uppercase(), path: path.clone() }, Source { kind: SourceKind::Openapi, path: file.into(), line });
            if let Some(id) = str_of(op, "operationId") {
                draft.name = id.into();
            }
            draft.summary = str_of(op, "summary").or_else(|| str_of(op, "description")).unwrap_or("").into();
            if op.get("callbacks").is_some() || op.get("links").is_some() {
                out.warn(file, line, format!("{}: callbacks and links are not supported", draft.name));
            }
            // Запрос: параметры пути и операции, затем тело.
            let mut children = Vec::new();
            let own: Vec<Value> = op.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut stack = Vec::new();
            for p in shared.iter().chain(own.iter()) {
                let (pfile, p) = deref(&r, file, p);
                let (Some(name), Some(loc)) = (str_of(&p, "name"), str_of(&p, "in")) else { continue };
                let required = loc == "path" || p.get("required").and_then(Value::as_bool).unwrap_or(false);
                match (version, loc) {
                    (Version::V2, "body") => {
                        if let Some(schema) = p.get("schema") {
                            children.push(summarize("body", schema, required, &pfile, &r, 2, &mut stack));
                        }
                    }
                    (Version::V2, _) => children.push(SchemaSummary::leaf(format!("{loc}.{name}"), str_of(&p, "type").unwrap_or("string")).required(required)),
                    (Version::V3, _) => {
                        let kind = p.get("schema").map(|s| summarize(name, s, required, &pfile, &r, 2, &mut stack).kind).unwrap_or_else(|| "string".into());
                        children.push(SchemaSummary::leaf(format!("{loc}.{name}"), kind).required(required));
                    }
                }
            }
            if version == Version::V3
                && let Some(body) = op.get("requestBody")
            {
                let (bfile, body) = deref(&r, file, body);
                let required = body.get("required").and_then(Value::as_bool).unwrap_or(false);
                if let Some(schema) = first_media_schema(body.get("content")) {
                    children.push(summarize("body", schema, required, &bfile, &r, 2, &mut stack));
                }
            }
            if !children.is_empty() {
                let mut req = SchemaSummary::object("request", children);
                req.limit();
                draft.request = Some(req);
            }
            // Ответы: первый 2xx — `response`, остальные — `responses`.
            let mut responses: BTreeMap<String, SchemaSummary> = BTreeMap::new();
            if let Some(resps) = op.get("responses").and_then(Value::as_object) {
                for (code, resp) in resps {
                    let (rfile, resp) = deref(&r, file, resp);
                    let schema = match version {
                        Version::V3 => first_media_schema(resp.get("content")),
                        Version::V2 => resp.get("schema"),
                    };
                    let mut s = match schema {
                        Some(s) => summarize("response", s, false, &rfile, &r, 1, &mut stack),
                        None => SchemaSummary::leaf("response", "empty"),
                    };
                    s.limit();
                    responses.insert(code.clone(), s);
                }
            }
            if let Some(code) = responses.keys().find(|c| c.starts_with('2')).cloned() {
                draft.response = responses.remove(&code);
            }
            draft.responses = responses;
            out.operations.push(draft);
        }
    }
    out.warnings.extend(r.take_warnings());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENAPI: &str = include_str!("../../../dev/fixtures/orders-openapi/openapi.yaml");
    const ORDER: &str = include_str!("../../../dev/fixtures/orders-openapi/schemas/order.yaml");
    const SWAGGER: &str = include_str!("../../../dev/fixtures/orders-swagger/swagger.json");

    fn opts() -> ParseOptions {
        ParseOptions { service: "orders".into(), ..Default::default() }
    }

    fn names(s: &SchemaSummary) -> Vec<&str> {
        s.children.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn openapi_3_operations_with_external_refs() {
        let read = |p: &str| (p == "api/schemas/order.yaml").then(|| ORDER.as_bytes().to_vec());
        let input = ContractInput { path: "api/openapi.yaml", bytes: OPENAPI.as_bytes(), read: &read };
        let out = parse(&input, &opts()).unwrap();
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["http:orders:POST /v1/orders", "http:orders:GET /v1/orders/{id}"]);
        let post = &out.operations[0];
        assert_eq!((post.name.as_str(), post.summary.as_str()), ("createOrder", "Create an order"));
        assert_eq!(post.source.line, Some(4));
        let req = post.request.as_ref().unwrap();
        assert_eq!(names(req), ["body"]);
        let body = &req.children[0];
        assert!(body.required);
        assert_eq!(names(body), ["items", "note"]);
        assert_eq!(body.children[0].children[0].children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["sku", "qty"], "Item из соседнего файла по локальной ссылке");
        assert_eq!(body.children[1].kind, "string", "[string, null] → string");
        let resp = post.response.as_ref().unwrap();
        assert_eq!(names(resp), ["id", "status", "items", "customer"]);
        assert_eq!(resp.children[1].enum_values, ["NEW", "PAID", "FAILED"]);
        assert_eq!(resp.children[3].children[1].kind, "ref:Customer");
        assert_eq!(post.responses.keys().collect::<Vec<_>>(), ["400"]);
        let get = &out.operations[1];
        let req = get.request.as_ref().unwrap();
        assert_eq!(names(req), ["path.id", "query.expand"]);
        assert!(req.children[0].required && !req.children[1].required);
        assert_eq!(get.responses["404"].kind, "empty");
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].message.contains("callbacks"));
    }

    #[test]
    fn swagger_2_body_and_header_params() {
        let read = |_: &str| None;
        let input = ContractInput { path: "swagger.json", bytes: SWAGGER.as_bytes(), read: &read };
        let out = parse(&input, &opts()).unwrap();
        let op = &out.operations[0];
        assert_eq!(op.id, "http:orders:POST /legacy/pay");
        let req = op.request.as_ref().unwrap();
        assert_eq!(names(req), ["header.X-Request-Id", "body"]);
        assert_eq!(names(&req.children[1]), ["amount", "currency"]);
        assert!(req.children[1].children[0].required);
        assert_eq!(names(op.response.as_ref().unwrap()), ["id"]);
    }

    #[test]
    fn not_openapi_is_an_error_and_missing_ref_file_warns() {
        let read = |_: &str| None;
        assert!(parse(&ContractInput { path: "x.yaml", bytes: b"info: {}\n", read: &read }, &opts()).is_err());
        let out = parse(&ContractInput { path: "api/openapi.yaml", bytes: OPENAPI.as_bytes(), read: &read }, &opts()).unwrap();
        let body = &out.operations[0].request.as_ref().unwrap().children[0];
        assert_eq!(body.kind, "unresolved");
        assert!(out.warnings.iter().any(|w| w.message.contains("$ref not found")), "{:?}", out.warnings);
    }
}
```

Пока модули `asyncapi`, `proto`, `schema`, `sql`, `spring` не написаны (Tasks 7–10), каждый из них — файл с заглушкой, которую следующие задачи заменяют:

```rust
//! Заполняется в Task N.
use crate::{ContractInput, ParseError, ParseOptions, ParseResult};

pub fn parse(input: &ContractInput, _opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let mut out = ParseResult::default();
    out.warn(input.path, None, "importer not implemented yet");
    Ok(out)
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p glafira-load-contracts && cargo clippy -p glafira-load-contracts --all-targets -- -D warnings`
Expected: тесты `lib`, `summary` (3), `openapi` (3) PASS. Если `source.line` в первом тесте отличается от 4, проверить, что `line_of` ищет `"/v1/orders:"` — в фикстуре строка 4.

- [ ] **Step 6: Commit**

```bash
git add crates/glafira-load-contracts dev/fixtures
git commit -m "feat(contracts): каркас импортёров, сжатие JSON Schema с \$ref и импортёр OpenAPI/Swagger

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Импортёр Protobuf (собственный разбор подмножества proto3)

**Files:**
- Create: `crates/glafira-load-contracts/src/proto.rs` (заменяет заглушку)
- Create: `dev/fixtures/orders-proto/proto/orders/v1/orders.proto`, `dev/fixtures/orders-proto/proto/common/money.proto`
- Test: `proto.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ContractInput`, `ParseOptions.include`, `OperationDraft`, `SchemaSummary`.
- Produces: `proto::parse` → операции `grpc` с `Address::Grpc { service: "<package>.<Service>", method }`, `streaming`, `request`/`response` из сообщений.

- [ ] **Step 1: Фикстуры**

`dev/fixtures/orders-proto/proto/orders/v1/orders.proto`:

```proto
syntax = "proto3";

package orders.v1;

import "common/money.proto";
import "google/protobuf/timestamp.proto";

option java_package = "demo.orders";

// Позиция заказа.
message Item {
  string sku = 1;
  int32 qty = 2;
  common.Money price = 3;
}

message Order {
  string id = 1;
  repeated Item items = 2;
  Status status = 3;
  map<string, string> labels = 4;
  google.protobuf.Timestamp created_at = 5;
  oneof payer {
    string customer_id = 6;
    string company_id = 7;
  }
  enum Status {
    NEW = 0;
    PAID = 1;
  }
  reserved 8, 9;
}

message CreateRequest { Order order = 1; }
message GetRequest { string id = 1; }
message WatchRequest {}

service OrderService {
  rpc Create(CreateRequest) returns (Order);
  rpc Get(GetRequest) returns (Order) {
    option (google.api.http) = { get: "/v1/orders/{id}" };
  }
  rpc Watch(WatchRequest) returns (stream Order);
}
```

`dev/fixtures/orders-proto/proto/common/money.proto`:

```proto
syntax = "proto3";
package common;

/* Денежная сумма. */
message Money {
  string currency = 1;
  int64 units = 2;
  int32 nanos = 3;
}
```

- [ ] **Step 2: `proto.rs`**

```rust
//! Protobuf 3 → операции `grpc` (спека L1 §5.2, решение плана 2: свой разбор подмножества,
//! потому что крейт не читает диск). Неподдержанные конструкции дают предупреждение со строкой.

use std::collections::{BTreeMap, BTreeSet};

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind, Streaming};
use glafira_load_model::schema::{MAX_DEPTH, SchemaSummary};

use crate::{ContractInput, ParseError, ParseOptions, ParseResult, dir_of, join_path};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Punct(char),
}

struct Token {
    tok: Tok,
    line: u32,
}

fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0usize, 1u32, Vec::new());
    while i < b.len() {
        let c = b[i] as char;
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            let end = src[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(b.len());
            line += src[i..end].matches('\n').count() as u32;
            i = end;
        } else if c.is_ascii_alphabetic() || c == '_' || c == '.' {
            let start = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            out.push(Token { tok: Tok::Ident(src[start..i].into()), line });
        } else if c.is_ascii_digit() || c == '-' {
            let start = i;
            i += 1;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            out.push(Token { tok: Tok::Num(src[start..i].into()), line });
        } else if c == '"' || c == '\'' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] as char != c {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            out.push(Token { tok: Tok::Str(src[start..i.min(b.len())].into()), line });
            i += 1;
        } else if "{}()<>[]=;,".contains(c) {
            out.push(Token { tok: Tok::Punct(c), line });
            i += 1;
        } else {
            return Err(ParseError { message: format!("unexpected character {c:?}"), line: Some(line) });
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default)]
struct Field {
    name: String,
    type_name: String,
    repeated: bool,
    /// Тип значения у `map<K, V>`.
    map_value: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Message {
    fields: Vec<Field>,
}

#[derive(Debug, Clone)]
struct Rpc {
    name: String,
    request: String,
    response: String,
    client_stream: bool,
    server_stream: bool,
    line: u32,
}

#[derive(Debug, Clone)]
struct Service {
    name: String,
    rpcs: Vec<Rpc>,
}

/// Разобранный файл; после слияния импортов — все известные сообщения и enum по полным именам.
#[derive(Debug, Default)]
struct File {
    package: String,
    imports: Vec<String>,
    messages: BTreeMap<String, Message>,
    enums: BTreeMap<String, Vec<String>>,
    services: Vec<Service>,
    warnings: Vec<(u32, String)>,
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
    file: File,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }
    fn line(&self) -> u32 {
        self.toks.get(self.i).or(self.toks.last()).map(|t| t.line).unwrap_or(1)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).map(|t| t.tok.clone());
        self.i += 1;
        t
    }
    fn err(&self, message: impl Into<String>) -> ParseError {
        ParseError { message: message.into(), line: Some(self.line()) }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            other => Err(self.err(format!("identifier expected, got {other:?}"))),
        }
    }
    fn punct(&mut self, c: char) -> Result<(), ParseError> {
        match self.next() {
            Some(Tok::Punct(p)) if p == c => Ok(()),
            other => Err(self.err(format!("{c:?} expected, got {other:?}"))),
        }
    }
    fn is_punct(&self, c: char) -> bool {
        self.peek() == Some(&Tok::Punct(c))
    }
    fn is_ident(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == s)
    }
    /// До `;` на нулевой глубине скобок (опции с `{ … }` внутри).
    fn skip_statement(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.next() {
            match t {
                Tok::Punct('{') | Tok::Punct('(') | Tok::Punct('[') => depth += 1,
                Tok::Punct('}') | Tok::Punct(')') | Tok::Punct(']') => depth -= 1,
                Tok::Punct(';') if depth <= 0 => return,
                _ => {}
            }
        }
    }
    /// Блок `{ … }` целиком (открывающая скобка ещё не съедена).
    fn skip_block(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.next() {
            match t {
                Tok::Punct('{') => depth += 1,
                Tok::Punct('}') => {
                    depth -= 1;
                    if depth <= 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
    fn skip_field_options(&mut self) {
        if self.is_punct('[') {
            let mut depth = 0i32;
            while let Some(t) = self.next() {
                match t {
                    Tok::Punct('[') => depth += 1,
                    Tok::Punct(']') => {
                        depth -= 1;
                        if depth <= 0 {
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn file(&mut self) -> Result<(), ParseError> {
        while let Some(t) = self.peek().cloned() {
            match t {
                Tok::Ident(w) if w == "syntax" => {
                    self.next();
                    self.punct('=')?;
                    if let Some(Tok::Str(s)) = self.next()
                        && s != "proto3"
                    {
                        self.file.warnings.push((self.line(), format!("syntax {s}: only proto3 is supported, parsing anyway")));
                    }
                    self.punct(';')?;
                }
                Tok::Ident(w) if w == "package" => {
                    self.next();
                    self.file.package = self.ident()?;
                    self.punct(';')?;
                }
                Tok::Ident(w) if w == "import" => {
                    self.next();
                    if self.is_ident("public") || self.is_ident("weak") {
                        self.next();
                    }
                    match self.next() {
                        Some(Tok::Str(p)) => self.file.imports.push(p),
                        other => return Err(self.err(format!("import path expected, got {other:?}"))),
                    }
                    self.punct(';')?;
                }
                Tok::Ident(w) if w == "option" => self.skip_statement(),
                Tok::Ident(w) if w == "message" => {
                    self.next();
                    let scope = self.file.package.clone();
                    self.message(&scope)?;
                }
                Tok::Ident(w) if w == "enum" => {
                    self.next();
                    let scope = self.file.package.clone();
                    self.enum_def(&scope)?;
                }
                Tok::Ident(w) if w == "service" => {
                    self.next();
                    self.service()?;
                }
                Tok::Ident(w) if w == "extend" => {
                    self.file.warnings.push((self.line(), "extend is not supported".into()));
                    self.next();
                    self.ident()?;
                    self.skip_block();
                }
                Tok::Punct(';') => {
                    self.next();
                }
                other => return Err(self.err(format!("unexpected {other:?} at top level"))),
            }
        }
        Ok(())
    }

    fn fq(scope: &str, name: &str) -> String {
        if scope.is_empty() { name.into() } else { format!("{scope}.{name}") }
    }

    fn message(&mut self, scope: &str) -> Result<(), ParseError> {
        let name = self.ident()?;
        let fq = Self::fq(scope, &name);
        self.punct('{')?;
        let mut msg = Message::default();
        while !self.is_punct('}') {
            let Some(t) = self.peek().cloned() else { return Err(self.err("unexpected end of file in message")) };
            match t {
                Tok::Ident(w) if w == "message" => {
                    self.next();
                    self.message(&fq)?;
                }
                Tok::Ident(w) if w == "enum" => {
                    self.next();
                    self.enum_def(&fq)?;
                }
                Tok::Ident(w) if w == "oneof" => {
                    self.next();
                    self.ident()?;
                    self.punct('{')?;
                    while !self.is_punct('}') {
                        if self.is_ident("option") {
                            self.skip_statement();
                        } else {
                            msg.fields.push(self.field(false)?);
                        }
                    }
                    self.punct('}')?;
                }
                Tok::Ident(w) if w == "reserved" || w == "extensions" || w == "option" => self.skip_statement(),
                Tok::Ident(w) if w == "map" => {
                    self.next();
                    self.punct('<')?;
                    self.ident()?;
                    self.punct(',')?;
                    let value = self.ident()?;
                    self.punct('>')?;
                    let fname = self.ident()?;
                    self.punct('=')?;
                    self.next();
                    self.skip_field_options();
                    self.punct(';')?;
                    msg.fields.push(Field { name: fname, type_name: "map".into(), repeated: false, map_value: Some(value) });
                }
                Tok::Ident(w) if w == "group" => {
                    self.file.warnings.push((self.line(), "group is not supported".into()));
                    self.skip_statement();
                }
                Tok::Ident(w) if w == "repeated" || w == "optional" || w == "required" => {
                    self.next();
                    msg.fields.push(self.field(w == "repeated")?);
                }
                Tok::Ident(_) => msg.fields.push(self.field(false)?),
                Tok::Punct(';') => {
                    self.next();
                }
                other => return Err(self.err(format!("unexpected {other:?} in message {name}"))),
            }
        }
        self.punct('}')?;
        self.file.messages.insert(fq, msg);
        Ok(())
    }

    fn field(&mut self, repeated: bool) -> Result<Field, ParseError> {
        let type_name = self.ident()?;
        let name = self.ident()?;
        self.punct('=')?;
        self.next();
        self.skip_field_options();
        self.punct(';')?;
        Ok(Field { name, type_name, repeated, map_value: None })
    }

    fn enum_def(&mut self, scope: &str) -> Result<(), ParseError> {
        let name = self.ident()?;
        self.punct('{')?;
        let mut values = Vec::new();
        while !self.is_punct('}') {
            if self.is_ident("option") || self.is_ident("reserved") {
                self.skip_statement();
                continue;
            }
            let v = self.ident()?;
            self.punct('=')?;
            self.next();
            self.skip_field_options();
            self.punct(';')?;
            values.push(v);
        }
        self.punct('}')?;
        self.file.enums.insert(Self::fq(scope, &name), values);
        Ok(())
    }

    fn service(&mut self) -> Result<(), ParseError> {
        let name = self.ident()?;
        self.punct('{')?;
        let mut rpcs = Vec::new();
        while !self.is_punct('}') {
            if self.is_ident("option") {
                self.skip_statement();
                continue;
            }
            if !self.is_ident("rpc") {
                return Err(self.err("rpc expected in service"));
            }
            let line = self.line();
            self.next();
            let rname = self.ident()?;
            self.punct('(')?;
            let client_stream = self.is_ident("stream") && self.next().is_some();
            let request = self.ident()?;
            self.punct(')')?;
            if !self.is_ident("returns") {
                return Err(self.err("returns expected"));
            }
            self.next();
            self.punct('(')?;
            let server_stream = self.is_ident("stream") && self.next().is_some();
            let response = self.ident()?;
            self.punct(')')?;
            if self.is_punct('{') {
                self.skip_block();
            } else {
                self.punct(';')?;
            }
            rpcs.push(Rpc { name: rname, request, response, client_stream, server_stream, line });
        }
        self.punct('}')?;
        self.file.services.push(Service { name, rpcs });
        Ok(())
    }
}

fn parse_text(text: &str) -> Result<File, ParseError> {
    let mut p = Parser { toks: lex(text)?, i: 0, file: File::default() };
    p.file()?;
    Ok(p.file)
}

/// Well-known types: их `import` не ищется, тип сворачивается в скаляр JSON-отображения.
fn well_known(name: &str) -> Option<&'static str> {
    Some(match name.trim_start_matches('.') {
        "google.protobuf.Timestamp" | "google.protobuf.Duration" | "google.protobuf.StringValue" | "google.protobuf.BytesValue" => "string",
        "google.protobuf.Empty" | "google.protobuf.Any" | "google.protobuf.Struct" | "google.protobuf.Value" => "object",
        "google.protobuf.Int32Value" | "google.protobuf.Int64Value" | "google.protobuf.UInt32Value" | "google.protobuf.UInt64Value" => "integer",
        "google.protobuf.FloatValue" | "google.protobuf.DoubleValue" => "number",
        "google.protobuf.BoolValue" => "boolean",
        _ => return None,
    })
}

fn scalar(name: &str) -> Option<&'static str> {
    Some(match name {
        "double" | "float" => "number",
        "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" | "fixed32" | "fixed64" | "sfixed32" | "sfixed64" => "integer",
        "bool" => "boolean",
        "string" => "string",
        "bytes" => "bytes",
        _ => return None,
    })
}

/// Имя типа из области `scope` (`pkg.Outer`): проверяются `scope.T`, родители, затем `T` как есть.
fn resolve_type(name: &str, scope: &str, file: &File) -> Option<String> {
    if let Some(n) = name.strip_prefix('.') {
        return (file.messages.contains_key(n) || file.enums.contains_key(n)).then(|| n.to_string());
    }
    let mut scope = scope.to_string();
    loop {
        let candidate = if scope.is_empty() { name.to_string() } else { format!("{scope}.{name}") };
        if file.messages.contains_key(&candidate) || file.enums.contains_key(&candidate) {
            return Some(candidate);
        }
        if scope.is_empty() {
            return None;
        }
        scope = scope.rsplit_once('.').map(|(p, _)| p.to_string()).unwrap_or_default();
    }
}

fn type_summary(name: &str, type_name: &str, scope: &str, file: &File, depth: usize, stack: &mut Vec<String>, unresolved: &mut BTreeSet<String>) -> SchemaSummary {
    if let Some(k) = scalar(type_name).or_else(|| well_known(type_name)) {
        return SchemaSummary::leaf(name, k);
    }
    let Some(fq) = resolve_type(type_name, scope, file) else {
        unresolved.insert(type_name.to_string());
        return SchemaSummary::leaf(name, format!("unresolved:{type_name}"));
    };
    if let Some(values) = file.enums.get(&fq) {
        let mut s = SchemaSummary::leaf(name, "enum");
        s.enum_values = values.clone();
        return s;
    }
    if stack.contains(&fq) {
        return SchemaSummary::leaf(name, format!("ref:{}", fq.rsplit('.').next().unwrap_or(&fq)));
    }
    message_summary(name, &fq, file, depth, stack, unresolved)
}

fn message_summary(name: &str, fq: &str, file: &File, depth: usize, stack: &mut Vec<String>, unresolved: &mut BTreeSet<String>) -> SchemaSummary {
    let mut out = SchemaSummary::object(name, vec![]);
    if depth >= MAX_DEPTH {
        out.truncated = true;
        return out;
    }
    let Some(msg) = file.messages.get(fq) else { return out };
    stack.push(fq.to_string());
    for f in &msg.fields {
        let child = match &f.map_value {
            Some(v) => {
                let mut m = SchemaSummary::leaf(&f.name, "map");
                m.children = vec![type_summary("value", v, fq, file, depth + 1, stack, unresolved)];
                m
            }
            None => type_summary(&f.name, &f.type_name, fq, file, depth + 1, stack, unresolved),
        };
        out.children.push(if f.repeated { SchemaSummary::array(&f.name, child) } else { child });
    }
    stack.pop();
    out
}

/// Разбирает файл и его импорты (кроме `google/protobuf/*`), сливая сообщения и enum.
fn load(path: &str, bytes: &[u8], input: &ContractInput, opts: &ParseOptions, visited: &mut BTreeSet<String>, out: &mut ParseResult) -> Result<File, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError { message: format!("not UTF-8: {e}"), line: None })?;
    let mut file = parse_text(text)?;
    for (line, w) in std::mem::take(&mut file.warnings) {
        out.warn(path, Some(line), w);
    }
    for import in file.imports.clone() {
        if import.starts_with("google/protobuf/") || !visited.insert(import.clone()) {
            continue;
        }
        let candidates: Vec<String> = opts.include.iter().map(|inc| join_path(inc, &import)).chain([join_path(dir_of(path), &import), import.clone()]).collect();
        let Some((found, bytes)) = candidates.iter().find_map(|c| (input.read)(c).map(|b| (c.clone(), b))) else {
            out.warn(path, None, format!("import not found: {import}"));
            continue;
        };
        match load(&found, &bytes, input, opts, visited, out) {
            Ok(imported) => {
                for (k, v) in imported.messages {
                    file.messages.entry(k).or_insert(v);
                }
                for (k, v) in imported.enums {
                    file.enums.entry(k).or_insert(v);
                }
            }
            Err(e) => out.warn(&found, e.line, e.message),
        }
    }
    Ok(file)
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let mut out = ParseResult::default();
    let mut visited = BTreeSet::from([input.path.to_string()]);
    let file = load(input.path, input.bytes, input, opts, &mut visited, &mut out)?;
    let mut unresolved = BTreeSet::new();
    for svc in &file.services {
        let service_fq = Parser::fq(&file.package, &svc.name);
        for rpc in &svc.rpcs {
            let mut draft = OperationDraft::new(&opts.service, OpKind::Grpc, Address::Grpc { service: service_fq.clone(), method: rpc.name.clone() }, Source { kind: SourceKind::Proto, path: input.path.into(), line: Some(rpc.line) });
            draft.name = rpc.name.clone();
            let mut stack = Vec::new();
            let mut req = type_summary("request", &rpc.request, &file.package, &file, 1, &mut stack, &mut unresolved);
            let mut resp = type_summary("response", &rpc.response, &file.package, &file, 1, &mut stack, &mut unresolved);
            req.limit();
            resp.limit();
            draft.request = Some(req);
            draft.response = Some(resp);
            draft.streaming = match (rpc.client_stream, rpc.server_stream) {
                (true, true) => Some(Streaming::Bidi),
                (true, false) => Some(Streaming::Client),
                (false, true) => Some(Streaming::Server),
                (false, false) => None,
            };
            out.operations.push(draft);
        }
    }
    for t in unresolved {
        out.warn(input.path, None, format!("type not found: {t}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str = include_str!("../../../dev/fixtures/orders-proto/proto/orders/v1/orders.proto");
    const MONEY: &str = include_str!("../../../dev/fixtures/orders-proto/proto/common/money.proto");

    fn opts() -> ParseOptions {
        ParseOptions { service: "orders".into(), include: vec!["proto".into()], ..Default::default() }
    }

    fn child<'a>(s: &'a SchemaSummary, name: &str) -> &'a SchemaSummary {
        s.children.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no field {name} in {:?}", s.children.iter().map(|c| &c.name).collect::<Vec<_>>()))
    }

    #[test]
    fn services_messages_imports_and_streaming() {
        let read = |p: &str| (p == "proto/common/money.proto").then(|| MONEY.as_bytes().to_vec());
        let input = ContractInput { path: "proto/orders/v1/orders.proto", bytes: ORDERS.as_bytes(), read: &read };
        let out = parse(&input, &opts()).unwrap();
        assert_eq!(out.warnings, vec![], "{:?}", out.warnings);
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), [
            "grpc:orders:orders.v1.OrderService/Create",
            "grpc:orders:orders.v1.OrderService/Get",
            "grpc:orders:orders.v1.OrderService/Watch",
        ]);
        let create = &out.operations[0];
        assert_eq!(create.source.line, Some(39));
        let order = child(create.request.as_ref().unwrap(), "order");
        assert_eq!(order.kind, "object");
        assert_eq!(child(order, "id").kind, "string");
        let items = child(order, "items");
        assert_eq!(items.kind, "array");
        let price = child(&items.children[0], "price");
        assert_eq!(price.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["currency", "units", "nanos"], "Money из импорта");
        assert_eq!(child(order, "status").enum_values, ["NEW", "PAID"]);
        assert_eq!((child(order, "labels").kind.as_str(), child(order, "labels").children[0].kind.as_str()), ("map", "string"));
        assert_eq!(child(order, "created_at").kind, "string", "well-known Timestamp");
        assert!(order.children.iter().any(|c| c.name == "customer_id") && order.children.iter().any(|c| c.name == "company_id"), "поля oneof лежат плоско");
        assert_eq!(out.operations[2].streaming, Some(Streaming::Server));
        assert_eq!(out.operations[1].streaming, None);
    }

    #[test]
    fn missing_import_warns_and_marks_unresolved() {
        let read = |_: &str| None;
        let input = ContractInput { path: "proto/orders/v1/orders.proto", bytes: ORDERS.as_bytes(), read: &read };
        let out = parse(&input, &opts()).unwrap();
        let price = child(&child(child(out.operations[0].request.as_ref().unwrap(), "order"), "items").children[0], "price");
        assert_eq!(price.kind, "unresolved:common.Money");
        let msgs: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(msgs.contains(&"import not found: common/money.proto"), "{msgs:?}");
        assert!(msgs.contains(&"type not found: common.Money"), "{msgs:?}");
    }

    #[test]
    fn syntax_errors_carry_a_line() {
        let read = |_: &str| None;
        let e = parse(&ContractInput { path: "a.proto", bytes: b"syntax = \"proto3\";\nmessage A {\n  string = 1;\n}\n", read: &read }, &opts()).unwrap_err();
        assert_eq!(e.line, Some(3));
        let out = parse(&ContractInput { path: "a.proto", bytes: b"syntax = \"proto2\";\nmessage A { optional string x = 1; }\nservice S { rpc Do(A) returns (A); }", read: &read }, &opts()).unwrap();
        assert!(out.warnings[0].message.contains("proto2"));
        assert_eq!(out.operations.len(), 1);
    }

    #[test]
    fn recursive_messages_stop_at_a_reference() {
        let read = |_: &str| None;
        let src = b"syntax = \"proto3\"; package t; message Node { Node next = 1; } service S { rpc Walk(Node) returns (Node); }";
        let out = parse(&ContractInput { path: "a.proto", bytes: src, read: &read }, &opts()).unwrap();
        assert_eq!(child(out.operations[0].request.as_ref().unwrap(), "next").kind, "ref:Node");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p glafira-load-contracts proto`
Expected: 4 PASS. Если строка `Create` не 39, сверить номер строки `rpc Create` в фикстуре и поправить ожидание (фикстура выше: `rpc Create` на строке 39).

- [ ] **Step 4: Commit**

```bash
git add crates/glafira-load-contracts/src/proto.rs dev/fixtures/orders-proto
git commit -m "feat(contracts): импортёр Protobuf — сервисы, сообщения, импорты, стриминг

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Импортёры AsyncAPI и схем сообщений (Avro, JSON Schema)

**Files:**
- Create: `crates/glafira-load-contracts/src/{asyncapi,schema}.rs` (заменяют заглушки)
- Create: `dev/fixtures/orders-asyncapi/{asyncapi-v2,asyncapi-v3}.yaml`, `dev/fixtures/schemas/{orders-created.avsc,payment.schema.json}`
- Test: `asyncapi.rs`, `schema.rs` (`mod tests`)

**Interfaces:**
- Consumes: `summary::{Resolver, summarize}`, `ParseOptions.topics`, `TopicBinding`, `Direction`.
- Produces: `asyncapi::parse`, `schema::parse` → операции `kafka_produce` (сообщение в `request`) и `kafka_consume` (сообщение в `response`).

- [ ] **Step 1: Фикстуры**

`dev/fixtures/orders-asyncapi/asyncapi-v2.yaml`:

```yaml
asyncapi: 2.6.0
info: { title: Orders events, version: 1.0.0 }
servers:
  prod: { url: kafka.internal:9092, protocol: kafka }
  legacy: { url: amqp.internal:5672, protocol: amqp }
channels:
  orders.created:
    publish:
      operationId: publishOrderCreated
      summary: Order created
      message:
        $ref: "#/components/messages/OrderCreated"
  payments.completed:
    subscribe:
      message:
        oneOf:
          - $ref: "#/components/messages/PaymentCompleted"
          - $ref: "#/components/messages/PaymentFailed"
components:
  messages:
    OrderCreated:
      name: OrderCreated
      payload:
        type: object
        required: [orderId]
        properties:
          orderId: { type: string }
          total: { type: number }
    PaymentCompleted:
      payload: { type: object, properties: { paymentId: { type: string } } }
    PaymentFailed:
      payload: { type: object, properties: { reason: { type: string } } }
```

`dev/fixtures/orders-asyncapi/asyncapi-v3.yaml`:

```yaml
asyncapi: 3.0.0
info: { title: Orders events, version: 2.0.0 }
channels:
  ordersCreated:
    address: orders.created
    messages:
      OrderCreated:
        payload:
          type: object
          properties:
            orderId: { type: string }
  invoices:
    address: billing.invoices
    messages:
      Invoice:
        $ref: "#/components/messages/Invoice"
operations:
  sendOrderCreated:
    action: send
    channel: { $ref: "#/channels/ordersCreated" }
    summary: Emit order created
  receiveInvoice:
    action: receive
    channel: { $ref: "#/channels/invoices" }
    messages:
      - $ref: "#/channels/invoices/messages/Invoice"
components:
  messages:
    Invoice:
      payload: { type: object, properties: { invoiceId: { type: string }, amount: { type: number } } }
```

`dev/fixtures/schemas/orders-created.avsc`:

```json
{
  "type": "record",
  "name": "OrderCreated",
  "namespace": "demo",
  "fields": [
    { "name": "orderId", "type": "string" },
    { "name": "total", "type": ["null", "double"], "default": null },
    { "name": "status", "type": { "type": "enum", "name": "Status", "symbols": ["NEW", "PAID"] } },
    { "name": "items", "type": { "type": "array", "items": { "type": "record", "name": "Item", "fields": [ { "name": "sku", "type": "string" }, { "name": "qty", "type": "int" } ] } } },
    { "name": "labels", "type": { "type": "map", "values": "string" } },
    { "name": "parent", "type": ["null", "demo.OrderCreated"] },
    { "name": "amount", "type": { "type": "bytes", "logicalType": "decimal", "precision": 10, "scale": 2 } }
  ]
}
```

`dev/fixtures/schemas/payment.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Payment",
  "type": "object",
  "required": ["paymentId"],
  "properties": { "paymentId": { "type": "string" }, "amount": { "type": "number" } }
}
```

- [ ] **Step 2: `asyncapi.rs`**

```rust
//! AsyncAPI 2.x и 3.x → операции Kafka (спека L1 §5.2): `publish`/`send` → `kafka_produce`,
//! `subscribe`/`receive` → `kafka_consume`. Сообщение produce лежит в `request`, consume — в `response`.

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use serde_json::Value;

use crate::summary::{Resolver, summarize};
use crate::{ContractInput, ParseError, ParseOptions, ParseResult, line_of, parse_document};

/// Снимает цепочку `$ref` (в v3 сообщение канала может ссылаться на компонент).
fn deref(r: &Resolver, file: &str, node: &Value) -> (String, Value) {
    let (mut file, mut node) = (file.to_string(), node.clone());
    for _ in 0..5 {
        let Some(reference) = node.get("$ref").and_then(Value::as_str).map(String::from) else { break };
        match r.resolve(&file, &reference) {
            Some((f, n)) => {
                file = f;
                node = n;
            }
            None => return (file, Value::Null),
        }
    }
    (file, node)
}

fn message_summary(r: &Resolver, file: &str, message: &Value, out: &mut ParseResult, path: &str) -> Option<SchemaSummary> {
    let (mfile, message) = deref(r, file, message);
    if let Some(variants) = message.get("oneOf").and_then(Value::as_array) {
        out.warn(path, None, "message oneOf: the first variant is taken");
        return message_summary(r, &mfile, variants.first()?, out, path);
    }
    let payload = message.get("payload")?;
    let mut s = summarize("message", payload, false, &mfile, r, 1, &mut vec![]);
    s.limit();
    Some(s)
}

fn push(out: &mut ParseResult, opts: &ParseOptions, kind: OpKind, topic: &str, name: &str, summary: &str, line: Option<u32>, path: &str, message: Option<SchemaSummary>) {
    let mut d = OperationDraft::new(&opts.service, kind, Address::Kafka { topic: topic.into() }, Source { kind: SourceKind::Asyncapi, path: path.into(), line });
    d.name = name.into();
    d.summary = summary.into();
    match kind {
        OpKind::KafkaProduce => d.request = message,
        _ => d.response = message,
    }
    out.operations.push(d);
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let doc = parse_document(input.bytes)?;
    let text = String::from_utf8_lossy(input.bytes);
    let Some(version) = doc.get("asyncapi").and_then(Value::as_str) else {
        return Err(ParseError { message: "not an AsyncAPI document (no asyncapi field)".into(), line: None });
    };
    let major: u32 = version.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    let r = Resolver::new(input.path, &doc, input.read);
    let path = input.path;
    let mut out = ParseResult::default();
    if let Some(servers) = doc.get("servers").and_then(Value::as_object) {
        for (name, s) in servers {
            let (_, s) = deref(&r, path, s);
            let protocol = str_of(&s, "protocol");
            if protocol != "kafka" && protocol != "kafka-secure" {
                out.warn(path, line_of(&text, &format!("{name}:")), format!("server {name}: protocol {protocol} is not Kafka, operations are still read as Kafka topics"));
            }
        }
    }
    match major {
        2 => {
            let Some(channels) = doc.get("channels").and_then(Value::as_object) else { return Ok(out) };
            for (channel, item) in channels {
                let (cfile, item) = deref(&r, path, item);
                let line = line_of(&text, &format!("{channel}:"));
                for (key, kind) in [("publish", OpKind::KafkaProduce), ("subscribe", OpKind::KafkaConsume)] {
                    let Some(op) = item.get(key) else { continue };
                    let name = op.get("operationId").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("{key} {channel}"));
                    let message = op.get("message").and_then(|m| message_summary(&r, &cfile, m, &mut out, path));
                    push(&mut out, opts, kind, channel, &name, str_of(op, "summary"), line, path, message);
                }
            }
        }
        3 => {
            let Some(operations) = doc.get("operations").and_then(Value::as_object) else { return Ok(out) };
            for (id, op) in operations {
                let (ofile, op) = deref(&r, path, op);
                let kind = match str_of(&op, "action") {
                    "send" => OpKind::KafkaProduce,
                    "receive" => OpKind::KafkaConsume,
                    other => {
                        out.warn(path, None, format!("operation {id}: unknown action {other:?}"));
                        continue;
                    }
                };
                let Some(channel_ref) = op.get("channel") else {
                    out.warn(path, None, format!("operation {id}: no channel"));
                    continue;
                };
                let (cfile, channel) = deref(&r, &ofile, channel_ref);
                let key = channel_ref.get("$ref").and_then(Value::as_str).and_then(|s| s.rsplit('/').next()).unwrap_or("");
                let address = channel.get("address").and_then(Value::as_str).unwrap_or(key);
                let message = op
                    .get("messages")
                    .and_then(Value::as_array)
                    .and_then(|m| m.first())
                    .map(|m| (ofile.clone(), m.clone()))
                    .or_else(|| channel.get("messages").and_then(Value::as_object).and_then(|m| m.values().next()).map(|m| (cfile.clone(), m.clone())))
                    .and_then(|(f, m)| message_summary(&r, &f, &m, &mut out, path));
                push(&mut out, opts, kind, address, id, str_of(&op, "summary"), line_of(&text, &format!("{id}:")), path, message);
            }
        }
        _ => return Err(ParseError { message: format!("unsupported AsyncAPI version {version}"), line: None }),
    }
    out.warnings.extend(r.take_warnings());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2: &str = include_str!("../../../dev/fixtures/orders-asyncapi/asyncapi-v2.yaml");
    const V3: &str = include_str!("../../../dev/fixtures/orders-asyncapi/asyncapi-v3.yaml");

    fn opts() -> ParseOptions {
        ParseOptions { service: "orders".into(), ..Default::default() }
    }
    fn names(s: &SchemaSummary) -> Vec<&str> {
        s.children.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn v2_publish_is_produce_and_subscribe_is_consume() {
        let read = |_: &str| None;
        let out = parse(&ContractInput { path: "asyncapi.yaml", bytes: V2.as_bytes(), read: &read }, &opts()).unwrap();
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["kafka_produce:orders.created", "kafka_consume:payments.completed"]);
        let p = &out.operations[0];
        assert_eq!((p.name.as_str(), p.summary.as_str(), p.source.line), ("publishOrderCreated", "Order created", Some(7)));
        assert_eq!(names(p.request.as_ref().unwrap()), ["orderId", "total"]);
        assert!(p.response.is_none());
        let c = &out.operations[1];
        assert_eq!(names(c.response.as_ref().unwrap()), ["paymentId"]);
        let msgs: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("amqp")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("oneOf")), "{msgs:?}");
    }

    #[test]
    fn v3_send_receive_with_channel_addresses() {
        let read = |_: &str| None;
        let out = parse(&ContractInput { path: "asyncapi.yaml", bytes: V3.as_bytes(), read: &read }, &opts()).unwrap();
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["kafka_consume:billing.invoices", "kafka_produce:orders.created"]);
        let invoice = &out.operations[0];
        assert_eq!(invoice.name, "receiveInvoice");
        assert_eq!(names(invoice.response.as_ref().unwrap()), ["invoiceId", "amount"], "цепочка $ref канал → компонент");
        let created = &out.operations[1];
        assert_eq!(created.summary, "Emit order created");
        assert_eq!(names(created.request.as_ref().unwrap()), ["orderId"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn other_documents_are_errors() {
        let read = |_: &str| None;
        assert!(parse(&ContractInput { path: "x.yaml", bytes: b"openapi: 3.0.0\n", read: &read }, &opts()).is_err());
        assert!(parse(&ContractInput { path: "x.yaml", bytes: b"asyncapi: 1.2.0\n", read: &read }, &opts()).is_err());
    }
}
```

- [ ] **Step 3: `schema.rs`**

```rust
//! Файлы схем без AsyncAPI: Avro `.avsc` и JSON Schema (спека L1 §5.2, решение плана 6).
//! Топик — из `kafka.topics` манифеста, иначе из имени файла (`topic_guessed`).

use std::collections::BTreeMap;

use glafira_load_model::project::Direction;
use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::{MAX_DEPTH, SchemaSummary};
use serde_json::Value;

use crate::summary::{Resolver, summarize};
use crate::{ContractInput, ParseError, ParseOptions, ParseResult, parse_document};

fn avro_primitive(t: &str) -> Option<&'static str> {
    Some(match t {
        "null" => "null",
        "boolean" => "boolean",
        "int" | "long" => "integer",
        "float" | "double" => "number",
        "bytes" | "fixed" => "bytes",
        "string" => "string",
        _ => return None,
    })
}

/// Полное имя именованного типа Avro: `namespace.name`, если namespace задан (у вложенных — унаследован).
fn full_name(schema: &Value, namespace: &str) -> String {
    let name = schema.get("name").and_then(Value::as_str).unwrap_or("");
    let ns = schema.get("namespace").and_then(Value::as_str).unwrap_or(namespace);
    if name.contains('.') || ns.is_empty() { name.into() } else { format!("{ns}.{name}") }
}

fn avro(name: &str, schema: &Value, required: bool, namespace: &str, named: &mut BTreeMap<String, Value>, depth: usize, stack: &mut Vec<String>) -> SchemaSummary {
    match schema {
        Value::String(t) => {
            if let Some(k) = avro_primitive(t) {
                return SchemaSummary::leaf(name, k).required(required);
            }
            let fq = if t.contains('.') || namespace.is_empty() { t.clone() } else { format!("{namespace}.{t}") };
            let Some(def) = named.get(&fq).or_else(|| named.get(t)).cloned() else {
                return SchemaSummary::leaf(name, format!("unresolved:{t}")).required(required);
            };
            if stack.contains(&fq) {
                return SchemaSummary::leaf(name, format!("ref:{}", fq.rsplit('.').next().unwrap_or(&fq))).required(required);
            }
            stack.push(fq);
            let out = avro(name, &def, required, namespace, named, depth, stack);
            stack.pop();
            out
        }
        Value::Array(union) => {
            let members: Vec<&Value> = union.iter().filter(|m| m.as_str() != Some("null")).collect();
            let nullable = members.len() != union.len();
            match members.as_slice() {
                [one] => avro(name, one, required && !nullable, namespace, named, depth, stack),
                _ => {
                    let mut s = SchemaSummary::leaf(name, "one_of").required(required && !nullable);
                    s.children = members.iter().enumerate().map(|(i, m)| avro(&format!("variant{}", i + 1), m, false, namespace, named, depth + 1, stack)).collect();
                    s
                }
            }
        }
        Value::Object(_) => {
            let t = schema.get("type").cloned().unwrap_or(Value::Null);
            let mut out = SchemaSummary::leaf(name, "any").required(required);
            if depth >= MAX_DEPTH {
                out.truncated = true;
                return out;
            }
            match t.as_str() {
                Some("record") => {
                    let fq = full_name(schema, namespace);
                    named.insert(fq.clone(), schema.clone());
                    let ns = schema.get("namespace").and_then(Value::as_str).unwrap_or(namespace);
                    out.kind = "object".into();
                    stack.push(fq);
                    if let Some(fields) = schema.get("fields").and_then(Value::as_array) {
                        for f in fields {
                            let fname = f.get("name").and_then(Value::as_str).unwrap_or("?");
                            let ftype = f.get("type").cloned().unwrap_or(Value::Null);
                            let req = f.get("default").is_none();
                            let mut child = avro(fname, &ftype, req, ns, named, depth + 1, stack);
                            if let Some(d) = f.get("default").filter(|d| !d.is_null()) {
                                child.example = Some(d.to_string());
                            }
                            out.children.push(child);
                        }
                    }
                    stack.pop();
                }
                Some("enum") => {
                    named.insert(full_name(schema, namespace), schema.clone());
                    out.kind = "enum".into();
                    out.enum_values = schema.get("symbols").and_then(Value::as_array).map(|s| s.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
                }
                Some("array") => {
                    let items = schema.get("items").cloned().unwrap_or(Value::Null);
                    out = SchemaSummary::array(name, avro("items", &items, false, namespace, named, depth + 1, stack)).required(required);
                }
                Some("map") => {
                    let values = schema.get("values").cloned().unwrap_or(Value::Null);
                    out.kind = "map".into();
                    out.children = vec![avro("value", &values, false, namespace, named, depth + 1, stack)];
                }
                Some("fixed") => {
                    named.insert(full_name(schema, namespace), schema.clone());
                    out.kind = "bytes".into();
                }
                Some(_) => return avro(name, &t, required, namespace, named, depth, stack),
                None => {}
            }
            out
        }
        _ => SchemaSummary::leaf(name, "any").required(required),
    }
}

fn file_stem(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.split_once('.').map(|(s, _)| s).unwrap_or(base)
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let doc = parse_document(input.bytes)?;
    let mut out = ParseResult::default();
    let is_avro = input.path.ends_with(".avsc") || doc.get("type").and_then(Value::as_str) == Some("record");
    let mut summary = if is_avro {
        avro("message", &doc, false, "", &mut BTreeMap::new(), 1, &mut vec![])
    } else {
        let r = Resolver::new(input.path, &doc, input.read);
        let s = summarize("message", &doc, false, input.path, &r, 1, &mut vec![]);
        out.warnings.extend(r.take_warnings());
        s
    };
    summary.limit();
    let (topic, direction, guessed) = match opts.topics.get(input.path) {
        Some(b) => (b.topic.clone(), b.direction, false),
        None => (file_stem(input.path).to_string(), Direction::Produce, true),
    };
    if guessed {
        out.warn(input.path, None, format!("topic {topic:?} is guessed from the file name: add the file to kafka.topics of the service"));
    }
    let kind = match direction {
        Direction::Produce => OpKind::KafkaProduce,
        Direction::Consume => OpKind::KafkaConsume,
    };
    let mut d = OperationDraft::new(&opts.service, kind, Address::Kafka { topic: topic.clone() }, Source { kind: SourceKind::Schema, path: input.path.into(), line: Some(1) });
    d.name = topic;
    d.topic_guessed = guessed;
    match kind {
        OpKind::KafkaProduce => d.request = Some(summary),
        _ => d.response = Some(summary),
    }
    out.operations.push(d);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TopicBinding;

    const AVSC: &str = include_str!("../../../dev/fixtures/schemas/orders-created.avsc");
    const JSON_SCHEMA: &str = include_str!("../../../dev/fixtures/schemas/payment.schema.json");

    fn child<'a>(s: &'a SchemaSummary, name: &str) -> &'a SchemaSummary {
        s.children.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn avro_without_binding_guesses_the_topic() {
        let read = |_: &str| None;
        let out = parse(&ContractInput { path: "schemas/orders-created.avsc", bytes: AVSC.as_bytes(), read: &read }, &ParseOptions { service: "orders".into(), ..Default::default() }).unwrap();
        let op = &out.operations[0];
        assert_eq!((op.id.as_str(), op.topic_guessed), ("kafka_produce:orders-created", true));
        assert!(out.warnings[0].message.contains("guessed"));
        let m = op.request.as_ref().unwrap();
        assert!(child(m, "orderId").required);
        let total = child(m, "total");
        assert_eq!((total.kind.as_str(), total.required), ("number", false));
        assert_eq!(child(m, "status").enum_values, ["NEW", "PAID"]);
        assert_eq!(child(&child(m, "items").children[0], "qty").kind, "integer");
        assert_eq!(child(m, "labels").children[0].kind, "string");
        assert_eq!(child(m, "parent").kind, "ref:OrderCreated");
        assert_eq!(child(m, "amount").kind, "bytes");
    }

    #[test]
    fn binding_sets_topic_and_direction() {
        let read = |_: &str| None;
        let mut opts = ParseOptions { service: "orders".into(), ..Default::default() };
        opts.topics.insert("schemas/orders-created.avsc".into(), TopicBinding { topic: "orders.created".into(), direction: Direction::Consume });
        let out = parse(&ContractInput { path: "schemas/orders-created.avsc", bytes: AVSC.as_bytes(), read: &read }, &opts).unwrap();
        let op = &out.operations[0];
        assert_eq!((op.id.as_str(), op.topic_guessed), ("kafka_consume:orders.created", false));
        assert!(op.request.is_none() && op.response.is_some());
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn json_schema_files_use_the_generic_summarizer() {
        let read = |_: &str| None;
        let out = parse(&ContractInput { path: "schemas/payment.schema.json", bytes: JSON_SCHEMA.as_bytes(), read: &read }, &ParseOptions { service: "pay".into(), ..Default::default() }).unwrap();
        let op = &out.operations[0];
        assert_eq!(op.id, "kafka_produce:payment");
        let m = op.request.as_ref().unwrap();
        assert!(child(m, "paymentId").required && !child(m, "amount").required);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p glafira-load-contracts && cargo clippy -p glafira-load-contracts --all-targets -- -D warnings`
Expected: тесты `asyncapi` (3) и `schema` (3) PASS. В v2-тесте строка `orders.created:` — 7-я строка фикстуры.

- [ ] **Step 5: Commit**

```bash
git add crates/glafira-load-contracts dev/fixtures
git commit -m "feat(contracts): импортёры AsyncAPI 2/3 и схем сообщений Avro и JSON Schema

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Импортёр SQL-миграций (Flyway, golang-migrate, Liquibase)

**Files:**
- Create: `crates/glafira-load-contracts/src/sql.rs` (заменяет заглушку)
- Create: `dev/fixtures/flyway/db/migration/{V1__init.sql,V2__add_column.sql}`, `dev/fixtures/liquibase/db/changelog/{changelog.xml,002.sql}`
- Test: `sql.rs` (`mod tests`)

**Interfaces:**
- Produces: `sql::parse` → `ParseResult.tables` (операций не создаёт), предупреждения на нераспознанные выражения.

- [ ] **Step 1: Фикстуры**

`dev/fixtures/flyway/db/migration/V1__init.sql`:

```sql
-- Заказы
CREATE TABLE orders (
    id uuid PRIMARY KEY,
    status text NOT NULL,
    total numeric(12, 2),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX orders_status_idx ON orders (status);

CREATE OR REPLACE FUNCTION touch() RETURNS trigger AS $$
BEGIN
  NEW.updated_at := now(); RETURN NEW;
END;
$$ LANGUAGE plpgsql;
```

`dev/fixtures/flyway/db/migration/V2__add_column.sql`:

```sql
ALTER TABLE orders ADD COLUMN note text;
ALTER TABLE orders DROP COLUMN total;
CREATE TABLE audit_log (id bigserial PRIMARY KEY, payload jsonb);
```

`dev/fixtures/liquibase/db/changelog/changelog.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<databaseChangeLog xmlns="http://www.liquibase.org/xml/ns/dbchangelog">
  <changeSet id="1" author="demo">
    <createTable tableName="invoices">
      <column name="id" type="bigint"><constraints primaryKey="true" nullable="false"/></column>
      <column name="amount" type="decimal(12,2)"><constraints nullable="false"/></column>
      <column name="note" type="varchar(255)"/>
    </createTable>
  </changeSet>
  <changeSet id="2" author="demo">
    <addColumn tableName="invoices">
      <column name="paid_at" type="timestamp"/>
    </addColumn>
    <sqlFile path="002.sql" relativeToChangelogFile="true"/>
  </changeSet>
  <include file="other.xml" relativeToChangelogFile="true"/>
</databaseChangeLog>
```

`dev/fixtures/liquibase/db/changelog/002.sql`:

```sql
--liquibase formatted sql
--changeset demo:3
CREATE TABLE payments (id bigint NOT NULL, invoice_id bigint NOT NULL, amount decimal(12,2));
```

- [ ] **Step 2: `sql.rs`**

```rust
//! SQL-миграции → справочник таблиц (спека L1 §5.2, решение плана 10 про Liquibase).
//! Операций `sql` не создаёт: их задаёт человек, а таблицы нужны как подсказки.

use sqlparser::ast::{AlterTableOperation, ColumnOption, Statement};
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::{Column, ContractInput, ParseError, ParseOptions, ParseResult, Table};

/// Разбивает текст на выражения по `;` вне одинарных кавычек и `$$ … $$`, с номером строки начала.
fn split_statements(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let (mut cur, mut start_line, mut line) = (String::new(), 1u32, 1u32);
    let (mut in_quote, mut in_dollar, mut in_comment) = (false, false, false);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            in_comment = false;
        }
        if !in_quote && !in_dollar && !in_comment && c == '-' && chars.get(i + 1) == Some(&'-') {
            in_comment = true;
        }
        if !in_comment {
            if c == '\'' && !in_dollar {
                in_quote = !in_quote;
            } else if c == '$' && chars.get(i + 1) == Some(&'$') && !in_quote {
                in_dollar = !in_dollar;
                cur.push('$');
                i += 1;
            }
        }
        if c == ';' && !in_quote && !in_dollar && !in_comment {
            if !cur.trim().is_empty() {
                out.push((start_line, cur.trim().to_string()));
            }
            cur.clear();
            start_line = line;
        } else {
            if cur.trim().is_empty() && !c.is_whitespace() {
                start_line = line;
            }
            cur.push(c);
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push((start_line, cur.trim().to_string()));
    }
    out
}

fn parse_one(sql: &str) -> Result<Vec<Statement>, String> {
    let dialects: [&dyn Dialect; 3] = [&PostgreSqlDialect {}, &MySqlDialect {}, &GenericDialect {}];
    let mut last = String::new();
    for d in dialects {
        match Parser::parse_sql(d, sql) {
            Ok(s) => return Ok(s),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn table_mut<'a>(tables: &'a mut Vec<Table>, name: &str) -> &'a mut Table {
    if let Some(i) = tables.iter().position(|t| t.name == name) {
        return &mut tables[i];
    }
    tables.push(Table { name: name.into(), columns: vec![] });
    tables.last_mut().expect("just pushed")
}

fn column_of(def: &sqlparser::ast::ColumnDef) -> Column {
    let nullable = !def.options.iter().any(|o| matches!(o.option, ColumnOption::NotNull | ColumnOption::Unique { is_primary: true, .. }));
    Column { name: def.name.value.clone(), data_type: def.data_type.to_string(), nullable }
}

fn apply(stmt: &Statement, tables: &mut Vec<Table>) -> bool {
    match stmt {
        Statement::CreateTable(ct) => {
            let t = table_mut(tables, &ct.name.to_string());
            t.columns = ct.columns.iter().map(column_of).collect();
            true
        }
        Statement::AlterTable { name, operations, .. } => {
            for op in operations {
                match op {
                    AlterTableOperation::AddColumn { column_def, .. } => table_mut(tables, &name.to_string()).columns.push(column_of(column_def)),
                    AlterTableOperation::DropColumn { column_name, .. } => {
                        if let Some(t) = tables.iter_mut().find(|t| t.name == name.to_string()) {
                            t.columns.retain(|c| &c.name != &column_name.value);
                        }
                    }
                    _ => {}
                }
            }
            true
        }
        _ => false,
    }
}

fn parse_sql_text(text: &str, path: &str, out: &mut ParseResult) {
    for (line, stmt) in split_statements(text) {
        match parse_one(&stmt) {
            Ok(statements) => {
                for s in &statements {
                    apply(s, &mut out.tables);
                }
            }
            Err(e) => out.warn(path, Some(line), format!("statement not parsed: {e}")),
        }
    }
}

fn parse_liquibase_xml(text: &str, path: &str, out: &mut ParseResult) -> Result<(), ParseError> {
    let doc = roxmltree::Document::parse(text).map_err(|e| ParseError { message: e.to_string(), line: Some(e.pos().row) })?;
    for node in doc.descendants().filter(|n| n.is_element()) {
        let line = Some(doc.text_pos_at(node.range().start).row);
        let columns = |n: roxmltree::Node| -> Vec<Column> {
            n.children()
                .filter(|c| c.has_tag_name("column"))
                .map(|c| Column {
                    name: c.attribute("name").unwrap_or("").into(),
                    data_type: c.attribute("type").unwrap_or("").into(),
                    nullable: c.children().find(|k| k.has_tag_name("constraints")).map(|k| k.attribute("nullable") != Some("false") && k.attribute("primaryKey") != Some("true")).unwrap_or(true),
                })
                .collect()
        };
        match node.tag_name().name() {
            "createTable" => {
                let name = node.attribute("tableName").unwrap_or("");
                table_mut(&mut out.tables, name).columns = columns(node);
            }
            "addColumn" => {
                let name = node.attribute("tableName").unwrap_or("");
                table_mut(&mut out.tables, name).columns.extend(columns(node));
            }
            "dropColumn" => {
                if let (Some(t), Some(c)) = (node.attribute("tableName"), node.attribute("columnName"))
                    && let Some(t) = out.tables.iter_mut().find(|x| x.name == t)
                {
                    t.columns.retain(|x| x.name != c);
                }
            }
            "include" | "includeAll" => out.warn(path, line, format!("{} {}: included files are scanned as separate contracts", node.tag_name().name(), node.attribute("file").or(node.attribute("path")).unwrap_or(""))),
            "sqlFile" => out.warn(path, line, format!("sqlFile {}: add the file as a separate sql contract", node.attribute("path").unwrap_or(""))),
            _ => {}
        }
    }
    Ok(())
}

pub fn parse(input: &ContractInput, _opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let text = std::str::from_utf8(input.bytes).map_err(|e| ParseError { message: format!("not UTF-8: {e}"), line: None })?;
    let mut out = ParseResult::default();
    let lower = input.path.to_ascii_lowercase();
    if lower.ends_with(".xml") {
        parse_liquibase_xml(text, input.path, &mut out)?;
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") || lower.ends_with(".json") {
        out.warn(input.path, None, "Liquibase YAML/JSON changelogs are not supported: use XML or formatted SQL");
    } else {
        parse_sql_text(text, input.path, &mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = include_str!("../../../dev/fixtures/flyway/db/migration/V1__init.sql");
    const V2: &str = include_str!("../../../dev/fixtures/flyway/db/migration/V2__add_column.sql");
    const XML: &str = include_str!("../../../dev/fixtures/liquibase/db/changelog/changelog.xml");
    const FORMATTED: &str = include_str!("../../../dev/fixtures/liquibase/db/changelog/002.sql");

    fn run(path: &str, text: &str) -> ParseResult {
        let read = |_: &str| None;
        parse(&ContractInput { path, bytes: text.as_bytes(), read: &read }, &ParseOptions::default()).unwrap()
    }

    fn cols(t: &Table) -> Vec<(&str, &str, bool)> {
        t.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable)).collect()
    }

    #[test]
    fn flyway_create_table_and_dollar_quoted_function() {
        let out = run("db/migration/V1__init.sql", V1);
        assert_eq!(out.tables.len(), 1);
        let orders = &out.tables[0];
        assert_eq!(orders.name, "orders");
        let c = cols(orders);
        assert_eq!(c[0].0, "id");
        assert!(!c[0].2, "primary key не nullable");
        assert_eq!((c[1].0, c[1].2), ("status", false));
        assert_eq!((c[2].0, c[2].2), ("total", true));
        assert!(c[3].1.to_ascii_lowercase().contains("timestamp"));
        assert!(out.warnings.len() <= 1, "функция с $$ либо разобрана, либо одно предупреждение: {:?}", out.warnings);
        if let Some(w) = out.warnings.first() {
            assert_eq!(w.line, Some(11), "предупреждение указывает на строку CREATE FUNCTION");
        }
    }

    #[test]
    fn alter_table_add_and_drop_columns() {
        let out = run("db/migration/V2__add_column.sql", V2);
        let orders = out.tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(cols(orders), vec![("note", "TEXT", true)]);
        assert!(out.tables.iter().any(|t| t.name == "audit_log"));
    }

    #[test]
    fn liquibase_xml_and_formatted_sql() {
        let out = run("db/changelog/changelog.xml", XML);
        let inv = out.tables.iter().find(|t| t.name == "invoices").unwrap();
        assert_eq!(cols(inv), vec![("id", "bigint", false), ("amount", "decimal(12,2)", false), ("note", "varchar(255)", true), ("paid_at", "timestamp", true)]);
        let msgs: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.starts_with("sqlFile 002.sql")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.starts_with("include other.xml")), "{msgs:?}");
        let out = run("db/changelog/002.sql", FORMATTED);
        assert_eq!(out.tables[0].name, "payments");
        assert_eq!(out.tables[0].columns.len(), 3);
        let out = run("db/changelog/x.yaml", "databaseChangeLog: []");
        assert!(out.tables.is_empty() && out.warnings[0].message.contains("not supported"));
    }

    #[test]
    fn bad_statement_warns_with_its_line_and_does_not_stop() {
        let out = run("m.sql", "CREATE TABLE a (id int);\n\nTHIS IS NOT SQL;\nCREATE TABLE b (id int);");
        assert_eq!(out.tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(out.warnings[0].line, Some(3));
    }
}
```

Если установленная версия `sqlparser` называет варианты иначе (`Statement::CreateTable` как структурный вариант появился в 0.47; `ColumnOption::Unique { is_primary, .. }` — с 0.40), поправить `apply` и `column_of` по документации версии из `Cargo.lock`; смысл тестов не меняется. Имена типов данных приходят как `to_string()` AST (`TEXT`, `NUMERIC(12,2)`) — тесты сравнивают их без учёта регистра там, где это важно.

- [ ] **Step 3: Run tests**

Run: `cargo test -p glafira-load-contracts sql && cargo clippy -p glafira-load-contracts --all-targets -- -D warnings`
Expected: 4 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/glafira-load-contracts/src/sql.rs dev/fixtures/flyway dev/fixtures/liquibase
git commit -m "feat(contracts): таблицы из миграций Flyway, golang-migrate и Liquibase

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Сканер аннотаций Spring (Java и Kotlin)

**Files:**
- Create: `crates/glafira-load-contracts/src/spring.rs` (заменяет заглушку)
- Create: `dev/fixtures/spring-java/pom.xml`, `dev/fixtures/spring-java/src/main/java/demo/{OrderController,OrderListener}.java`, `dev/fixtures/spring-kotlin/build.gradle.kts`, `dev/fixtures/spring-kotlin/src/main/kotlin/demo/PaymentController.kt`
- Test: `spring.rs` (`mod tests`)

**Interfaces:**
- Produces: `spring::parse` → операции `http` (по `@*Mapping`), `kafka_consume` (`@KafkaListener`), `kafka_produce` (`<…kafka…|…template…>.send("topic"`), предупреждения «path not computed» для путей через константы.

- [ ] **Step 1: Фикстуры**

`dev/fixtures/spring-java/pom.xml`:

```xml
<project><modelVersion>4.0.0</modelVersion><groupId>demo</groupId><artifactId>orders</artifactId><version>1</version></project>
```

`dev/fixtures/spring-java/src/main/java/demo/OrderController.java`:

```java
package demo;

import org.springframework.web.bind.annotation.*;

@RestController
@RequestMapping("/v1/orders")
public class OrderController {

    @PostMapping
    public OrderDto create(@RequestBody CreateOrderRequest request) {
        return null;
    }

    @GetMapping("/{id}")
    public OrderDto get(@PathVariable("id") String id, @RequestParam(required = false) String expand) {
        return null;
    }

    @RequestMapping(method = RequestMethod.DELETE, path = "/{id}")
    public void delete(@PathVariable String id) {
    }

    @GetMapping(Paths.SEARCH)
    public java.util.List<OrderDto> search() {
        return null;
    }
}
```

`dev/fixtures/spring-java/src/main/java/demo/OrderListener.java`:

```java
package demo;

import org.springframework.kafka.annotation.KafkaListener;
import org.springframework.kafka.core.KafkaTemplate;

public class OrderListener {
    private final KafkaTemplate<String, String> kafkaTemplate = null;

    @KafkaListener(topics = {"orders.created", "orders.paid"}, groupId = "audit")
    public void onOrder(String payload) {
        kafkaTemplate.send("orders.audit", payload);
    }
}
```

`dev/fixtures/spring-kotlin/build.gradle.kts`:

```kotlin
plugins { kotlin("jvm") version "2.0.0" }
```

`dev/fixtures/spring-kotlin/src/main/kotlin/demo/PaymentController.kt`:

```kotlin
package demo

import org.springframework.web.bind.annotation.*

@RestController
@RequestMapping("/v1/payments")
class PaymentController {

    @PostMapping
    fun create(@RequestBody req: PaymentRequest): PaymentDto = TODO()

    @GetMapping("/{id}")
    fun get(@PathVariable id: String): PaymentDto = TODO()
}
```

- [ ] **Step 2: `spring.rs`**

```rust
//! Spring MVC/WebFlux и Spring Kafka по текстовым правилам, без компиляции (спека L1 §5.2):
//! единственная эвристика по коду в L1. Путь через константу — предупреждение, операция не создаётся.

use std::sync::LazyLock;

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use regex::Regex;

use crate::{ContractInput, ParseError, ParseOptions, ParseResult};

static CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:class|interface|object)\s+\w+").unwrap());
static MAPPING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@(Get|Post|Put|Delete|Patch|Request)Mapping\s*(?:\(((?:[^()]|\([^()]*\))*)\))?").unwrap());
static STRING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]*)""#).unwrap());
static NAMED_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b(?:value|path)\s*=\s*(?:\{\s*)?"([^"]*)""#).unwrap());
static METHOD_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"RequestMethod\.(\w+)").unwrap());
static JAVA_METHOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:public|private|protected|default)?\s*(?:static\s+|final\s+|synchronized\s+)*[\w<>\[\],.?\s]+?\s+(\w+)\s*\(").unwrap());
static KOTLIN_METHOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bfun\s+(?:<[^>]*>\s*)?(\w+)\s*\(").unwrap());
static BODY_JAVA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@RequestBody\s+(?:@\w+\s+)*(?:final\s+)?([\w.<>,? ]+?)\s+\w+\s*[,)]").unwrap());
static BODY_KOTLIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@RequestBody\s+\w+\s*:\s*([\w.<>,? ]+?)\s*[,)=]").unwrap());
static PATH_VAR_JAVA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@PathVariable(?:\s*\(\s*(?:value\s*=\s*|name\s*=\s*)?"(\w+)"[^)]*\))?\s+(?:final\s+)?[\w.<>]+\s+(\w+)"#).unwrap());
static PATH_VAR_KOTLIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@PathVariable(?:\s*\(\s*(?:value\s*=\s*|name\s*=\s*)?"(\w+)"[^)]*\))?\s+(\w+)\s*:"#).unwrap());
static QUERY_JAVA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@RequestParam(?:\s*\(([^)]*)\))?\s+(?:final\s+)?[\w.<>]+\s+(\w+)"#).unwrap());
static QUERY_KOTLIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"@RequestParam(?:\s*\(([^)]*)\))?\s+(\w+)\s*:"#).unwrap());
static LISTENER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@KafkaListener\s*\(((?:[^()]|\([^()]*\))*)\)").unwrap());
static TOPICS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\btopics\s*=\s*(\{[^}]*\}|\[[^\]]*\]|"[^"]*")"#).unwrap());
static SEND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\w*(?:kafka|template)\w*\s*\.\s*send\s*\(\s*"([^"]+)""#).unwrap());

fn line_at(text: &str, pos: usize) -> u32 {
    text[..pos].matches('\n').count() as u32 + 1
}

fn join(base: &str, path: &str) -> String {
    let b = base.trim_end_matches('/');
    let p = path.trim();
    if p.is_empty() {
        return if b.is_empty() { "/".into() } else { b.into() };
    }
    format!("{b}/{}", p.trim_start_matches('/'))
}

/// Путь из аргументов аннотации: строковый литерал (позиционный, `value=`, `path=`) или `None`,
/// если аргументы есть, а литерала нет (константа).
fn path_of(args: Option<&str>) -> Result<String, ()> {
    let Some(args) = args else { return Ok(String::new()) };
    if let Some(m) = NAMED_PATH.captures(args) {
        return Ok(m[1].to_string());
    }
    let trimmed = args.trim();
    if let Some(m) = STRING.captures(trimmed)
        && (trimmed.starts_with('"') || trimmed.starts_with("{\"") || trimmed.starts_with("[\""))
    {
        return Ok(m[1].to_string());
    }
    if trimmed.is_empty() || trimmed.starts_with("method") || trimmed.starts_with("consumes") || trimmed.starts_with("produces") {
        return Ok(String::new());
    }
    Err(())
}

/// Сигнатура метода после аннотации: имя и текст параметров до тела.
fn signature(text: &str, from: usize, kotlin: bool) -> Option<(String, &str, usize)> {
    let tail = &text[from..];
    let m = if kotlin { KOTLIN_METHOD.captures(tail)? } else { JAVA_METHOD.captures(tail)? };
    let name = m[1].to_string();
    let open = m.get(0)?.end() - 1;
    let mut depth = 0i32;
    for (i, c) in tail[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((name, &tail[open..open + i + 1], from + open));
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let text = std::str::from_utf8(input.bytes).map_err(|e| ParseError { message: format!("not UTF-8: {e}"), line: None })?;
    let kotlin = input.path.ends_with(".kt");
    let mut out = ParseResult::default();
    let class_at = CLASS.find(text).map(|m| m.start()).unwrap_or(0);
    let base = MAPPING.captures_iter(text).take_while(|c| c.get(0).map(|m| m.start() < class_at).unwrap_or(false)).find(|c| &c[1] == "Request").map(|c| path_of(c.get(2).map(|m| m.as_str()))).transpose();
    let base = match base {
        Ok(b) => b.unwrap_or_default(),
        Err(()) => {
            out.warn(input.path, Some(1), "class-level @RequestMapping path is not a string literal: paths not computed");
            return Ok(out);
        }
    };
    for cap in MAPPING.captures_iter(text) {
        let whole = cap.get(0).expect("match");
        if whole.start() < class_at {
            continue;
        }
        let line = line_at(text, whole.start());
        let args = cap.get(2).map(|m| m.as_str());
        let method = match &cap[1] {
            "Request" => match args.and_then(|a| METHOD_ATTR.captures(a)) {
                Some(m) => m[1].to_ascii_uppercase(),
                None => {
                    out.warn(input.path, Some(line), "@RequestMapping without method: operation skipped");
                    continue;
                }
            },
            verb => verb.to_ascii_uppercase(),
        };
        let path = match path_of(args) {
            Ok(p) => join(&base, &p),
            Err(()) => {
                out.warn(input.path, Some(line), "mapping path is not a string literal (constant?): path not computed");
                continue;
            }
        };
        let Some((name, params, _)) = signature(text, whole.end(), kotlin) else {
            out.warn(input.path, Some(line), "method signature after the mapping was not recognized");
            continue;
        };
        let mut d = OperationDraft::new(&opts.service, OpKind::Http, Address::Http { method, path }, Source { kind: SourceKind::Spring, path: input.path.into(), line: Some(line) });
        d.name = name;
        let mut children = Vec::new();
        let (pv, qv, body) = if kotlin { (&*PATH_VAR_KOTLIN, &*QUERY_KOTLIN, &*BODY_KOTLIN) } else { (&*PATH_VAR_JAVA, &*QUERY_JAVA, &*BODY_JAVA) };
        for m in pv.captures_iter(params) {
            let n = m.get(1).map(|x| x.as_str()).unwrap_or(&m[2]);
            children.push(SchemaSummary::leaf(format!("path.{n}"), "string").required(true));
        }
        for m in qv.captures_iter(params) {
            let attrs = m.get(1).map(|x| x.as_str()).unwrap_or("");
            let n = NAMED_PATH.captures(attrs).map(|c| c[1].to_string()).unwrap_or_else(|| m[2].to_string());
            let required = !attrs.contains("required = false") && !attrs.contains("required=false") && !attrs.contains("defaultValue");
            children.push(SchemaSummary::leaf(format!("query.{n}"), "string").required(required));
        }
        if let Some(m) = body.captures(params) {
            children.push(SchemaSummary::leaf("body", m[1].trim()).required(true));
        }
        if !children.is_empty() {
            d.request = Some(SchemaSummary::object("request", children));
        }
        out.operations.push(d);
    }
    for cap in LISTENER.captures_iter(text) {
        let line = line_at(text, cap.get(0).expect("match").start());
        let Some(t) = TOPICS.captures(&cap[1]) else {
            out.warn(input.path, Some(line), "@KafkaListener without literal topics: operation skipped");
            continue;
        };
        for topic in STRING.captures_iter(&t[1]) {
            let mut d = OperationDraft::new(&opts.service, OpKind::KafkaConsume, Address::Kafka { topic: topic[1].into() }, Source { kind: SourceKind::Spring, path: input.path.into(), line: Some(line) });
            d.name = signature(text, cap.get(0).expect("match").end(), kotlin).map(|s| s.0).unwrap_or_else(|| topic[1].to_string());
            out.operations.push(d);
        }
    }
    for cap in SEND.captures_iter(text) {
        let line = line_at(text, cap.get(0).expect("match").start());
        let d = OperationDraft::new(&opts.service, OpKind::KafkaProduce, Address::Kafka { topic: cap[1].into() }, Source { kind: SourceKind::Spring, path: input.path.into(), line: Some(line) });
        out.operations.push(d);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER: &str = include_str!("../../../dev/fixtures/spring-java/src/main/java/demo/OrderController.java");
    const LISTENER_SRC: &str = include_str!("../../../dev/fixtures/spring-java/src/main/java/demo/OrderListener.java");
    const KOTLIN: &str = include_str!("../../../dev/fixtures/spring-kotlin/src/main/kotlin/demo/PaymentController.kt");

    fn run(path: &str, text: &str) -> ParseResult {
        let read = |_: &str| None;
        parse(&ContractInput { path, bytes: text.as_bytes(), read: &read }, &ParseOptions { service: "orders".into(), ..Default::default() }).unwrap()
    }

    fn names(s: &SchemaSummary) -> Vec<(&str, &str, bool)> {
        s.children.iter().map(|c| (c.name.as_str(), c.kind.as_str(), c.required)).collect()
    }

    #[test]
    fn java_controller_with_class_prefix_and_constant_path() {
        let out = run("src/main/java/demo/OrderController.java", CONTROLLER);
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["http:orders:POST /v1/orders", "http:orders:GET /v1/orders/{id}", "http:orders:DELETE /v1/orders/{id}"]);
        let create = &out.operations[0];
        assert_eq!((create.name.as_str(), create.source.line), ("create", Some(9)));
        assert_eq!(names(create.request.as_ref().unwrap()), vec![("body", "CreateOrderRequest", true)]);
        let get = &out.operations[1];
        assert_eq!(names(get.request.as_ref().unwrap()), vec![("path.id", "string", true), ("query.expand", "string", false)]);
        assert_eq!(out.operations[2].name, "delete");
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].line, Some(23));
        assert!(out.warnings[0].message.contains("not a string literal"));
    }

    #[test]
    fn kafka_listener_and_template_send() {
        let out = run("src/main/java/demo/OrderListener.java", LISTENER_SRC);
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["kafka_consume:orders.created", "kafka_consume:orders.paid", "kafka_produce:orders.audit"]);
        assert_eq!(out.operations[0].name, "onOrder");
        assert_eq!(out.operations[2].source.line, Some(11));
    }

    #[test]
    fn kotlin_controller() {
        let out = run("src/main/kotlin/demo/PaymentController.kt", KOTLIN);
        assert_eq!(out.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["http:orders:POST /v1/payments", "http:orders:GET /v1/payments/{id}"]);
        assert_eq!(names(out.operations[0].request.as_ref().unwrap()), vec![("body", "PaymentRequest", true)]);
        assert_eq!(names(out.operations[1].request.as_ref().unwrap()), vec![("path.id", "string", true)]);
        assert_eq!(out.operations[1].name, "get");
    }

    #[test]
    fn files_without_mappings_give_nothing() {
        let out = run("src/main/java/demo/Util.java", "package demo;\npublic class Util { public static int x() { return 1; } }\n");
        assert!(out.operations.is_empty() && out.warnings.is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p glafira-load-contracts spring && cargo clippy -p glafira-load-contracts --all-targets -- -D warnings`
Expected: 4 PASS. Номера строк: `@PostMapping` — строка 9 фикстуры, `@GetMapping(Paths.SEARCH)` — 23, `kafkaTemplate.send` — 11.

- [ ] **Step 4: Commit**

```bash
git add crates/glafira-load-contracts/src/spring.rs dev/fixtures/spring-java dev/fixtures/spring-kotlin
git commit -m "feat(contracts): сканер аннотаций Spring MVC и Spring Kafka для Java и Kotlin

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Крейт `glafira-load-xlsx` — выгрузка и чтение шаблона ПРОМ

**Files:**
- Modify: `crates/glafira-load-xlsx/Cargo.toml`
- Create: `crates/glafira-load-xlsx/src/{lib,export,import}.rs`
- Test: `crates/glafira-load-xlsx/src/lib.rs` (`mod tests`, round-trip через временный файл)

**Interfaces:**
- Consumes: `glafira_load_model::prod_load::{Figures, TemplateRow, Cell, SourceInfo, preview_import}`, `registry::OpKind`.
- Produces: `glafira_load_xlsx::{TemplateLabels, TemplateRowOut, TemplateRead, TemplateError, write_template, read_template, MARKER_PREFIX, TEMPLATE_FORMAT}`.

- [ ] **Step 1: Зависимости**

```toml
[dependencies]
glafira-load-model = { path = "../glafira-load-model" }
serde = { version = "1", features = ["derive"] }
rust_xlsxwriter = "0.93"
calamine = "0.31"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

(`cargo add rust_xlsxwriter calamine` даёт текущие версии; API ниже — `Workbook`, `Worksheet::{write_string_with_format, write_number_with_format, write_blank, set_column_width, add_data_validation, set_freeze_panes, autofilter, protect, insert_note}`, `Format::{set_bold, set_background_color, set_locked, set_unlocked}`, `DataValidation::{allow_decimal_number}`, `Note::new`; у calamine — `open_workbook_auto_from_rs`, `Reader::{sheet_names, worksheet_range}`, `Data`.)

- [ ] **Step 2: `lib.rs`**

```rust
//! Excel-шаблон нагрузки ПРОМ (спека L1 §7.1–§7.2): запись и чтение. Что в прочитанном
//! валидно — решает `glafira_load_model::prod_load::preview_import`.

pub mod export;
pub mod import;

use serde::{Deserialize, Serialize};

pub use export::{TemplateRowOut, write_template};
pub use import::{TemplateRead, read_template};

/// A1 листа «Параметры»: `glafira-load:prod-load:<формат>:<проект>`.
pub const MARKER_PREFIX: &str = "glafira-load:prod-load:";
pub const TEMPLATE_FORMAT: u32 = 1;
/// Колонки A–L листа «Операции».
pub const COLUMNS: usize = 12;
/// Строки B3..B7 листа «Параметры»: стенд, период с, период по, система, кто заполнил.
pub const PARAMS: usize = 5;

/// Подписи листов и колонок на языке интерфейса (решение плана 5: приходят из фронтенда).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateLabels {
    pub sheet_operations: String,
    pub sheet_params: String,
    pub sheet_help: String,
    /// Ровно `COLUMNS` заголовков A–L.
    pub columns: Vec<String>,
    /// Примечания к заголовкам (пустая строка — без примечания), по одному на колонку.
    pub notes: Vec<String>,
    /// Ровно `PARAMS` подписей листа «Параметры».
    pub params: Vec<String>,
    pub help_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Format(String),
    #[error("the file has no Glafira marker in the parameters sheet")]
    NoMarker,
    #[error("unsupported template format {0}")]
    BadFormat(u32),
}

impl From<rust_xlsxwriter::XlsxError> for TemplateError {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        TemplateError::Io(e.to_string())
    }
}

impl From<calamine::Error> for TemplateError {
    fn from(e: calamine::Error) -> Self {
        TemplateError::Format(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::prod_load::{Cell, Figures, SourceInfo, preview_import};
    use glafira_load_model::registry::OpKind;
    use std::collections::BTreeMap;

    pub(crate) fn labels() -> TemplateLabels {
        TemplateLabels {
            sheet_operations: "Операции".into(),
            sheet_params: "Параметры".into(),
            sheet_help: "Справка".into(),
            columns: ["ID", "Сервис", "Тип", "Операция", "Описание", "Среднее RPS", "Пиковое RPS", "Доля ошибок, %", "Размер запроса, байт", "Размер ответа, байт", "p95 ПРОМ, мс", "Комментарий"].map(String::from).to_vec(),
            notes: vec![String::new(); COLUMNS],
            params: ["Стенд ПРОМ", "Период с", "Период по", "Источник цифр", "Заполнил"].map(String::from).to_vec(),
            help_lines: vec!["Заполните колонки F–K.".into(), "ID менять нельзя.".into()],
        }
    }

    #[test]
    fn export_then_import_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.xlsx");
        let rows = vec![
            TemplateRowOut { id: "http:orders:POST /v1/orders".into(), service: "orders".into(), kind: "http".into(), name: "createOrder".into(), summary: "Create".into(), figures: Some(Figures { avg: 120.0, peak: 340.0, error_pct: Some(0.2), req_bytes: Some(850), resp_bytes: Some(420), p95_ms: Some(180.0), lag_max_s: None, comment: "ok".into() }) },
            TemplateRowOut { id: "kafka_consume:orders.created".into(), service: "orders".into(), kind: "kafka_consume".into(), name: "orders.created".into(), summary: String::new(), figures: Some(Figures { avg: 10.0, peak: 20.0, error_pct: None, req_bytes: Some(1200), resp_bytes: None, p95_ms: None, lag_max_s: Some(5.0), comment: String::new() }) },
            TemplateRowOut { id: "http:orders:GET /v1/orders/{id}".into(), service: "orders".into(), kind: "http".into(), name: "getOrder".into(), summary: String::new(), figures: None },
        ];
        write_template(&path, "orders-platform", &rows, &labels()).unwrap();
        let read = read_template(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((read.format, read.project.as_str()), (TEMPLATE_FORMAT, "orders-platform"));
        assert_eq!(read.rows.len(), 3);
        assert_eq!(read.rows[0].row, 2);
        assert_eq!(read.rows[0].avg, Cell::Number { value: 120.0 });
        assert_eq!(read.rows[0].comment, "ok");
        assert_eq!(read.rows[1].p95_ms, Cell::Number { value: 5.0 }, "лаг consume лежит в последней числовой колонке");
        assert_eq!(read.rows[2].avg, Cell::Empty);
        let known: BTreeMap<String, OpKind> = [("http:orders:POST /v1/orders", OpKind::Http), ("kafka_consume:orders.created", OpKind::KafkaConsume), ("http:orders:GET /v1/orders/{id}", OpKind::Http)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let p = preview_import(&read.rows, SourceInfo::default(), &known, None);
        assert!(p.invalid.is_empty(), "{:?}", p.invalid);
        let r = p.result.unwrap();
        assert_eq!(r.operations["http:orders:POST /v1/orders"], rows[0].figures.clone().unwrap());
        assert_eq!(r.operations["kafka_consume:orders.created"].lag_max_s, Some(5.0));
        assert_eq!(p.empty, vec!["http:orders:GET /v1/orders/{id}"]);
    }

    #[test]
    fn files_without_marker_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.xlsx");
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet();
        ws.write_string(0, 0, "ID").unwrap();
        wb.add_worksheet();
        wb.save(&path).unwrap();
        assert_eq!(read_template(&std::fs::read(&path).unwrap()).unwrap_err(), TemplateError::NoMarker);
        assert!(matches!(read_template(b"not an xlsx").unwrap_err(), TemplateError::Format(_)));
    }
}
```

- [ ] **Step 3: `export.rs`**

```rust
use std::path::Path;

use glafira_load_model::prod_load::Figures;
use rust_xlsxwriter::{Color, DataValidation, DataValidationRule, Format, Note, Workbook, Worksheet};

use crate::{COLUMNS, MARKER_PREFIX, PARAMS, TEMPLATE_FORMAT, TemplateError, TemplateLabels};

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRowOut {
    pub id: String,
    pub service: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
    /// Текущие цифры (решение плана 4): шаблон можно выгрузить заполненным.
    pub figures: Option<Figures>,
}

const WIDTHS: [f64; COLUMNS] = [40.0, 14.0, 14.0, 28.0, 36.0, 14.0, 14.0, 14.0, 18.0, 18.0, 14.0, 40.0];

fn check_labels(labels: &TemplateLabels) -> Result<(), TemplateError> {
    if labels.columns.len() != COLUMNS || labels.notes.len() != COLUMNS || labels.params.len() != PARAMS {
        return Err(TemplateError::Format(format!("labels: {} columns, {} notes, {} params expected", COLUMNS, COLUMNS, PARAMS)));
    }
    Ok(())
}

fn number(ws: &mut Worksheet, row: u32, col: u16, v: Option<f64>, f: &Format) -> Result<(), TemplateError> {
    match v {
        Some(v) => ws.write_number_with_format(row, col, v, f)?,
        None => ws.write_blank(row, col, f)?,
    };
    Ok(())
}

pub fn write_template(path: &Path, project: &str, rows: &[TemplateRowOut], labels: &TemplateLabels) -> Result<(), TemplateError> {
    check_labels(labels)?;
    let mut wb = Workbook::new();
    let header = Format::new().set_bold().set_background_color(Color::RGB(0xDDE3EA)).set_locked();
    let locked = Format::new().set_background_color(Color::RGB(0xF2F4F7)).set_locked();
    let unlocked = Format::new().set_unlocked();
    {
        let ws = wb.add_worksheet();
        ws.set_name(&labels.sheet_operations)?;
        for (c, title) in labels.columns.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, title, &header)?;
            ws.set_column_width(c as u16, WIDTHS[c])?;
            if !labels.notes[c].is_empty() {
                ws.insert_note(0, c as u16, &Note::new(&labels.notes[c]))?;
            }
        }
        for (i, r) in rows.iter().enumerate() {
            let row = (i + 1) as u32;
            for (c, v) in [&r.id, &r.service, &r.kind, &r.name, &r.summary].into_iter().enumerate() {
                ws.write_string_with_format(row, c as u16, v, &locked)?;
            }
            let f = r.figures.as_ref();
            let consume = r.kind == "kafka_consume";
            number(ws, row, 5, f.map(|f| f.avg), &unlocked)?;
            number(ws, row, 6, f.map(|f| f.peak), &unlocked)?;
            number(ws, row, 7, f.and_then(|f| f.error_pct), &unlocked)?;
            number(ws, row, 8, f.and_then(|f| f.req_bytes).map(|v| v as f64), &unlocked)?;
            number(ws, row, 9, f.and_then(|f| f.resp_bytes).map(|v| v as f64), &unlocked)?;
            number(ws, row, 10, f.and_then(|f| if consume { f.lag_max_s } else { f.p95_ms }), &unlocked)?;
            ws.write_string_with_format(row, 11, f.map(|f| f.comment.as_str()).unwrap_or(""), &unlocked)?;
        }
        let last = rows.len() as u32;
        if last > 0 {
            let non_negative = DataValidation::new().allow_decimal_number(DataValidationRule::GreaterThanOrEqualTo(0.0));
            for c in [5u16, 6, 8, 9, 10] {
                ws.add_data_validation(1, c, last, c, &non_negative)?;
            }
            let percent = DataValidation::new().allow_decimal_number(DataValidationRule::Between(0.0, 100.0));
            ws.add_data_validation(1, 7, last, 7, &percent)?;
            ws.autofilter(0, 0, last, (COLUMNS - 1) as u16)?;
        }
        ws.set_freeze_panes(1, 0)?;
        ws.protect();
    }
    {
        let ws = wb.add_worksheet();
        ws.set_name(&labels.sheet_params)?;
        ws.write_string(0, 0, &format!("{MARKER_PREFIX}{TEMPLATE_FORMAT}:{project}"))?;
        ws.set_column_width(0, 24.0)?;
        ws.set_column_width(1, 40.0)?;
        for (i, label) in labels.params.iter().enumerate() {
            ws.write_string_with_format((i + 2) as u32, 0, label, &header)?;
            ws.write_blank((i + 2) as u32, 1, &unlocked)?;
        }
    }
    {
        let ws = wb.add_worksheet();
        ws.set_name(&labels.sheet_help)?;
        ws.set_column_width(0, 100.0)?;
        for (i, line) in labels.help_lines.iter().enumerate() {
            ws.write_string(i as u32, 0, line)?;
        }
    }
    wb.save(path)?;
    Ok(())
}
```

- [ ] **Step 4: `import.rs`**

```rust
use std::io::Cursor;

use calamine::{Data, Range, Reader, open_workbook_auto_from_rs};
use glafira_load_model::prod_load::{Cell, SourceInfo, TemplateRow};

use crate::{MARKER_PREFIX, PARAMS, TEMPLATE_FORMAT, TemplateError};

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRead {
    pub format: u32,
    pub project: String,
    pub rows: Vec<TemplateRow>,
    /// Лист «Параметры»; `imported_at` и `file` заполняет приложение.
    pub source: SourceInfo,
}

fn text(d: Option<&Data>) -> String {
    match d {
        Some(Data::String(s)) => s.trim().to_string(),
        Some(Data::Float(f)) => f.to_string(),
        Some(Data::Int(i)) => i.to_string(),
        Some(Data::Bool(b)) => b.to_string(),
        Some(Data::DateTime(dt)) => dt.as_datetime().map(|d| d.date().to_string()).unwrap_or_else(|| dt.to_string()),
        Some(Data::DateTimeIso(s)) | Some(Data::DurationIso(s)) => s.clone(),
        Some(Data::Error(e)) => format!("#{e:?}"),
        Some(Data::Empty) | None => String::new(),
    }
}

fn cell(d: Option<&Data>) -> Cell {
    match d {
        Some(Data::Float(f)) => Cell::Number { value: *f },
        Some(Data::Int(i)) => Cell::Number { value: *i as f64 },
        Some(Data::Empty) | None => Cell::Empty,
        Some(Data::String(s)) if s.trim().is_empty() => Cell::Empty,
        Some(Data::String(s)) => match s.trim().replace(',', ".").parse::<f64>() {
            Ok(v) => Cell::Number { value: v },
            Err(_) => Cell::Text { value: s.trim().to_string() },
        },
        other => Cell::Text { value: text(other) },
    }
}

fn marker(params: &Range<Data>) -> Result<(u32, String), TemplateError> {
    let a1 = text(params.get((0, 0)));
    let Some(rest) = a1.strip_prefix(MARKER_PREFIX) else { return Err(TemplateError::NoMarker) };
    let (format, project) = rest.split_once(':').ok_or(TemplateError::NoMarker)?;
    let format: u32 = format.parse().map_err(|_| TemplateError::NoMarker)?;
    if format != TEMPLATE_FORMAT {
        return Err(TemplateError::BadFormat(format));
    }
    Ok((format, project.to_string()))
}

pub fn read_template(bytes: &[u8]) -> Result<TemplateRead, TemplateError> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))?;
    let names = wb.sheet_names().to_vec();
    if names.len() < 2 {
        return Err(TemplateError::NoMarker);
    }
    let params = wb.worksheet_range(&names[1])?;
    let (format, project) = marker(&params)?;
    let ops = wb.worksheet_range(&names[0])?;
    let mut rows = Vec::new();
    for (i, r) in ops.rows().enumerate().skip(1) {
        let id = text(r.first());
        if id.is_empty() {
            continue;
        }
        rows.push(TemplateRow {
            row: (i + 1) as u32,
            id,
            avg: cell(r.get(5)),
            peak: cell(r.get(6)),
            error_pct: cell(r.get(7)),
            req_bytes: cell(r.get(8)),
            resp_bytes: cell(r.get(9)),
            p95_ms: cell(r.get(10)),
            comment: text(r.get(11)),
        });
    }
    let p = |i: usize| text(params.get((i + 2, 1)));
    let mut values: Vec<String> = (0..PARAMS).map(p).collect();
    let filled_by = values.pop().unwrap_or_default();
    let system = values.pop().unwrap_or_default();
    let period_to = values.pop().unwrap_or_default();
    let period_from = values.pop().unwrap_or_default();
    let stand = values.pop().unwrap_or_default();
    Ok(TemplateRead { format, project, rows, source: SourceInfo { stand, period_from, period_to, system, filled_by, imported_at: String::new(), file: String::new() } })
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p glafira-load-xlsx && cargo clippy -p glafira-load-xlsx --all-targets -- -D warnings`
Expected: 2 PASS. Если `calamine` возвращает `Data::Float` для целых, `cell` уже это учитывает. Открыть `t.xlsx` из временной папки в LibreOffice вручную один раз: лист «Операции» защищён, колонки F–L редактируются, заголовки закреплены, в A1 листа «Параметры» — маркер.

- [ ] **Step 6: Commit**

```bash
git add crates/glafira-load-xlsx Cargo.lock
git commit -m "feat(xlsx): выгрузка защищённого Excel-шаблона нагрузки ПРОМ и чтение заполненного

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Папка проекта, границы путей, версия из git, команды проекта

**Files:**
- Create: `src-tauri/src/project/{mod,paths,manifest}.rs`, `src-tauri/src/git/{mod,version}.rs`, `src-tauri/src/commands/project.rs`
- Modify: `src-tauri/src/lib.rs` (модули и `generate_handler!`), `src-tauri/src/commands/mod.rs`, `src-tauri/Cargo.toml` (`tokio` dev-dep)
- Test: unit-тесты в `paths.rs`, `manifest.rs`, `version.rs`, `commands/project.rs`

**Interfaces:**
- Consumes: `glafira_load_model::{from_yaml, to_yaml, project::*, registry::{Registry, REGISTRY_FILE}, prod_load::{ProdLoad, PROD_LOAD_FILE}, profile::{ProfileFile, PROFILE_FILE}}`, `storage::fs::atomic_write`, `settings::SettingsStore`.
- Produces: `project::paths::{resolve, inside, relative}`; `project::manifest::{ProjectDir, Project, RecentProject, create}` с методами `open`, `root`, `file`, `read_text`, `write_text`, `write_bytes`, `manifest`, `save_manifest`, `registry`, `save_registry`, `prod_load`, `save_prod_load`, `profile`, `save_profile`, `view`, `service_root`; `git::version::{GitVersion, describe, describe_with}`; команды `project_create`, `project_open`, `project_close`, `project_save`, `project_recent`, `open_path`.

- [ ] **Step 1: `project/paths.rs`**

```rust
//! Границы проекта и клонов: любой путь чтения или записи проверяется (спека L1 §3.3, §11).

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult, ErrorCode};

/// `rel` относительно `base`, абсолютный — как есть.
pub fn resolve(base: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

fn canonical(p: &Path) -> AppResult<PathBuf> {
    p.canonicalize().map_err(|e| AppError::from(e).with_details(p.display().to_string()))
}

/// Канонический `candidate`, если он внутри `base`. Несуществующий файл проверяется по родителю
/// (так проверяются пути записи).
pub fn inside(base: &Path, candidate: &Path) -> AppResult<PathBuf> {
    let base_c = canonical(base)?;
    let target = if candidate.exists() {
        canonical(candidate)?
    } else {
        let parent = candidate.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        canonical(parent)?.join(candidate.file_name().unwrap_or_default())
    };
    if target.starts_with(&base_c) {
        Ok(target)
    } else {
        Err(app_err!(
            ErrorCode::PathOutsideProject,
            "errors.project.outside",
            "Path {{path}} is outside {{base}}",
            path = candidate.display(),
            base = base.display()
        ))
    }
}

/// Путь относительно `base` с разделителем `/` (для манифеста и реестра); вне `base` — абсолютный.
pub fn relative(base: &Path, abs: &Path) -> String {
    match abs.strip_prefix(base) {
        Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        Err(_) => abs.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_accepts_children_and_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("a/b")).unwrap();
        std::fs::write(base.join("a/b/f.txt"), "x").unwrap();
        assert!(inside(base, &base.join("a/b/f.txt")).is_ok());
        assert!(inside(base, &base.join("a/new.txt")).is_ok(), "несуществующий файл внутри — ок");
        let e = inside(base, &base.join("a/../../outside.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
        assert_eq!(relative(base, &base.join("a/b/f.txt")), "a/b/f.txt");
        assert_eq!(resolve(base, "a"), base.join("a"));
        assert_eq!(resolve(base, "/abs"), PathBuf::from("/abs"));
    }
}
```

- [ ] **Step 2: `project/manifest.rs`**

```rust
//! Папка проекта НТ: манифест и файлы этапа L1 (спека L1 §3). Все записи — через `atomic_write`.

use std::path::{Path, PathBuf};

use glafira_load_model::prod_load::{PROD_LOAD_FILE, ProdLoad};
use glafira_load_model::profile::{PROFILE_FILE, ProfileFile};
use glafira_load_model::project::{MANIFEST_FILE, Manifest, ManifestProblem, validate};
use glafira_load_model::registry::{REGISTRY_FILE, Registry};
use glafira_load_model::{ModelError, from_yaml, to_yaml};
use serde::Serialize;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::paths::{inside, resolve};
use crate::storage::fs::atomic_write;

/// Проект для фронтенда: манифест, проблемы проверки и сервисы без папки клона.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub manifest: Manifest,
    pub problems: Vec<ManifestProblem>,
    /// Есть проблемы проверки: запись заблокирована, пока их не исправят в файле.
    pub read_only: bool,
    pub missing_services: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: Option<String>,
    pub missing: bool,
}

pub struct ProjectDir {
    root: PathBuf,
}

fn invalid(file: &str, e: ModelError) -> AppError {
    app_err!(
        ErrorCode::ProjectInvalid,
        "errors.project.invalidFile",
        "{{file}}: {{error}}",
        file = file,
        error = e.message
    )
    .with_details(e.line.map(|l| format!("line {l}")).unwrap_or_default())
}

impl ProjectDir {
    /// Папка с манифестом; без манифеста — `NOT_FOUND`.
    pub fn open(path: &str) -> AppResult<Self> {
        let root = PathBuf::from(path);
        if !root.join(MANIFEST_FILE).is_file() {
            return Err(app_err!(
                ErrorCode::NotFound,
                "errors.project.notFound",
                "No {{file}} in {{path}}",
                file = MANIFEST_FILE,
                path = path
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn file(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn read_text(&self, rel: &str) -> AppResult<Option<String>> {
        let p = inside(&self.root, &self.file(rel))?;
        match std::fs::read_to_string(&p) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::from(e).with_details(p.display().to_string())),
        }
    }

    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) -> AppResult<()> {
        let p = inside(&self.root, &self.file(rel))?;
        atomic_write(&p, bytes)
    }

    pub fn write_text(&self, rel: &str, text: &str) -> AppResult<()> {
        self.write_bytes(rel, text.as_bytes())
    }

    fn read_yaml<T: serde::de::DeserializeOwned>(&self, rel: &str) -> AppResult<Option<T>> {
        self.read_text(rel)?.map(|t| from_yaml(&t).map_err(|e| invalid(rel, e))).transpose()
    }

    pub fn manifest(&self) -> AppResult<Manifest> {
        self.read_yaml(MANIFEST_FILE)?.ok_or_else(|| app_err!(ErrorCode::NotFound, "errors.project.notFound", "No {{file}} in {{path}}", file = MANIFEST_FILE, path = self.root.display()))
    }

    pub fn save_manifest(&self, m: &Manifest) -> AppResult<()> {
        self.write_text(MANIFEST_FILE, &to_yaml(m))
    }

    pub fn registry(&self) -> AppResult<Registry> {
        Ok(self.read_yaml(REGISTRY_FILE)?.unwrap_or_default())
    }

    pub fn save_registry(&self, r: &Registry) -> AppResult<()> {
        self.write_text(REGISTRY_FILE, &to_yaml(r))
    }

    pub fn prod_load(&self) -> AppResult<ProdLoad> {
        Ok(self.read_yaml(PROD_LOAD_FILE)?.unwrap_or_default())
    }

    pub fn save_prod_load(&self, p: &ProdLoad) -> AppResult<()> {
        self.write_text(PROD_LOAD_FILE, &to_yaml(p))
    }

    pub fn profile(&self) -> AppResult<ProfileFile> {
        Ok(self.read_yaml(PROFILE_FILE)?.unwrap_or_default())
    }

    pub fn save_profile(&self, p: &ProfileFile) -> AppResult<()> {
        self.write_text(PROFILE_FILE, &to_yaml(p))
    }

    /// Папка клона сервиса: существует и является каталогом.
    pub fn service_root(&self, m: &Manifest, service_id: &str) -> AppResult<PathBuf> {
        let s = m.service(service_id).ok_or_else(|| app_err!(ErrorCode::NotFound, "errors.project.noService", "Service {{id}} is not in the manifest", id = service_id))?;
        let p = resolve(&self.root, &s.path);
        if !p.is_dir() {
            return Err(app_err!(ErrorCode::NotFound, "errors.project.serviceDirMissing", "Folder of service {{id}} not found: {{path}}", id = service_id, path = p.display()));
        }
        Ok(p)
    }

    pub fn view(&self) -> AppResult<Project> {
        let manifest = self.manifest()?;
        let problems = validate(&manifest);
        let missing_services = manifest.services.iter().filter(|s| !resolve(&self.root, &s.path).is_dir()).map(|s| s.id.clone()).collect();
        Ok(Project { path: self.root.to_string_lossy().into_owned(), read_only: !problems.is_empty(), manifest, problems, missing_services })
    }
}

/// Новый проект: папка не существует или пуста. Без `git init` — проект кладут в свой репозиторий.
pub fn create(path: &str, name: &str) -> AppResult<Project> {
    if name.trim().is_empty() {
        return Err(app_err!(ErrorCode::InvalidInput, "errors.project.emptyName", "Project name cannot be empty"));
    }
    let root = PathBuf::from(path);
    if root.exists() && std::fs::read_dir(&root)?.next().is_some() {
        return Err(app_err!(ErrorCode::InvalidInput, "errors.project.dirNotEmpty", "Folder {{path}} is not empty", path = path));
    }
    std::fs::create_dir_all(root.join("registry/scans"))?;
    std::fs::create_dir_all(root.join("load/imports"))?;
    atomic_write(&root.join(MANIFEST_FILE), to_yaml(&Manifest::new(name.trim())).as_bytes())?;
    atomic_write(&root.join(REGISTRY_FILE), to_yaml(&Registry::default()).as_bytes())?;
    atomic_write(&root.join(".gitignore"), b"runs/\n")?;
    ProjectDir::open(path)?.view()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_open_save_and_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        let p = create(path.to_str().unwrap(), "orders").unwrap();
        assert_eq!(p.manifest.name, "orders");
        assert!(!p.read_only && p.problems.is_empty());
        assert!(path.join(".gitignore").exists());
        assert!(create(path.to_str().unwrap(), "again").is_err(), "папка не пуста");
        assert_eq!(create(dir.path().join("q").to_str().unwrap(), "  ").unwrap_err().code, ErrorCode::InvalidInput);

        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        let mut m = d.manifest().unwrap();
        m.services.push(glafira_load_model::project::Service { id: "orders".into(), name: "Orders".into(), path: "../nope".into(), contracts: vec![], kafka: Default::default(), extra: Default::default() });
        d.save_manifest(&m).unwrap();
        let v = d.view().unwrap();
        assert_eq!(v.missing_services, vec!["orders"]);
        assert_eq!(d.service_root(&m, "orders").unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(d.registry().unwrap().operations.len(), 0);
        assert_eq!(d.profile().unwrap().profiles.len(), 3, "профиль по умолчанию без файла");

        std::fs::write(path.join(MANIFEST_FILE), "format: 1\nname: [\n").unwrap();
        let e = d.manifest().unwrap_err();
        assert_eq!(e.code, ErrorCode::ProjectInvalid);
        assert!(e.details.as_deref().unwrap_or("").starts_with("line "), "{e:?}");
        assert_eq!(ProjectDir::open(dir.path().join("missing").to_str().unwrap()).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn writes_stay_inside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        assert_eq!(d.write_text("../escape.txt", "no").unwrap_err().code, ErrorCode::PathOutsideProject);
        d.write_text("load/profile.yaml", "format: 1\n").unwrap();
        assert!(d.read_text("load/profile.yaml").unwrap().is_some());
    }
}
```

`project/mod.rs`:

```rust
pub mod manifest;
pub mod paths;
```

- [ ] **Step 3: `git/version.rs`**

```rust
//! Версия клона сервиса через системный `git` с таймаутом (спека L1 §4).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

pub const TIMEOUT: Duration = Duration::from_secs(5);
pub const UNVERSIONED: &str = "unversioned";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitVersion {
    pub version: String,
    pub commit: String,
    pub available: bool,
    /// Почему git недоступен: нет бинаря, не репозиторий, таймаут. Текст команды, не переводится.
    pub error: Option<String>,
}

impl GitVersion {
    pub fn unversioned(error: impl Into<String>) -> Self {
        Self { version: UNVERSIONED.into(), commit: String::new(), available: false, error: Some(error.into()) }
    }
}

async fn run(git: &OsStr, repo: &Path, args: &[&str]) -> Result<String, String> {
    let fut = tokio::process::Command::new(git).arg("-C").arg(repo).args(args).output();
    match tokio::time::timeout(TIMEOUT, fut).await {
        Err(_) => Err(format!("git timed out after {}s", TIMEOUT.as_secs())),
        Ok(Err(e)) => Err(format!("git not started: {e}")),
        Ok(Ok(out)) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

pub async fn describe_with(git: impl AsRef<OsStr>, repo: &Path) -> GitVersion {
    let git = git.as_ref();
    match run(git, repo, &["describe", "--tags", "--always", "--dirty"]).await {
        Ok(version) => {
            let commit = run(git, repo, &["rev-parse", "HEAD"]).await.unwrap_or_default();
            GitVersion { version, commit, available: true, error: None }
        }
        Err(e) => GitVersion::unversioned(e),
    }
}

pub async fn describe(repo: &Path) -> GitVersion {
    describe_with("git", repo).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn real_repo_tag_and_commit() {
        let dir = tempfile::tempdir().unwrap();
        let sh = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(dir.path()).args(args).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        };
        sh(&["init", "-q"]);
        sh(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"]);
        sh(&["tag", "v1.2.3"]);
        let v = describe(dir.path()).await;
        assert_eq!((v.version.as_str(), v.available, v.error), ("v1.2.3", true, None));
        assert_eq!(v.commit.len(), 40);
        std::fs::write(dir.path().join("x"), "dirty").unwrap();
        sh(&["add", "x"]);
        assert_eq!(describe(dir.path()).await.version, "v1.2.3-dirty");
    }

    #[tokio::test]
    async fn missing_binary_and_plain_folder_are_unversioned() {
        let dir = tempfile::tempdir().unwrap();
        let v = describe_with("glafira-no-such-git-binary", dir.path()).await;
        assert_eq!((v.version.as_str(), v.available), (UNVERSIONED, false));
        assert!(v.error.unwrap().contains("not started"));
        let v = describe(dir.path()).await;
        assert!(!v.available, "обычная папка — не репозиторий");
    }
}
```

`git/mod.rs`: `pub mod version;`. В `src-tauri/Cargo.toml` `[dev-dependencies]` добавить `tokio = { version = "1", features = ["rt", "macros"] }` (если уже есть — расширить features).

- [ ] **Step 4: `commands/project.rs`**

```rust
use glafira_load_model::project::{Manifest, validate};
use tauri::State;

use super::blocking;
use crate::error::{AppResult, ErrorCode};
use crate::project::manifest::{Project, ProjectDir, RecentProject, create};
use crate::project::paths::{inside, resolve};
use crate::state::AppState;

fn remember(st: &AppState, path: &str) -> AppResult<()> {
    st.settings.remember_project(path)
}

#[tauri::command]
pub async fn project_create(state: State<'_, AppState>, path: String, name: String) -> AppResult<Project> {
    let st = state.inner().clone();
    blocking(move || {
        let p = create(&path, &name)?;
        remember(&st, &p.path)?;
        Ok(p)
    })
    .await
}

#[tauri::command]
pub async fn project_open(state: State<'_, AppState>, path: String) -> AppResult<Project> {
    let st = state.inner().clone();
    blocking(move || {
        let p = ProjectDir::open(&path)?.view()?;
        remember(&st, &p.path)?;
        Ok(p)
    })
    .await
}

/// «Убрать из списка»: проект на диске не трогается.
#[tauri::command]
pub async fn project_close(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let st = state.inner().clone();
    blocking(move || st.settings.forget_project(&path)).await
}

#[tauri::command]
pub async fn project_save(path: String, manifest: Manifest) -> AppResult<Project> {
    blocking(move || {
        let problems = validate(&manifest);
        if !problems.is_empty() {
            return Err(app_err!(ErrorCode::InvalidInput, "errors.project.invalidManifest", "The manifest has {{count}} problem(s)", count = problems.len())
                .with_details(serde_json::to_string(&problems).unwrap_or_default()));
        }
        let d = ProjectDir::open(&path)?;
        d.save_manifest(&manifest)?;
        d.view()
    })
    .await
}

#[tauri::command]
pub async fn project_recent(state: State<'_, AppState>) -> AppResult<Vec<RecentProject>> {
    let st = state.inner().clone();
    blocking(move || {
        Ok(st
            .settings
            .projects()
            .recent
            .into_iter()
            .map(|path| match ProjectDir::open(&path).and_then(|d| d.manifest()) {
                Ok(m) => RecentProject { path, name: Some(m.name), missing: false },
                Err(_) => RecentProject { path, name: None, missing: true },
            })
            .collect())
    })
    .await
}

/// Открывает файл или папку системным средством, если путь внутри проекта или клона его сервиса.
#[tauri::command]
pub async fn open_path(app: tauri::AppHandle, project: String, path: String) -> AppResult<()> {
    let target = blocking(move || {
        let d = ProjectDir::open(&project)?;
        let m = d.manifest()?;
        let candidate = std::path::PathBuf::from(&path);
        let mut roots = vec![d.root().to_path_buf()];
        roots.extend(m.services.iter().map(|s| resolve(d.root(), &s.path)).filter(|p| p.is_dir()));
        roots.iter().find_map(|r| inside(r, &candidate).ok()).ok_or_else(|| {
            app_err!(ErrorCode::PathOutsideProject, "errors.project.outside", "Path {{path}} is outside {{base}}", path = candidate.display(), base = d.root().display())
        })
    })
    .await?;
    tauri_plugin_opener::OpenerExt::opener(&app).open_path(target.to_string_lossy(), None::<&str>).map_err(|e| crate::error::AppError::internal_with(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_rejects_invalid_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        let mut m = Manifest::new("x");
        m.services.push(glafira_load_model::project::Service { id: "Bad".into(), name: String::new(), path: ".".into(), contracts: vec![], kafka: Default::default(), extra: Default::default() });
        let e = project_save(path.to_string_lossy().into_owned(), m).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert!(e.details.unwrap().contains("bad_service_id"));
    }
}
```

В `commands/mod.rs` добавить `pub mod project;`; в `lib.rs` — `pub mod git; pub mod project;` и в `generate_handler!`: `commands::project::project_create, commands::project::project_open, commands::project::project_close, commands::project::project_save, commands::project::project_recent, commands::project::open_path`.

Словари (`en.json` → `errors.project`): `outside` «Path {{path}} is outside {{base}}», `invalidFile` «{{file}}: {{error}}», `notFound` «No {{file}} in {{path}}», `noService` «Service {{id}} is not in the manifest», `serviceDirMissing` «Folder of service {{id}} not found: {{path}}», `dirNotEmpty` «Folder {{path}} is not empty», `invalidManifest` «The manifest has {{count}} problem(s)»; `ru.json` — «Путь {{path}} вне {{base}}», «{{file}}: {{error}}», «В {{path}} нет {{file}}», «Сервиса {{id}} нет в манифесте», «Папка сервиса {{id}} не найдена: {{path}}», «Папка {{path}} не пуста», «В манифесте проблем: {{count}}».

- [ ] **Step 5: Run tests**

Run: `cargo test -p glafira-load-testing --lib && pnpm i18n:check`
Expected: тесты `project::paths`, `project::manifest`, `git::version` (нужен `git` в PATH), `commands::project` PASS; `i18n:check` подтверждает, что все ключи `app_err!` есть в обоих словарях.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(project): папка проекта с атомарной записью, границы путей, версия из git и команды проекта

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Обход клонов, разбор с прогрессом и отменой, команды реестра

**Files:**
- Create: `src-tauri/src/scan/{mod,discover,run}.rs`, `src-tauri/src/commands/{scan,registry}.rs`
- Modify: `src-tauri/Cargo.toml` (`globset`, `ignore`), `src-tauri/src/state.rs` (`scans`), `src-tauri/src/lib.rs`, `src-tauri/src/commands/mod.rs`
- Test: `discover.rs`, `run.rs`, `commands/registry.rs` (`mod tests`)

**Interfaces:**
- Consumes: `glafira_load_contracts::{parse, ContractInput, ParseOptions, TopicBinding, Diagnostic, Table}`, `glafira_load_model::{merge::merge, diff::{Scan, SCAN_FORMAT, registry_diff, RegistryDiff}, registry::*}`, `project::manifest::ProjectDir`, `git::version::GitVersion`.
- Produces: `scan::discover::discover(clone: &Path) -> Vec<ContractRef>`; `scan::run::{ScanRegistry, ScanProgress, ScanReport, scan_service_blocking, now_rfc3339, now_label}`; команды `service_discover_contracts`, `service_scan`, `scan_cancel`, `registry_list`, `registry_get`, `registry_update_manual`, `registry_add_manual`, `registry_delete`, `registry_scans`, `registry_diff`, `registry_tables`.

- [ ] **Step 1: Зависимости и состояние**

`src-tauri/Cargo.toml`, `[dependencies]`: `globset = "0.4"`, `ignore = "0.4"`.

`state.rs` — добавить поле `pub scans: Arc<crate::scan::run::ScanRegistry>` с `Arc::default()` в `new`.

- [ ] **Step 2: `scan/discover.rs`**

```rust
//! Автопоиск контрактов в клоне по маскам (спека L1 §5.1).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use glafira_load_model::project::{ContractKind, ContractRef};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

pub const SKIP_DIRS: [&str; 8] = [".git", "node_modules", "target", "build", "dist", "vendor", ".idea", ".gradle"];

const PATTERNS: [(ContractKind, &[&str]); 6] = [
    (ContractKind::Openapi, &["**/openapi*.{yaml,yml,json}", "**/swagger*.{yaml,yml,json}", "**/api-docs*.json"]),
    (ContractKind::Asyncapi, &["**/asyncapi*.{yaml,yml,json}"]),
    (ContractKind::Proto, &["**/*.proto"]),
    (ContractKind::Schema, &["**/*.avsc", "**/*.schema.json"]),
    (ContractKind::Sql, &["**/db/migration/**/*.sql", "**/migrations/**/*.sql", "**/db/changelog/**/*.{xml,yaml,yml,sql}"]),
    (ContractKind::Spring, &["**/src/main/java/**/*.java", "**/src/main/kotlin/**/*.kt"]),
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
    ["pom.xml", "build.gradle", "build.gradle.kts"].iter().any(|f| clone.join(f).is_file())
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

pub fn discover(clone: &Path) -> Vec<ContractRef> {
    let jvm = is_jvm_project(clone);
    let mut out = Vec::new();
    let walk = WalkBuilder::new(clone)
        .hidden(false)
        .git_ignore(true)
        .filter_entry(|e| e.file_name().to_str().map(|n| !SKIP_DIRS.contains(&n)).unwrap_or(true))
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let rel = crate::project::paths::relative(clone, entry.path());
        let Some((kind, _)) = SETS.iter().find(|(_, set)| set.is_match(&rel)) else { continue };
        if *kind == ContractKind::Spring && !jvm {
            continue;
        }
        let include = if *kind == ContractKind::Proto { proto_includes(&rel) } else { vec![] };
        out.push(ContractRef { kind: *kind, path: rel, include, enabled: true });
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
        let by_kind = |k: ContractKind| found.iter().filter(|c| c.kind == k).map(|c| c.path.as_str()).collect::<Vec<_>>();
        assert_eq!(by_kind(ContractKind::Openapi), ["orders-openapi/openapi.yaml", "orders-swagger/swagger.json"]);
        assert_eq!(by_kind(ContractKind::Asyncapi).len(), 2);
        assert_eq!(by_kind(ContractKind::Proto), ["orders-proto/proto/common/money.proto", "orders-proto/proto/orders/v1/orders.proto"]);
        assert_eq!(by_kind(ContractKind::Schema), ["schemas/orders-created.avsc", "schemas/payment.schema.json"]);
        assert_eq!(by_kind(ContractKind::Sql).len(), 4);
        assert!(by_kind(ContractKind::Spring).is_empty(), "pom.xml лежит не в корне fixtures");
        let proto = found.iter().find(|c| c.path.ends_with("orders.proto")).unwrap();
        assert_eq!(proto.include, ["", "orders-proto", "orders-proto/proto", "orders-proto/proto/orders", "orders-proto/proto/orders/v1"]);
        let spring = discover(&fixtures.join("spring-java"));
        assert_eq!(spring.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["src/main/java/demo/OrderController.java", "src/main/java/demo/OrderListener.java"]);
    }

    #[test]
    fn skips_service_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(dir.path().join("node_modules/x/openapi.yaml"), "openapi: 3.0.0").unwrap();
        std::fs::write(dir.path().join("openapi.yaml"), "openapi: 3.0.0").unwrap();
        assert_eq!(discover(dir.path()).len(), 1);
    }
}
```

- [ ] **Step 3: `scan/run.rs`**

```rust
//! Разбор сервиса: контракты → черновики → слияние → снимок (спека L1 §5.3, §6.3).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use glafira_load_contracts::{ContractInput, Diagnostic, ParseOptions, Table, TopicBinding, parse};
use glafira_load_model::diff::{SCAN_FORMAT, Scan};
use glafira_load_model::merge::{MergeSummary, merge};
use glafira_load_model::project::{ContractRef, Service};
use glafira_load_model::registry::{SCANS_DIR, Seen};
use glafira_load_model::to_yaml;
use serde::Serialize;

use crate::error::{AppResult, ErrorCode};
use crate::git::version::GitVersion;
use crate::project::manifest::ProjectDir;
use crate::project::paths::inside;
use crate::scan::discover::discover;
use crate::sync::MutexExt;

pub const TABLES_DIR: &str = "registry/tables";

/// Идущие разборы: ключ `<проект>|<сервис>`, значение — флаг отмены.
#[derive(Default)]
pub struct ScanRegistry {
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ScanRegistry {
    pub fn key(project: &str, service: &str) -> String {
        format!("{project}|{service}")
    }

    pub fn start(&self, key: &str) -> AppResult<Arc<AtomicBool>> {
        let mut r = self.running.lock_safe();
        if r.contains_key(key) {
            return Err(app_err!(ErrorCode::ScanBusy, "errors.scan.busy", "This service is already being scanned"));
        }
        let flag = Arc::new(AtomicBool::new(false));
        r.insert(key.into(), flag.clone());
        Ok(flag)
    }

    pub fn finish(&self, key: &str) {
        self.running.lock_safe().remove(key);
    }

    /// Ставит флаг отмены; `false` — такого разбора нет.
    pub fn cancel(&self, key: &str) -> bool {
        self.running.lock_safe().get(key).map(|f| f.store(true, Ordering::Relaxed)).is_some()
    }

    pub fn cancel_all(&self, project: &str) -> usize {
        self.running.lock_safe().iter().filter(|(k, _)| k.starts_with(&format!("{project}|"))).map(|(_, f)| f.store(true, Ordering::Relaxed)).count()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub service: String,
    pub done: usize,
    pub total: usize,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub service: String,
    pub git: GitVersion,
    pub contracts: usize,
    pub operations_found: usize,
    pub tables_found: usize,
    pub summary: MergeSummary,
    pub warnings: Vec<Diagnostic>,
    pub errors: Vec<Diagnostic>,
    /// `registry/scans/<метка>.yaml`; пусто, если разбор отменён.
    pub scan_file: String,
    pub cancelled: bool,
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

/// `ГГГГ-ММ-ДД-ЧЧММСС` для имён снимков и копий шаблонов.
pub fn now_label() -> String {
    let f = time::macros::format_description!("[year]-[month]-[day]-[hour][minute][second]");
    time::OffsetDateTime::now_utc().format(f).unwrap_or_default()
}

fn options(service: &Service, contract: &ContractRef) -> ParseOptions {
    let topics = service
        .kafka
        .topics
        .iter()
        .map(|(topic, s)| (s.schema().to_string(), TopicBinding { topic: topic.clone(), direction: s.direction() }))
        .collect::<BTreeMap<_, _>>();
    ParseOptions { service: service.id.clone(), topics, include: contract.include.clone() }
}

/// Блокирующий вызов: читает контракты, сливает реестр, пишет снимок и таблицы.
pub fn scan_service_blocking(dir: &ProjectDir, service_id: &str, git: GitVersion, progress: &(dyn Fn(ScanProgress) + Sync), cancel: &AtomicBool) -> AppResult<ScanReport> {
    let manifest = dir.manifest()?;
    let clone = dir.service_root(&manifest, service_id)?;
    let service = manifest.service(service_id).expect("checked by service_root").clone();
    let contracts: Vec<ContractRef> = if service.contracts.is_empty() { discover(&clone) } else { service.contracts.iter().filter(|c| c.enabled).cloned().collect() };
    let read = |rel: &str| -> Option<Vec<u8>> { inside(&clone, &clone.join(rel)).ok().and_then(|p| std::fs::read(p).ok()) };
    let (mut drafts, mut tables, mut warnings, mut errors) = (Vec::new(), Vec::<Table>::new(), Vec::new(), Vec::new());
    let total = contracts.len();
    let mut cancelled = false;
    for (i, c) in contracts.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        progress(ScanProgress { service: service_id.into(), done: i, total, file: c.path.clone() });
        let path = match inside(&clone, &clone.join(&c.path)) {
            Ok(p) => p,
            Err(e) => {
                errors.push(Diagnostic { path: c.path.clone(), line: None, message: e.message });
                continue;
            }
        };
        let Ok(bytes) = std::fs::read(&path) else {
            errors.push(Diagnostic { path: c.path.clone(), line: None, message: "file not readable".into() });
            continue;
        };
        let input = ContractInput { path: &c.path, bytes: &bytes, read: &read };
        match parse(c.kind, &input, &options(&service, c)) {
            Ok(r) => {
                drafts.extend(r.operations);
                tables.extend(r.tables);
                warnings.extend(r.warnings);
            }
            Err(e) => errors.push(Diagnostic { path: c.path.clone(), line: e.line, message: e.message }),
        }
    }
    progress(ScanProgress { service: service_id.into(), done: total, total, file: String::new() });
    if cancelled {
        return Ok(ScanReport { service: service_id.into(), git, contracts: total, operations_found: drafts.len(), tables_found: tables.len(), summary: MergeSummary::default(), warnings, errors, scan_file: String::new(), cancelled: true });
    }
    let seen = Seen { version: git.version.clone(), commit: git.commit.clone(), at: now_rfc3339() };
    let mut registry = dir.registry()?;
    let summary = merge(&mut registry, service_id, drafts.clone(), &seen);
    dir.save_registry(&registry)?;
    let scan = Scan { format: SCAN_FORMAT, at: seen.at.clone(), service: service_id.into(), version: seen.version.clone(), commit: seen.commit.clone(), summary: summary.clone(), fingerprints: registry.fingerprints() };
    let scan_file = format!("{SCANS_DIR}/{}.yaml", now_label());
    dir.write_text(&scan_file, &to_yaml(&scan))?;
    dir.write_text(&format!("{TABLES_DIR}/{service_id}.yaml"), &to_yaml(&tables))?;
    Ok(ScanReport { service: service_id.into(), git, contracts: total, operations_found: drafts.len(), tables_found: tables.len(), summary, warnings, errors, scan_file, cancelled: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest::create;
    use glafira_load_model::project::{ContractKind, ContractRef, Manifest, Service};
    use std::path::Path;

    fn project_with(service_path: &str, contracts: Vec<ContractRef>) -> (tempfile::TempDir, ProjectDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "t").unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        let mut m = Manifest::new("t");
        m.services.push(Service { id: "orders".into(), name: "Orders".into(), path: service_path.into(), contracts, kafka: Default::default(), extra: Default::default() });
        d.save_manifest(&m).unwrap();
        (dir, d)
    }

    fn fixtures() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../dev/fixtures").canonicalize().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn scan_merges_writes_snapshot_and_is_idempotent() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let seen = std::sync::Mutex::new(Vec::new());
        let progress = |p: ScanProgress| seen.lock().unwrap().push(p.done);
        let r = scan_service_blocking(&d, "orders", GitVersion::unversioned("test"), &progress, &AtomicBool::new(false)).unwrap();
        assert_eq!((r.contracts, r.operations_found, r.summary.added.len()), (1, 2, 2));
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(d.file(&r.scan_file).is_file());
        assert!(d.file("registry/tables/orders.yaml").is_file());
        assert_eq!(*seen.lock().unwrap(), vec![0, 1]);
        let text1 = d.read_text("registry/operations.yaml").unwrap().unwrap();
        let r2 = scan_service_blocking(&d, "orders", GitVersion::unversioned("test"), &|_| {}, &AtomicBool::new(false)).unwrap();
        assert!(r2.summary.is_empty());
        assert_eq!(d.read_text("registry/operations.yaml").unwrap().unwrap(), text1);
        assert_eq!(d.registry().unwrap().get("http:orders:POST /v1/orders").unwrap().last_seen.version, "unversioned");
    }

    #[test]
    fn explicit_contracts_outside_the_clone_and_broken_files_are_errors_not_panics() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![
            ContractRef { kind: ContractKind::Openapi, path: "../orders-swagger/swagger.json".into(), include: vec![], enabled: true },
            ContractRef { kind: ContractKind::Proto, path: "openapi.yaml".into(), include: vec![], enabled: true },
            ContractRef { kind: ContractKind::Sql, path: "nope.sql".into(), include: vec![], enabled: false },
        ]);
        let r = scan_service_blocking(&d, "orders", GitVersion::unversioned("test"), &|_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(r.contracts, 2, "выключенный контракт не считается");
        assert_eq!(r.errors.len(), 2);
        assert!(r.errors[0].message.contains("outside"));
        assert_eq!(r.operations_found, 0);
    }

    #[test]
    fn cancel_flag_stops_before_merging() {
        let (_tmp, d) = project_with(&format!("{}/orders-openapi", fixtures()), vec![]);
        let cancel = AtomicBool::new(true);
        let r = scan_service_blocking(&d, "orders", GitVersion::unversioned("test"), &|_| {}, &cancel).unwrap();
        assert!(r.cancelled && r.scan_file.is_empty());
        assert_eq!(d.registry().unwrap().operations.len(), 0);
    }

    #[test]
    fn registry_keys_and_busy() {
        let r = ScanRegistry::default();
        let k = ScanRegistry::key("/p", "s");
        let flag = r.start(&k).unwrap();
        assert_eq!(r.start(&k).unwrap_err().code, ErrorCode::ScanBusy);
        assert!(r.cancel(&k) && flag.load(Ordering::Relaxed));
        r.finish(&k);
        assert!(!r.cancel(&k));
    }
}
```

`scan/mod.rs`: `pub mod discover; pub mod run;`.

- [ ] **Step 4: `commands/scan.rs`**

```rust
use glafira_load_model::project::ContractRef;
use tauri::State;
use tauri::ipc::Channel;

use super::blocking;
use crate::error::AppResult;
use crate::project::manifest::ProjectDir;
use crate::scan::discover::discover;
use crate::scan::run::{ScanProgress, ScanRegistry, ScanReport, scan_service_blocking};
use crate::state::AppState;

#[tauri::command]
pub async fn service_discover_contracts(path: String, service_id: String) -> AppResult<Vec<ContractRef>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let m = d.manifest()?;
        Ok(discover(&d.service_root(&m, &service_id)?))
    })
    .await
}

/// Разбор одного сервиса или всех (`service_id = None`), по очереди; прогресс — в канал.
#[tauri::command]
pub async fn service_scan(state: State<'_, AppState>, path: String, service_id: Option<String>, on_progress: Channel<ScanProgress>) -> AppResult<Vec<ScanReport>> {
    let st = state.inner().clone();
    let ids: Vec<String> = match service_id {
        Some(id) => vec![id],
        None => {
            let p = path.clone();
            blocking(move || Ok(ProjectDir::open(&p)?.manifest()?.services.iter().map(|s| s.id.clone()).collect())).await?
        }
    };
    let mut reports = Vec::new();
    for id in ids {
        let key = ScanRegistry::key(&path, &id);
        let cancel = st.scans.start(&key)?;
        let (p, sid, ch, scans) = (path.clone(), id.clone(), on_progress.clone(), st.scans.clone());
        let git = {
            let d = ProjectDir::open(&path)?;
            let m = d.manifest()?;
            match d.service_root(&m, &id) {
                Ok(root) => crate::git::version::describe(&root).await,
                Err(e) => {
                    scans.finish(&key);
                    return Err(e);
                }
            }
        };
        let result = blocking(move || {
            let d = ProjectDir::open(&p)?;
            let progress = move |pr: ScanProgress| {
                let _ = ch.send(pr);
            };
            scan_service_blocking(&d, &sid, git, &progress, &cancel)
        })
        .await;
        st.scans.finish(&key);
        reports.push(result?);
    }
    Ok(reports)
}

#[tauri::command]
pub fn scan_cancel(state: State<'_, AppState>, path: String, service_id: Option<String>) -> AppResult<()> {
    match service_id {
        Some(id) => {
            state.scans.cancel(&ScanRegistry::key(&path, &id));
        }
        None => {
            state.scans.cancel_all(&path);
        }
    }
    Ok(())
}
```

- [ ] **Step 5: `commands/registry.rs`**

```rust
use std::collections::BTreeMap;

use glafira_load_contracts::Table;
use glafira_load_model::diff::{RegistryDiff, Scan, registry_diff};
use glafira_load_model::from_yaml;
use glafira_load_model::registry::{Manual, OpKind, Operation, OperationDraft, Registry, SCANS_DIR, Seen, Source, SourceKind, Status, operation_id};
use serde::{Deserialize, Serialize};

use super::blocking;
use crate::error::{AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::scan::run::{TABLES_DIR, now_rfc3339};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryFilter {
    pub service: Option<String>,
    pub kind: Option<OpKind>,
    pub source: Option<SourceKind>,
    pub status: Option<Status>,
    /// Подстрока `id` или имени без учёта регистра.
    pub query: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRow {
    pub id: String,
    pub service: String,
    pub kind: OpKind,
    pub name: String,
    pub summary: String,
    pub source: SourceKind,
    pub status: Status,
    pub last_version: String,
    pub topic_guessed: bool,
}

fn row(o: &Operation) -> OperationRow {
    OperationRow { id: o.draft.id.clone(), service: o.draft.service.clone(), kind: o.draft.kind, name: o.draft.name.clone(), summary: o.draft.summary.clone(), source: o.draft.source.kind, status: o.status, last_version: o.last_seen.version.clone(), topic_guessed: o.draft.topic_guessed }
}

fn matches(o: &Operation, f: &RegistryFilter) -> bool {
    f.service.as_ref().is_none_or(|s| &o.draft.service == s)
        && f.kind.is_none_or(|k| o.draft.kind == k)
        && f.source.is_none_or(|s| o.draft.source.kind == s)
        && f.status.is_none_or(|s| o.status == s)
        && f.query.as_ref().filter(|q| !q.trim().is_empty()).is_none_or(|q| {
            let q = q.trim().to_lowercase();
            o.draft.id.to_lowercase().contains(&q) || o.draft.name.to_lowercase().contains(&q)
        })
}

fn not_found(id: &str) -> crate::error::AppError {
    app_err!(ErrorCode::NotFound, "errors.registry.notFound", "Operation {{id}} not found", id = id)
}

#[tauri::command]
pub async fn registry_list(path: String, filter: RegistryFilter) -> AppResult<Vec<OperationRow>> {
    blocking(move || Ok(ProjectDir::open(&path)?.registry()?.operations.iter().filter(|o| matches(o, &filter)).map(row).collect())).await
}

#[tauri::command]
pub async fn registry_get(path: String, id: String) -> AppResult<Operation> {
    blocking(move || ProjectDir::open(&path)?.registry()?.get(&id).cloned().ok_or_else(|| not_found(&id))).await
}

#[tauri::command]
pub async fn registry_update_manual(path: String, id: String, manual: Manual) -> AppResult<Operation> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let mut r = d.registry()?;
        let op = r.get_mut(&id).ok_or_else(|| not_found(&id))?;
        op.manual = manual;
        let out = op.clone();
        d.save_registry(&r)?;
        Ok(out)
    })
    .await
}

/// Ручная операция: источник `manual`, `id` по правилам, версия — момент добавления.
#[tauri::command]
pub async fn registry_add_manual(path: String, draft: OperationDraft) -> AppResult<Operation> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let m = d.manifest()?;
        if m.service(&draft.service).is_none() {
            return Err(app_err!(ErrorCode::InvalidInput, "errors.project.noService", "Service {{id}} is not in the manifest", id = draft.service));
        }
        let mut draft = draft;
        draft.id = operation_id(&draft.service, draft.kind, &draft.address);
        draft.source = Source::manual();
        if draft.name.trim().is_empty() {
            draft.name = glafira_load_model::registry::address_label(&draft.address);
        }
        let mut r = d.registry()?;
        if r.get(&draft.id).is_some() {
            return Err(app_err!(ErrorCode::InvalidInput, "errors.registry.duplicate", "Operation {{id}} already exists", id = draft.id));
        }
        let seen = Seen { version: "manual".into(), commit: String::new(), at: now_rfc3339() };
        let op = Operation { fingerprint: draft.fingerprint(), draft, status: Status::Active, first_seen: seen.clone(), last_seen: seen, manual: Manual::default() };
        r.operations.push(op.clone());
        r.sort();
        d.save_registry(&r)?;
        Ok(op)
    })
    .await
}

#[tauri::command]
pub async fn registry_delete(path: String, id: String) -> AppResult<()> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let mut r = d.registry()?;
        if !r.remove(&id) {
            return Err(not_found(&id));
        }
        d.save_registry(&r)
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanMeta {
    pub file: String,
    pub at: String,
    pub service: String,
    pub version: String,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    pub restored: usize,
}

fn read_scan(d: &ProjectDir, file: &str) -> AppResult<Scan> {
    let text = d.read_text(file)?.ok_or_else(|| app_err!(ErrorCode::NotFound, "errors.registry.scanNotFound", "Scan {{file}} not found", file = file))?;
    from_yaml(&text).map_err(|e| app_err!(ErrorCode::ProjectInvalid, "errors.project.invalidFile", "{{file}}: {{error}}", file = file, error = e.message))
}

/// Снимки от новых к старым (имена файлов — метки времени).
#[tauri::command]
pub async fn registry_scans(path: String) -> AppResult<Vec<ScanMeta>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let dir = d.file(SCANS_DIR);
        let mut files: Vec<String> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd.flatten().filter_map(|e| e.file_name().to_str().filter(|n| n.ends_with(".yaml")).map(|n| format!("{SCANS_DIR}/{n}"))).collect(),
            Err(_) => vec![],
        };
        files.sort();
        files.reverse();
        let mut out = Vec::new();
        for file in files {
            match read_scan(&d, &file) {
                Ok(s) => out.push(ScanMeta { file, at: s.at, service: s.service, version: s.version, added: s.summary.added.len(), changed: s.summary.changed.len(), removed: s.summary.removed.len(), restored: s.summary.restored.len() }),
                Err(e) => tracing::warn!("scan {file} skipped: {}", e.message),
            }
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn registry_diff(path: String, from: String, to: String) -> AppResult<RegistryDiff> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        Ok(registry_diff(&read_scan(&d, &from)?, &read_scan(&d, &to)?))
    })
    .await
}

/// Таблицы из миграций по сервисам (подсказки для ручных `sql`-операций).
#[tauri::command]
pub async fn registry_tables(path: String) -> AppResult<BTreeMap<String, Vec<Table>>> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let mut out = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(d.file(TABLES_DIR)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(service) = name.strip_suffix(".yaml") else { continue };
                if let Some(text) = d.read_text(&format!("{TABLES_DIR}/{name}"))?
                    && let Ok(t) = from_yaml::<Vec<Table>>(&text)
                {
                    out.insert(service.to_string(), t);
                }
            }
        }
        Ok(out)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest::create;
    use glafira_load_model::project::{Manifest, Service};
    use glafira_load_model::registry::Address;

    async fn project() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").to_string_lossy().into_owned();
        create(&path, "t").unwrap();
        let d = ProjectDir::open(&path).unwrap();
        let mut m = Manifest::new("t");
        m.services.push(Service { id: "billing".into(), name: String::new(), path: ".".into(), contracts: vec![], kafka: Default::default(), extra: Default::default() });
        d.save_manifest(&m).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn manual_operations_are_added_listed_updated_and_deleted() {
        let (_tmp, path) = project().await;
        let mut draft = OperationDraft::new("billing", OpKind::Sql, Address::Sql { name: "invoices.insert".into() }, Source::manual());
        draft.id = "ignored".into();
        let op = registry_add_manual(path.clone(), draft.clone()).await.unwrap();
        assert_eq!(op.draft.id, "sql:billing:invoices.insert");
        assert_eq!(registry_add_manual(path.clone(), draft).await.unwrap_err().code, ErrorCode::InvalidInput, "дубликат");
        let rows = registry_list(path.clone(), RegistryFilter { query: Some("INVOICES".into()), ..Default::default() }).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(registry_list(path.clone(), RegistryFilter { kind: Some(OpKind::Http), ..Default::default() }).await.unwrap().is_empty());
        let updated = registry_update_manual(path.clone(), op.draft.id.clone(), Manual { tags: vec!["critical".into()], notes: "n".into(), owner: String::new() }).await.unwrap();
        assert_eq!(updated.manual.tags, ["critical"]);
        assert_eq!(registry_get(path.clone(), op.draft.id.clone()).await.unwrap().manual.notes, "n");
        registry_delete(path.clone(), op.draft.id.clone()).await.unwrap();
        assert_eq!(registry_get(path.clone(), op.draft.id).await.unwrap_err().code, ErrorCode::NotFound);
        assert!(registry_scans(path.clone()).await.unwrap().is_empty());
        assert!(registry_tables(path).await.unwrap().is_empty());
    }
}
```

В `commands/mod.rs`: `pub mod registry; pub mod scan;`. В `lib.rs`: `pub mod scan;` и команды в `generate_handler!`: `commands::scan::service_discover_contracts, commands::scan::service_scan, commands::scan::scan_cancel, commands::registry::registry_list, commands::registry::registry_get, commands::registry::registry_update_manual, commands::registry::registry_add_manual, commands::registry::registry_delete, commands::registry::registry_scans, commands::registry::registry_diff, commands::registry::registry_tables`.

Словари: `errors.scan.busy` «This service is already being scanned» / «Этот сервис уже разбирается»; `errors.registry.notFound` «Operation {{id}} not found» / «Операция {{id}} не найдена»; `errors.registry.duplicate` «Operation {{id}} already exists» / «Операция {{id}} уже есть»; `errors.registry.scanNotFound` «Scan {{file}} not found» / «Снимок {{file}} не найден».

- [ ] **Step 6: Run tests**

Run: `cargo test -p glafira-load-testing --lib && cargo clippy --workspace --all-targets -- -D warnings && pnpm i18n:check`
Expected: тесты `scan::discover` (2), `scan::run` (4), `commands::registry` (1) PASS; clippy и i18n чисто.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(scan): автопоиск контрактов, разбор сервиса с прогрессом и отменой, команды реестра

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: Команды шаблона ПРОМ, импорта и профиля

**Files:**
- Create: `src-tauri/src/load/{mod,template}.rs`, `src-tauri/src/commands/load.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/commands/mod.rs`
- Test: `load/template.rs`, `commands/load.rs` (`mod tests`)

**Interfaces:**
- Consumes: `glafira_load_xlsx::{write_template, read_template, TemplateLabels, TemplateRowOut, TemplateError}`, `glafira_load_model::prod_load::{preview_import, ImportPreview, ProdLoad, IMPORTS_DIR}`, `profile::{ProfileFile, Computed, compute}`.
- Produces: команды `prod_load_export_template`, `prod_load_preview_import`, `prod_load_apply_import`, `prod_load_get`, `profile_get`, `profile_save`, `profile_compute`.

- [ ] **Step 1: `load/template.rs`**

```rust
//! Шаблон ПРОМ: строки из реестра, чтение заполненного файла, предпросмотр (спека L1 §7).

use std::collections::BTreeMap;
use std::path::Path;

use glafira_load_model::prod_load::{ImportPreview, preview_import};
use glafira_load_model::registry::{OpKind, Status};
use glafira_load_xlsx::{TemplateError, TemplateLabels, TemplateRowOut, read_template, write_template};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::scan::run::now_rfc3339;

fn map_err(e: TemplateError, expected: &str) -> AppError {
    match e {
        TemplateError::NoMarker | TemplateError::BadFormat(_) => app_err!(ErrorCode::TemplateMismatch, "errors.template.notTemplate", "The file is not a Glafira production-load template for project {{expected}}", expected = expected),
        TemplateError::Format(m) => app_err!(ErrorCode::InvalidInput, "errors.template.unreadable", "The file cannot be read as .xlsx").with_details(m),
        TemplateError::Io(m) => AppError::internal_with(m),
    }
}

/// Выгрузка: активные операции в порядке реестра; `prefill` — с текущими цифрами.
pub fn export(d: &ProjectDir, out: &Path, labels: &TemplateLabels, prefill: bool) -> AppResult<usize> {
    let name = d.manifest()?.name;
    let registry = d.registry()?;
    let prod = if prefill { d.prod_load()? } else { Default::default() };
    let rows: Vec<TemplateRowOut> = registry
        .operations
        .iter()
        .filter(|o| o.status == Status::Active)
        .map(|o| TemplateRowOut { id: o.draft.id.clone(), service: o.draft.service.clone(), kind: o.draft.kind.as_str().into(), name: o.draft.name.clone(), summary: o.draft.summary.clone(), figures: prod.operations.get(&o.draft.id).cloned() })
        .collect();
    write_template(out, &name, &rows, labels).map_err(|e| map_err(e, &name))?;
    Ok(rows.len())
}

/// Предпросмотр: маркер, проект, строки против реестра и текущих цифр.
pub fn preview(d: &ProjectDir, file: &Path) -> AppResult<ImportPreview> {
    let name = d.manifest()?.name;
    let bytes = std::fs::read(file).map_err(|e| AppError::from(e).with_details(file.display().to_string()))?;
    let read = read_template(&bytes).map_err(|e| map_err(e, &name))?;
    if read.project != name {
        return Err(app_err!(ErrorCode::TemplateMismatch, "errors.template.otherProject", "The template belongs to project {{found}}, expected {{expected}}", found = read.project, expected = name));
    }
    let registry = d.registry()?;
    let known: BTreeMap<String, OpKind> = registry.operations.iter().filter(|o| o.status == Status::Active).map(|o| (o.draft.id.clone(), o.draft.kind)).collect();
    let current = d.prod_load()?;
    let mut source = read.source;
    source.imported_at = now_rfc3339();
    Ok(preview_import(&read.rows, source, &known, Some(&current)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest::create;
    use glafira_load_model::project::{Manifest, Service};
    use glafira_load_model::registry::{Address, Manual, OpKind, Operation, OperationDraft, Registry, Seen, Source, Status};

    pub(crate) fn labels() -> TemplateLabels {
        TemplateLabels {
            sheet_operations: "Operations".into(),
            sheet_params: "Parameters".into(),
            sheet_help: "Help".into(),
            columns: (0..12).map(|i| format!("c{i}")).collect(),
            notes: vec![String::new(); 12],
            params: (0..5).map(|i| format!("p{i}")).collect(),
            help_lines: vec![],
        }
    }

    pub(crate) fn project_with_ops() -> (tempfile::TempDir, ProjectDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").to_string_lossy().into_owned();
        create(&path, "orders-platform").unwrap();
        let d = ProjectDir::open(&path).unwrap();
        let mut m = Manifest::new("orders-platform");
        m.services.push(Service { id: "orders".into(), name: String::new(), path: ".".into(), contracts: vec![], kafka: Default::default(), extra: Default::default() });
        d.save_manifest(&m).unwrap();
        let mut r = Registry::default();
        for (kind, addr) in [(OpKind::Http, Address::Http { method: "POST".into(), path: "/v1/orders".into() }), (OpKind::KafkaConsume, Address::Kafka { topic: "orders.created".into() })] {
            let draft = OperationDraft::new("orders", kind, addr, Source::manual());
            r.operations.push(Operation { fingerprint: draft.fingerprint(), draft, status: Status::Active, first_seen: Seen::default(), last_seen: Seen::default(), manual: Manual::default() });
        }
        d.save_registry(&r).unwrap();
        (dir, d)
    }

    #[test]
    fn export_then_preview_without_figures_is_all_empty() {
        let (tmp, d) = project_with_ops();
        let file = tmp.path().join("t.xlsx");
        assert_eq!(export(&d, &file, &labels(), false).unwrap(), 2);
        let p = preview(&d, &file).unwrap();
        assert_eq!(p.empty.len(), 2);
        assert!(p.result.is_some());
        let (_tmp2, other) = project_with_ops();
        let mut m = other.manifest().unwrap();
        m.name = "another".into();
        other.save_manifest(&m).unwrap();
        assert_eq!(preview(&other, &file).unwrap_err().code, ErrorCode::TemplateMismatch);
        std::fs::write(tmp.path().join("x.xlsx"), b"junk").unwrap();
        assert_eq!(preview(&d, &tmp.path().join("x.xlsx")).unwrap_err().code, ErrorCode::InvalidInput);
    }
}
```

`load/mod.rs`: `pub mod template;`.

- [ ] **Step 2: `commands/load.rs`**

```rust
use std::path::PathBuf;

use glafira_load_model::prod_load::{IMPORTS_DIR, ImportPreview, ProdLoad};
use glafira_load_model::profile::{Computed, ProfileFile, compute};
use glafira_load_xlsx::TemplateLabels;

use super::blocking;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::load::template::{export, preview};
use crate::project::manifest::ProjectDir;
use crate::scan::run::now_label;

#[tauri::command]
pub async fn prod_load_export_template(path: String, file: String, labels: TemplateLabels, prefill: bool) -> AppResult<usize> {
    blocking(move || export(&ProjectDir::open(&path)?, &PathBuf::from(file), &labels, prefill)).await
}

#[tauri::command]
pub async fn prod_load_preview_import(path: String, file: String) -> AppResult<ImportPreview> {
    blocking(move || preview(&ProjectDir::open(&path)?, &PathBuf::from(file))).await
}

/// Применяет импорт: повторный предпросмотр (файл могли поменять), копия в `load/imports/`, запись `prod-load.yaml`.
#[tauri::command]
pub async fn prod_load_apply_import(path: String, file: String) -> AppResult<ProdLoad> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        let src = PathBuf::from(&file);
        let p = preview(&d, &src)?;
        let Some(mut result) = p.result else {
            return Err(app_err!(ErrorCode::TemplateInvalid, "errors.template.invalidRows", "The template has {{count}} invalid cell(s)", count = p.invalid.len()));
        };
        let copy = format!("{IMPORTS_DIR}/{}.xlsx", now_label());
        let bytes = std::fs::read(&src).map_err(|e| AppError::from(e).with_details(src.display().to_string()))?;
        d.write_bytes(&copy, &bytes)?;
        result.source.file = copy;
        d.save_prod_load(&result)?;
        Ok(result)
    })
    .await
}

#[tauri::command]
pub async fn prod_load_get(path: String) -> AppResult<ProdLoad> {
    blocking(move || ProjectDir::open(&path)?.prod_load()).await
}

#[tauri::command]
pub async fn profile_get(path: String) -> AppResult<ProfileFile> {
    blocking(move || ProjectDir::open(&path)?.profile()).await
}

#[tauri::command]
pub async fn profile_save(path: String, profile: ProfileFile) -> AppResult<ProfileFile> {
    blocking(move || {
        if profile.profiles.is_empty() {
            return Err(app_err!(ErrorCode::InvalidInput, "errors.profile.noProfiles", "Add at least one profile"));
        }
        let d = ProjectDir::open(&path)?;
        d.save_profile(&profile)?;
        Ok(profile)
    })
    .await
}

#[tauri::command]
pub async fn profile_compute(path: String, profile_name: String) -> AppResult<Computed> {
    blocking(move || {
        let d = ProjectDir::open(&path)?;
        compute(&d.profile()?, &profile_name, &d.registry()?, &d.prod_load()?).map_err(|e| app_err!(ErrorCode::NotFound, "errors.profile.notFound", "Profile {{name}} not found", name = e.to_string()))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::template::tests::{labels, project_with_ops};
    use glafira_load_model::prod_load::Figures;

    #[tokio::test]
    async fn apply_import_copies_the_file_and_computes_a_profile() {
        let (tmp, d) = project_with_ops();
        let path = d.root().to_string_lossy().into_owned();
        let mut prod = ProdLoad::default();
        prod.operations.insert("http:orders:POST /v1/orders".into(), Figures { avg: 100.0, peak: 300.0, ..Default::default() });
        prod.operations.insert("kafka_consume:orders.created".into(), Figures { avg: 10.0, peak: 20.0, lag_max_s: Some(5.0), ..Default::default() });
        d.save_prod_load(&prod).unwrap();
        let file = tmp.path().join("filled.xlsx");
        assert_eq!(prod_load_export_template(path.clone(), file.to_string_lossy().into_owned(), labels(), true).await.unwrap(), 2);
        d.save_prod_load(&ProdLoad::default()).unwrap();
        let preview = prod_load_preview_import(path.clone(), file.to_string_lossy().into_owned()).await.unwrap();
        assert_eq!(preview.filled.len(), 2);
        let applied = prod_load_apply_import(path.clone(), file.to_string_lossy().into_owned()).await.unwrap();
        assert!(applied.source.file.starts_with("load/imports/") && d.file(&applied.source.file).is_file());
        assert_eq!(prod_load_get(path.clone()).await.unwrap().operations["kafka_consume:orders.created"].lag_max_s, Some(5.0));
        let c = profile_compute(path.clone(), "peak".into()).await.unwrap();
        assert_eq!((c.targets.len(), c.total), (2, 320.0));
        assert_eq!(profile_compute(path.clone(), "nope".into()).await.unwrap_err().code, ErrorCode::NotFound);
        let mut f = profile_get(path.clone()).await.unwrap();
        f.coefficients.k_env = 0.5;
        profile_save(path.clone(), f).await.unwrap();
        assert_eq!(profile_compute(path, "peak".into()).await.unwrap().total, 160.0);
    }
}
```

В `commands/mod.rs`: `pub mod load;`. В `lib.rs`: `pub mod load;` и в `generate_handler!`: `commands::load::prod_load_export_template, commands::load::prod_load_preview_import, commands::load::prod_load_apply_import, commands::load::prod_load_get, commands::load::profile_get, commands::load::profile_save, commands::load::profile_compute`.

Словари (`errors.template`): `notTemplate` «The file is not a Glafira production-load template for project {{expected}}» / «Файл не является шаблоном нагрузки ПРОМ Glafira для проекта {{expected}}»; `unreadable` «The file cannot be read as .xlsx» / «Файл не читается как .xlsx»; `otherProject` «The template belongs to project {{found}}, expected {{expected}}» / «Шаблон от проекта {{found}}, ожидался {{expected}}»; `invalidRows` «The template has {{count}} invalid cell(s)» / «В шаблоне ошибочных ячеек: {{count}}». `errors.profile`: `noProfiles` «Add at least one profile» / «Добавьте хотя бы один профиль»; `notFound` «Profile {{name}} not found» / «Профиль {{name}} не найден».

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm i18n:check`
Expected: все тесты PASS, включая `load::template` и `commands::load`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(load): команды выгрузки шаблона ПРОМ, предпросмотра и применения импорта, профиля

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 15: Фронтенд — типы, клиент IPC, дерево проектов и вкладка «Проект»

**Files:**
- Modify: `src/api/types.ts`, `src/api/client.ts`, `src/components/Sidebar.tsx`, `src/components/TabContent.tsx`, `src/locales/{en,ru}.json`, `src/styles.css`
- Create: `src/lib/paths.ts`, `src/components/projects/ProjectTree.tsx`, `src/components/project/ProjectPanel.tsx`
- Test: `src/lib/paths.test.ts`, `src/components/projects/ProjectTree.test.tsx`, `src/components/project/ProjectPanel.test.tsx`

**Interfaces:**
- Consumes: команды Tasks 12–14 (имена и формы ответов как в Rust-структурах: модель — snake_case, DTO — camelCase).
- Produces: `api.*` методы для всех команд L1; `joinProjectPath(project, rel)`; `ProjectTree`, `ProjectPanel`.

- [ ] **Step 1: `src/api/types.ts`** — добавить после общих типов:

```ts
// ---- Модель проекта: snake_case, как в файлах проекта (решение плана 9) ----

export type ContractKind = "openapi" | "proto" | "asyncapi" | "schema" | "sql" | "spring";
export interface ContractRef { kind: ContractKind; path: string; include?: string[]; enabled?: boolean }
export type Direction = "produce" | "consume";
export type TopicSchema = string | { schema: string; direction?: Direction };
export interface Service {
  id: string;
  name: string;
  path: string;
  contracts: ContractRef[];
  kafka?: { topics: Record<string, TopicSchema> };
}
export type EnvKind = "dev" | "test" | "stage" | "prod";
export interface Environment { id: string; kind: EnvKind; http?: string; kafka?: string; grpc?: string; sql?: string }
export interface Manifest { format: number; name: string; services: Service[]; environments: Environment[] }
export type ManifestProblem =
  | { kind: "bad_format"; format: number }
  | { kind: "empty_name" }
  | { kind: "bad_service_id" | "duplicate_service_id" | "empty_service_path" | "bad_environment_id" | "duplicate_environment_id"; id: string };

export interface SchemaSummary {
  name: string;
  type: string;
  required?: boolean;
  enum?: string[];
  example?: string;
  children?: SchemaSummary[];
  truncated?: boolean;
}
export type OpKind = "http" | "kafka_produce" | "kafka_consume" | "grpc" | "sql";
export type Address =
  | { type: "http"; method: string; path: string }
  | { type: "kafka"; topic: string }
  | { type: "grpc"; service: string; method: string }
  | { type: "sql"; name: string };
export type SourceKind = "openapi" | "proto" | "asyncapi" | "schema" | "sql" | "spring" | "manual";
export interface Source { kind: SourceKind; path: string; line?: number }
export type Status = "active" | "removed";
export type Streaming = "client" | "server" | "bidi";
export interface Seen { version: string; commit: string; at: string }
export interface Manual { tags?: string[]; notes?: string; owner?: string }
export interface OperationDraft {
  id: string;
  service: string;
  kind: OpKind;
  name: string;
  summary?: string;
  address: Address;
  request?: SchemaSummary;
  response?: SchemaSummary;
  responses?: Record<string, SchemaSummary>;
  source: Source;
  streaming?: Streaming;
  topic_guessed?: boolean;
}
export interface Operation extends OperationDraft {
  fingerprint: string;
  status: Status;
  first_seen: Seen;
  last_seen: Seen;
  manual?: Manual;
}
export type FieldChange =
  | { change: "added" | "removed"; path: string; kind: string }
  | { change: "type_changed"; path: string; from: string; to: string }
  | { change: "required_changed"; path: string; required: boolean }
  | { change: "address_changed"; from: string; to: string };
export interface ChangedOperation { id: string; changes: FieldChange[] }
export interface MergeSummary { added: string[]; changed: ChangedOperation[]; removed: string[]; restored: string[]; unchanged: number }
export interface RegistryDiff { added: string[]; removed: string[]; changed: string[] }

export interface Figures {
  avg: number;
  peak: number;
  error_pct?: number;
  req_bytes?: number;
  resp_bytes?: number;
  p95_ms?: number;
  lag_max_s?: number;
  comment?: string;
}
export interface SourceInfo { stand: string; period_from: string; period_to: string; system: string; filled_by: string; imported_at: string; file: string }
export interface ProdLoad { format: number; source: SourceInfo; operations: Record<string, Figures> }
export type InvalidReason = "not_a_number" | "negative" | "not_an_integer" | "percent_out_of_range" | "peak_below_avg" | "avg_missing";
export interface InvalidCell { row: number; id: string; column: string; value: string; reason: InvalidReason }
export interface FiguresDiff { id: string; old: Figures | null; new: Figures }
export interface ImportPreview {
  filled: string[];
  empty: string[];
  unknown_ids: string[];
  invalid: InvalidCell[];
  diff: FiguresDiff[];
  source: SourceInfo;
  result: ProdLoad | null;
}

export interface Override { k_env?: number; k_growth?: number }
export interface Coefficients { k_env: number; k_growth: number; overrides: { services: Record<string, Override>; operations: Record<string, Override> } }
export type Basis = "avg" | "peak";
export type Phase =
  | { kind: "ramp"; minutes: number; to_pct: number }
  | { kind: "steady"; minutes: number }
  | { kind: "steps"; step_pct: number; step_minutes: number; max_pct: number };
export interface Profile { basis: Basis; phases: Phase[] }
export interface ProfileFile { format: number; coefficients: Coefficients; profiles: Record<string, Profile> }
export interface Target { id: string; service: string; kind: OpKind; basis_value: number; k_env: number; k_growth: number; target: number; share: number }
export interface Point { minute: number; pct: number }
export interface Computed { profile: string; basis: Basis; targets: Target[]; total: number; uncovered: string[]; timeline: Point[]; total_minutes: number }

// ---- DTO приложения: camelCase ----

export interface Project { path: string; manifest: Manifest; problems: ManifestProblem[]; readOnly: boolean; missingServices: string[] }
export interface RecentProject { path: string; name: string | null; missing: boolean }
export interface GitVersion { version: string; commit: string; available: boolean; error: string | null }
export interface Diagnostic { path: string; line: number | null; message: string }
export interface ScanProgress { service: string; done: number; total: number; file: string }
export interface ScanReport {
  service: string;
  git: GitVersion;
  contracts: number;
  operationsFound: number;
  tablesFound: number;
  summary: MergeSummary;
  warnings: Diagnostic[];
  errors: Diagnostic[];
  scanFile: string;
  cancelled: boolean;
}
export interface OperationRow { id: string; service: string; kind: OpKind; name: string; summary: string; source: SourceKind; status: Status; lastVersion: string; topicGuessed: boolean }
export interface RegistryFilter { service?: string; kind?: OpKind; source?: SourceKind; status?: Status; query?: string }
export interface ScanMeta { file: string; at: string; service: string; version: string; added: number; changed: number; removed: number; restored: number }
export interface TableColumn { name: string; dataType: string; nullable: boolean }
export interface Table { name: string; columns: TableColumn[] }
export interface TemplateLabels {
  sheetOperations: string;
  sheetParams: string;
  sheetHelp: string;
  columns: string[];
  notes: string[];
  params: string[];
  helpLines: string[];
}
```

- [ ] **Step 2: `src/api/client.ts`** — добавить в объект `api`:

```ts
  projectCreate: (path: string, name: string) => call<Project>("project_create", { path, name }),
  projectOpen: (path: string) => call<Project>("project_open", { path }),
  projectClose: (path: string) => call<void>("project_close", { path }),
  projectSave: (path: string, manifest: Manifest) => call<Project>("project_save", { path, manifest }),
  projectRecent: () => call<RecentProject[]>("project_recent"),
  openPath: (project: string, path: string) => call<void>("open_path", { project, path }),
  serviceDiscoverContracts: (path: string, serviceId: string) => call<ContractRef[]>("service_discover_contracts", { path, serviceId }),
  /** Разбор сервиса (или всех при `serviceId = null`); прогресс приходит в `onProgress`. */
  serviceScan: (path: string, serviceId: string | null, onProgress: (p: ScanProgress) => void) => {
    const channel = new Channel<ScanProgress>();
    channel.onmessage = onProgress;
    return call<ScanReport[]>("service_scan", { path, serviceId, onProgress: channel });
  },
  scanCancel: (path: string, serviceId: string | null) => call<void>("scan_cancel", { path, serviceId }),
  registryList: (path: string, filter: RegistryFilter) => call<OperationRow[]>("registry_list", { path, filter }),
  registryGet: (path: string, id: string) => call<Operation>("registry_get", { path, id }),
  registryUpdateManual: (path: string, id: string, manual: Manual) => call<Operation>("registry_update_manual", { path, id, manual }),
  registryAddManual: (path: string, draft: OperationDraft) => call<Operation>("registry_add_manual", { path, draft }),
  registryDelete: (path: string, id: string) => call<void>("registry_delete", { path, id }),
  registryScans: (path: string) => call<ScanMeta[]>("registry_scans", { path }),
  registryDiff: (path: string, from: string, to: string) => call<RegistryDiff>("registry_diff", { path, from, to }),
  registryTables: (path: string) => call<Record<string, Table[]>>("registry_tables", { path }),
  prodLoadExportTemplate: (path: string, file: string, labels: TemplateLabels, prefill: boolean) => call<number>("prod_load_export_template", { path, file, labels, prefill }),
  prodLoadPreviewImport: (path: string, file: string) => call<ImportPreview>("prod_load_preview_import", { path, file }),
  prodLoadApplyImport: (path: string, file: string) => call<ProdLoad>("prod_load_apply_import", { path, file }),
  prodLoadGet: (path: string) => call<ProdLoad>("prod_load_get", { path }),
  profileGet: (path: string) => call<ProfileFile>("profile_get", { path }),
  profileSave: (path: string, profile: ProfileFile) => call<ProfileFile>("profile_save", { path, profile }),
  profileCompute: (path: string, profileName: string) => call<Computed>("profile_compute", { path, profileName }),
```

(импортировать `Channel` из `@tauri-apps/api/core` и новые типы из `./types`).

- [ ] **Step 3: `src/lib/paths.ts`** и тест

```ts
/** Абсолютный путь клона: `rel` относительно папки проекта или как есть, если абсолютный (POSIX или Windows). */
export function joinProjectPath(project: string, rel: string): string {
  if (/^(?:[a-zA-Z]:[\\/]|\\\\|\/)/.test(rel)) return rel;
  const sep = project.includes("\\") && !project.includes("/") ? "\\" : "/";
  return `${project.replace(/[\\/]+$/, "")}${sep}${rel.replace(/^[\\/]+/, "")}`;
}

/** Имя папки из пути. */
export function baseName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
}
```

`src/lib/paths.test.ts`:

```ts
import { baseName, joinProjectPath } from "./paths";

it("joins relative paths and keeps absolute ones", () => {
  expect(joinProjectPath("/home/u/p", "../orders")).toBe("/home/u/p/../orders");
  expect(joinProjectPath("/home/u/p/", "/abs/x")).toBe("/abs/x");
  expect(joinProjectPath("C:\\work\\p", "svc")).toBe("C:\\work\\p\\svc");
  expect(joinProjectPath("C:\\work\\p", "D:\\other")).toBe("D:\\other");
  expect(baseName("/a/b/c/")).toBe("c");
});
```

В `Layout.tsx` заменить локальную `projectLabel` на `baseName` из `lib/paths` (экспорт `projectLabel` удалить, тест `Layout.test.tsx` поправить на `baseName`).

- [ ] **Step 4: Тест дерева** — `src/components/projects/ProjectTree.test.tsx`:

```tsx
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { useWorkspace } from "../../state/workspace";
import { ProjectTree } from "./ProjectTree";

const api = {
  projectRecent: vi.fn(async () => [
    { path: "/p/orders", name: "Orders platform", missing: false },
    { path: "/p/gone", name: null, missing: true },
  ]),
  projectOpen: vi.fn(async (path: string) => ({ path, manifest: { format: 1, name: "Orders platform", services: [{ id: "orders", name: "Orders", path: "../orders", contracts: [] }], environments: [] }, problems: [], readOnly: false, missingServices: [] })),
  projectClose: vi.fn(async () => undefined),
};
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => "/p/new") }));

describe("ProjectTree", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("lists recent projects, expands sections and opens tabs", async () => {
    renderWithClient(<ProjectTree />);
    const orders = await screen.findByRole("treeitem", { name: /Orders platform/ });
    await userEvent.click(within(orders).getByRole("button", { name: /Orders platform/ }));
    expect(useWorkspace.getState().tabs.map((t) => t.id)).toEqual(["project:/p/orders"]);
    await userEvent.click(await screen.findByRole("button", { name: "Реестр операций" }));
    expect(useWorkspace.getState().activeId).toBe("registry:/p/orders");
    await userEvent.click(screen.getByRole("button", { name: "Orders" }));
    expect(useWorkspace.getState().activeId).toBe("service:/p/orders:orders");
    expect(screen.getByText("Папка не найдена")).toBeInTheDocument();
  });

  it("removes a missing project from the list and closes its tabs", async () => {
    renderWithClient(<ProjectTree />);
    useWorkspace.getState().open({ kind: "registry", path: "/p/gone" });
    const gone = await screen.findByRole("treeitem", { name: /gone/ });
    await userEvent.click(within(gone).getByRole("button", { name: "Убрать из списка" }));
    expect(api.projectClose).toHaveBeenCalledWith("/p/gone");
    expect(useWorkspace.getState().tabs).toEqual([]);
  });

  it("opens a project folder through the dialog", async () => {
    renderWithClient(<ProjectTree />);
    await userEvent.click(await screen.findByRole("button", { name: "Открыть проект…" }));
    expect(api.projectOpen).toHaveBeenCalledWith("/p/new");
    expect(useWorkspace.getState().activeId).toBe("project:/p/new");
  });
});
```

- [ ] **Step 5: `ProjectTree.tsx`**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { RecentProject } from "../../api/types";
import { baseName } from "../../lib/paths";
import { type TabSpec, useWorkspace } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";

const COLLAPSED_KEY = "glafira-load.tree.collapsed";

function readCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function writeCollapsed(ids: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...ids]));
  } catch {
    // Хранилище недоступно — состояние живёт только до перезапуска.
  }
}

/** Узел проекта: имя, разделы и сервисы (сервисы берутся из манифеста при раскрытии). */
function ProjectNode({ project, collapsed, onToggle }: { project: RecentProject; collapsed: boolean; onToggle: () => void }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const open = useWorkspace((s) => s.open);
  const closeForProject = useWorkspace((s) => s.closeForProject);
  const manifest = useQuery({ queryKey: ["project", project.path], queryFn: () => api.projectOpen(project.path), enabled: !collapsed && !project.missing });
  const remove = useMutation({
    mutationFn: () => api.projectClose(project.path),
    onSuccess: () => {
      closeForProject(project.path);
      void qc.invalidateQueries({ queryKey: ["projects"] });
    },
  });
  const label = project.name ?? baseName(project.path);
  const section = (spec: TabSpec, text: string) => (
    <li role="treeitem" key={text}>
      <button type="button" className="link tree-name" onClick={() => open(spec)}>
        {text}
      </button>
    </li>
  );
  return (
    <li role="treeitem" aria-expanded={!collapsed} aria-label={label} className={project.missing ? "tree-folder tree-error" : "tree-folder"}>
      <div className="tree-conn">
        <button type="button" className="tree-caret" aria-label={t("sidebar.folderToggle", { name: label })} onClick={onToggle} disabled={project.missing}>
          {collapsed ? "▸" : "▾"}
        </button>
        <button type="button" className="link tree-name" title={project.path} disabled={project.missing} onClick={() => open({ kind: "project", path: project.path })}>
          {label}
        </button>
        {project.missing && <span className="muted">{t("sidebar.missing")}</span>}
        <button type="button" className="icon" aria-label={t("sidebar.remove")} onClick={() => remove.mutate()}>
          ×
        </button>
      </div>
      {!collapsed && !project.missing && (
        <ul role="group" className="tree">
          {section({ kind: "project", path: project.path }, t("sidebar.project"))}
          <li role="treeitem" aria-label={t("sidebar.services")}>
            <span className="muted">{t("sidebar.services")}</span>
            <ul role="group" className="tree">
              {(manifest.data?.manifest.services ?? []).map((s) => (
                <li role="treeitem" key={s.id}>
                  <button type="button" className="link tree-name" onClick={() => open({ kind: "service", path: project.path, serviceId: s.id })}>
                    {s.name || s.id}
                  </button>
                </li>
              ))}
            </ul>
          </li>
          {section({ kind: "registry", path: project.path }, t("sidebar.registry"))}
          {section({ kind: "prod-load", path: project.path }, t("sidebar.prodLoad"))}
          {section({ kind: "profile", path: project.path }, t("sidebar.profile"))}
        </ul>
      )}
      {manifest.error && <ErrorBanner error={manifest.error} />}
    </li>
  );
}

export function ProjectTree() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const open = useWorkspace((s) => s.open);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const projects = useQuery({ queryKey: ["projects"], queryFn: api.projectRecent });
  const openProject = useMutation({
    mutationFn: async () => {
      const path = await openDialog({ directory: true, multiple: false, title: t("sidebar.openProject") });
      if (!path) return null;
      return api.projectOpen(path);
    },
    onSuccess: (p) => {
      if (!p) return;
      void qc.invalidateQueries({ queryKey: ["projects"] });
      open({ kind: "project", path: p.path });
    },
  });
  const toggle = (path: string) => {
    const next = new Set(collapsed);
    if (!next.delete(path)) next.add(path);
    writeCollapsed(next);
    setCollapsed(next);
  };

  return (
    <div className="tree" data-testid="project-tree">
      <div className="sidebar-header">
        <strong>{t("sidebar.projects")}</strong>
        <div className="sidebar-actions">
          <button type="button" className="link" onClick={() => openProject.mutate()}>
            {t("sidebar.openProject")}
          </button>
          <button type="button" className="link" onClick={() => open({ kind: "project", path: null })}>
            {t("sidebar.newProject")}
          </button>
        </div>
      </div>
      {openProject.error && <ErrorBanner error={openProject.error} />}
      {projects.error && <ErrorBanner error={projects.error} onRetry={() => projects.refetch()} />}
      {projects.data?.length === 0 && <p className="empty">{t("sidebar.emptyHint")}</p>}
      <ul role="tree" className="tree">
        {(projects.data ?? []).map((p) => (
          <ProjectNode key={p.path} project={p} collapsed={collapsed.has(p.path)} onToggle={() => toggle(p.path)} />
        ))}
      </ul>
    </div>
  );
}
```

В `Sidebar.tsx` заменить заглушку `<div className="tree" …>` на `<ProjectTree />`. Ключи: `sidebar.project` «Project» / «Проект», `sidebar.folderToggle` «Expand or collapse {{name}}» / «Развернуть или свернуть {{name}}».

- [ ] **Step 6: Тест вкладки «Проект»** — `src/components/project/ProjectPanel.test.tsx`:

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { useWorkspace } from "../../state/workspace";
import { ProjectPanel } from "./ProjectPanel";

const project = { path: "/p/orders", manifest: { format: 1, name: "Orders", services: [{ id: "orders", name: "Orders", path: "../orders", contracts: [] }], environments: [{ id: "test", kind: "test", http: "https://t" }] }, problems: [], readOnly: false, missingServices: ["orders"] };
const api = {
  projectOpen: vi.fn(async () => project),
  projectSave: vi.fn(async (_p: string, manifest: unknown) => ({ ...project, manifest })),
  projectCreate: vi.fn(async (path: string, name: string) => ({ ...project, path, manifest: { ...project.manifest, name, services: [] } })),
  openPath: vi.fn(async () => undefined),
  serviceScan: vi.fn(async () => []),
};
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => "/p/created") }));

describe("ProjectPanel", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("edits the manifest and saves it", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    expect(await screen.findByDisplayValue("Orders")).toBeInTheDocument();
    expect(screen.getByText(/Папка сервиса не найдена/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Добавить стенд" }));
    const ids = screen.getAllByLabelText("ID стенда");
    await userEvent.type(ids[ids.length - 1], "stage");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.projectSave).toHaveBeenCalledWith("/p/orders", expect.objectContaining({ environments: [expect.objectContaining({ id: "test" }), expect.objectContaining({ id: "stage", kind: "dev" })] }));
  });

  it("creates a new project and switches the tab", async () => {
    useWorkspace.getState().open({ kind: "project", path: null });
    renderWithClient(<ProjectPanel path={null} />);
    await userEvent.type(screen.getByLabelText("Имя проекта"), "New one");
    await userEvent.click(screen.getByRole("button", { name: "Выбрать папку…" }));
    await userEvent.click(await screen.findByRole("button", { name: "Создать проект" }));
    expect(api.projectCreate).toHaveBeenCalledWith("/p/created", "New one");
    expect(useWorkspace.getState().tabs.map((t) => t.id)).toEqual(["project:/p/created"]);
  });
});
```

- [ ] **Step 7: `ProjectPanel.tsx`**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { EnvKind, Environment, Manifest, ManifestProblem, Project, ScanProgress, ScanReport, Service } from "../../api/types";
import { useHelpChapter } from "../../state/helpContext";
import { tabId, useWorkspace } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";

const ENV_KINDS: EnvKind[] = ["dev", "test", "stage", "prod"];

function problemText(t: (k: string, o?: Record<string, unknown>) => string, p: ManifestProblem): string {
  return "id" in p ? t(`project.problems.${p.kind}`, { id: p.id }) : "format" in p ? t("project.problems.bad_format", { format: p.format }) : t("project.problems.empty_name");
}

/** Форма нового проекта: имя и пустая папка. */
function NewProject() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const { open, close } = useWorkspace();
  const [name, setName] = useState("");
  const [folder, setFolder] = useState<string | null>(null);
  const choose = async () => setFolder((await openDialog({ directory: true, multiple: false, title: t("project.chooseFolder") })) ?? null);
  const create = useMutation({
    mutationFn: () => api.projectCreate(folder ?? "", name.trim()),
    onSuccess: (p) => {
      void qc.invalidateQueries({ queryKey: ["projects"] });
      close(tabId({ kind: "project", path: null }));
      open({ kind: "project", path: p.path });
    },
  });
  return (
    <div className="page">
      <div className="toolbar">
        <h2>{t("project.newTitle")}</h2>
      </div>
      <div className="form">
        <div className="field">
          <label htmlFor="project-name">{t("project.name")}</label>
          <input id="project-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="field">
          <span className="field-label">{t("project.folder")}</span>
          <div className="row">
            <code>{folder ?? t("project.noFolder")}</code>
            <button type="button" onClick={choose}>
              {t("project.chooseFolder")}
            </button>
          </div>
          <Hint k="hints.project.folder" label={t("project.folder")} />
        </div>
        {create.error && <ErrorBanner error={create.error} />}
        <button type="button" disabled={!folder || !name.trim() || create.isPending} onClick={() => create.mutate()}>
          {t("project.create")}
        </button>
      </div>
    </div>
  );
}

function ServiceRow({ s, project, onChange, onRemove, missing }: { s: Service; project: string; onChange: (s: Service) => void; onRemove: () => void; missing: boolean }) {
  const { t } = useTranslation();
  const choose = async () => {
    const dir = await openDialog({ directory: true, multiple: false, defaultPath: project });
    if (dir) onChange({ ...s, path: dir });
  };
  return (
    <fieldset className="panel">
      <div className="row">
        <label>
          {t("project.serviceId")}
          <input aria-label={t("project.serviceId")} value={s.id} pattern="[a-z0-9-]{1,64}" onChange={(e) => onChange({ ...s, id: e.target.value })} />
        </label>
        <label>
          {t("project.serviceName")}
          <input value={s.name} onChange={(e) => onChange({ ...s, name: e.target.value })} />
        </label>
        <button type="button" className="icon" aria-label={t("common.delete")} onClick={onRemove}>
          ×
        </button>
      </div>
      <div className="row">
        <code className="path-field">{s.path || t("project.noFolder")}</code>
        <button type="button" onClick={choose}>
          {t("project.chooseFolder")}
        </button>
        {missing && <span className="warn">{t("project.serviceMissing")}</span>}
      </div>
    </fieldset>
  );
}

function EnvRow({ e, onChange, onRemove }: { e: Environment; onChange: (e: Environment) => void; onRemove: () => void }) {
  const { t } = useTranslation();
  const field = (key: "http" | "kafka" | "grpc" | "sql") => (
    <label key={key}>
      {t(`project.env.${key}`)}
      <input value={e[key] ?? ""} onChange={(ev) => onChange({ ...e, [key]: ev.target.value || undefined })} />
    </label>
  );
  return (
    <fieldset className="panel">
      <div className="row">
        <label>
          {t("project.envId")}
          <input aria-label={t("project.envId")} value={e.id} onChange={(ev) => onChange({ ...e, id: ev.target.value })} />
        </label>
        <label>
          {t("project.envKind")}
          <select value={e.kind} onChange={(ev) => onChange({ ...e, kind: ev.target.value as EnvKind })}>
            {ENV_KINDS.map((k) => (
              <option key={k} value={k}>
                {k.toUpperCase()}
              </option>
            ))}
          </select>
        </label>
        <button type="button" className="icon" aria-label={t("common.delete")} onClick={onRemove}>
          ×
        </button>
      </div>
      <div className="row">{(["http", "kafka", "grpc", "sql"] as const).map(field)}</div>
    </fieldset>
  );
}

function ExistingProject({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["project", path], queryFn: () => api.projectOpen(path) });
  const [draft, setDraft] = useState<Manifest | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [reports, setReports] = useState<ScanReport[]>([]);
  useEffect(() => {
    if (q.data) setDraft(structuredClone(q.data.manifest));
  }, [q.data]);
  const save = useMutation({
    mutationFn: (m: Manifest) => api.projectSave(path, m),
    onSuccess: (p: Project) => {
      qc.setQueryData(["project", path], p);
      void qc.invalidateQueries({ queryKey: ["projects"] });
    },
  });
  const scanAll = useMutation({
    mutationFn: () => api.serviceScan(path, null, setProgress),
    onSuccess: (r) => {
      setReports(r);
      setProgress(null);
      void qc.invalidateQueries({ queryKey: ["registry", path] });
    },
    onError: () => setProgress(null),
  });

  if (q.error) return <ErrorBanner error={q.error} onRetry={() => q.refetch()} />;
  const p = q.data;
  if (!p || !draft) return <p className="empty">{t("common.loading")}</p>;
  const update = (patch: Partial<Manifest>) => setDraft({ ...draft, ...patch });
  const dirty = JSON.stringify(draft) !== JSON.stringify(p.manifest);

  return (
    <div className="page">
      <div className="toolbar">
        <h2>{p.manifest.name}</h2>
        <button type="button" onClick={() => api.openPath(path, path)}>
          {t("project.openFolder")}
        </button>
        <button type="button" disabled={scanAll.isPending || p.readOnly} onClick={() => scanAll.mutate()}>
          {t("project.scanAll")}
        </button>
        {scanAll.isPending && (
          <button type="button" onClick={() => api.scanCancel(path, null)}>
            {t("common.cancel")}
          </button>
        )}
      </div>
      {p.readOnly && (
        <div className="notice" role="alert">
          <strong>{t("project.readOnly")}</strong>
          <ul>
            {p.problems.map((pr, i) => (
              <li key={i}>{problemText(t, pr)}</li>
            ))}
          </ul>
        </div>
      )}
      {progress && (
        <p className="muted">
          {t("project.scanProgress", { service: progress.service, done: progress.done, total: progress.total, file: progress.file })}
        </p>
      )}
      {scanAll.error && <ErrorBanner error={scanAll.error} />}
      {reports.length > 0 && (
        <ul className="stats">
          {reports.map((r) => (
            <li key={r.service}>
              {t("project.scanResult", { service: r.service, version: r.git.version, found: r.operationsFound, added: r.summary.added.length, changed: r.summary.changed.length, removed: r.summary.removed.length, errors: r.errors.length })}
            </li>
          ))}
        </ul>
      )}
      <fieldset className="form" disabled={p.readOnly}>
        <div className="field">
          <label htmlFor="manifest-name">{t("project.name")}</label>
          <input id="manifest-name" value={draft.name} onChange={(e) => update({ name: e.target.value })} />
          <code className="muted">{path}</code>
        </div>
        <h3>
          {t("project.services")} <Hint k="hints.project.services" label={t("project.services")} />
        </h3>
        {draft.services.map((s, i) => (
          <ServiceRow
            key={i}
            s={s}
            project={path}
            missing={p.missingServices.includes(s.id)}
            onChange={(next) => update({ services: draft.services.map((x, j) => (j === i ? next : x)) })}
            onRemove={() => update({ services: draft.services.filter((_, j) => j !== i) })}
          />
        ))}
        <button type="button" onClick={() => update({ services: [...draft.services, { id: "", name: "", path: "", contracts: [] }] })}>
          {t("project.addService")}
        </button>
        <h3>
          {t("project.environments")} <Hint k="hints.project.environments" label={t("project.environments")} />
        </h3>
        {draft.environments.map((e, i) => (
          <EnvRow key={i} e={e} onChange={(next) => update({ environments: draft.environments.map((x, j) => (j === i ? next : x)) })} onRemove={() => update({ environments: draft.environments.filter((_, j) => j !== i) })} />
        ))}
        <button type="button" onClick={() => update({ environments: [...draft.environments, { id: "", kind: "dev" }] })}>
          {t("project.addEnvironment")}
        </button>
        {save.error && <ErrorBanner error={save.error} />}
        <div className="row">
          <button type="button" disabled={!dirty || save.isPending} onClick={() => save.mutate(draft)}>
            {t("common.save")}
          </button>
          <button type="button" disabled={!dirty} onClick={() => setDraft(structuredClone(p.manifest))}>
            {t("project.revert")}
          </button>
        </div>
      </fieldset>
    </div>
  );
}

export function ProjectPanel({ path }: { path: string | null }) {
  useHelpChapter(tabId({ kind: "project", path }), "project");
  return path ? <ExistingProject path={path} /> : <NewProject />;
}
```

В `TabContent.tsx` добавить `case "project": return <ProjectPanel path={tab.path} />;`. Ключи `en.json` (ветка `project`): `newTitle` «New load project», `name` «Project name», `folder` «Folder», `noFolder` «not chosen», `chooseFolder` «Choose folder…», `create` «Create project», `openFolder` «Open folder», `scanAll` «Scan all services», `scanProgress` «{{service}}: {{done}} of {{total}} {{file}}», `scanResult` «{{service}} ({{version}}): found {{found}}, new {{added}}, changed {{changed}}, removed {{removed}}, errors {{errors}}», `readOnly` «The manifest has problems; fix the file and reopen the project:», `services` «Services», `serviceId` «Service ID», `serviceName` «Name», `serviceMissing` «Service folder not found», `addService` «Add service», `environments` «Environments», `envId` «Environment ID», `envKind` «Kind», `env.http` «HTTP base URL», `env.kafka` «Kafka bootstrap», `env.grpc` «gRPC address», `env.sql` «DB connection (no password)», `addEnvironment` «Add environment», `revert` «Revert», `problems.bad_format` «Unsupported manifest format {{format}}», `problems.empty_name` «Project name is empty», `problems.bad_service_id` «Bad service ID “{{id}}” (a–z, 0–9, hyphen)», `problems.duplicate_service_id` «Duplicate service ID “{{id}}”», `problems.empty_service_path` «Service “{{id}}” has no folder», `problems.bad_environment_id` «Bad environment ID “{{id}}”», `problems.duplicate_environment_id` «Duplicate environment ID “{{id}}”»; `hints.project.folder` «An empty folder inside your git repository: the project is a set of text files that you commit.», `hints.project.services` «A service is a local git clone of the system under test; its contracts are found automatically or listed on the service tab.», `hints.project.environments` «Addresses of your stands without passwords; secrets come with the runner in stage L2.». `ru.json`: «Новый проект НТ», «Имя проекта», «Папка», «не выбрана», «Выбрать папку…», «Создать проект», «Открыть папку», «Разобрать все сервисы», «{{service}}: {{done}} из {{total}} {{file}}», «{{service}} ({{version}}): найдено {{found}}, новых {{added}}, изменённых {{changed}}, удалённых {{removed}}, ошибок {{errors}}», «В манифесте есть проблемы; исправьте файл и откройте проект заново:», «Сервисы», «ID сервиса», «Имя», «Папка сервиса не найдена», «Добавить сервис», «Стенды», «ID стенда», «Вид», «Базовый адрес HTTP», «Bootstrap Kafka», «Адрес gRPC», «Строка БД (без пароля)», «Добавить стенд», «Отменить правки», проблемы — «Неподдерживаемый формат манифеста {{format}}», «Имя проекта пустое», «Плохой ID сервиса «{{id}}» (a–z, 0–9, дефис)», «Повтор ID сервиса «{{id}}»», «У сервиса «{{id}}» нет папки», «Плохой ID стенда «{{id}}»», «Повтор ID стенда «{{id}}»»; подсказки — «Пустая папка внутри вашего git-репозитория: проект — набор текстовых файлов, которые вы коммитите.», «Сервис — локальный клон git тестируемой системы; контракты находятся автоматически или перечисляются на вкладке сервиса.», «Адреса стендов без паролей; секреты появятся вместе с раннером на этапе L2.»

- [ ] **Step 8: Run tests**

Run: `pnpm test && pnpm i18n:check && pnpm build`
Expected: тесты `paths`, `ProjectTree` (3), `ProjectPanel` (2), `Layout` PASS; словари согласованы; сборка проходит. `pnpm tauri dev`: «Новый проект…» создаёт папку с манифестом, проект появляется в дереве, вкладка «Проект» сохраняет сервисы и стенды.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(ui): типы и клиент IPC, дерево проектов, вкладка «Проект» с созданием и правкой манифеста

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 16: Фронтенд — вкладка «Сервис»: контракты, разбор, отчёт

**Files:**
- Create: `src/components/services/ServicePanel.tsx`, `src/components/services/ScanReportView.tsx`
- Modify: `src/components/TabContent.tsx`, `src/locales/{en,ru}.json`
- Test: `src/components/services/ServicePanel.test.tsx`

**Interfaces:**
- Consumes: `api.projectOpen`, `api.projectSave`, `api.serviceDiscoverContracts`, `api.serviceScan`, `api.scanCancel`, `api.openPath`, `joinProjectPath`.
- Produces: `ServicePanel({ path, serviceId })`, `ScanReportView({ report, project, clone })` (переиспользуется в Task 17 для «Разобрать» из реестра не нужен — только здесь).

- [ ] **Step 1: Тест** — `src/components/services/ServicePanel.test.tsx`:

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { ServicePanel } from "./ServicePanel";

const project = { path: "/p/orders", manifest: { format: 1, name: "Orders", services: [{ id: "orders", name: "Orders", path: "../orders-service", contracts: [] }], environments: [] }, problems: [], readOnly: false, missingServices: [] };
const report = {
  service: "orders",
  git: { version: "unversioned", commit: "", available: false, error: "git not started: not found" },
  contracts: 2,
  operationsFound: 3,
  tablesFound: 0,
  summary: { added: ["http:orders:POST /v1/orders"], changed: [], removed: [], restored: [], unchanged: 2 },
  warnings: [{ path: "api/openapi.yaml", line: 4, message: "callbacks are not supported" }],
  errors: [{ path: "proto/x.proto", line: 7, message: "identifier expected" }],
  scanFile: "registry/scans/2026-10-31-120000.yaml",
  cancelled: false,
};
const api = {
  projectOpen: vi.fn(async () => project),
  projectSave: vi.fn(async (_p: string, manifest: unknown) => ({ ...project, manifest })),
  serviceDiscoverContracts: vi.fn(async () => [
    { kind: "openapi", path: "api/openapi.yaml", enabled: true },
    { kind: "proto", path: "proto/x.proto", include: ["proto"], enabled: true },
  ]),
  serviceScan: vi.fn(async (_p: string, _s: string, onProgress: (x: unknown) => void) => {
    onProgress({ service: "orders", done: 1, total: 2, file: "proto/x.proto" });
    return [report];
  }),
  scanCancel: vi.fn(async () => undefined),
  openPath: vi.fn(async () => undefined),
};
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));

describe("ServicePanel", () => {
  it("shows discovered contracts, toggles one into the manifest, scans and renders the report", async () => {
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    const proto = await screen.findByLabelText("proto/x.proto");
    expect(proto).toBeChecked();
    await userEvent.click(proto);
    expect(api.projectSave).toHaveBeenCalledWith("/p/orders", expect.objectContaining({ services: [expect.objectContaining({ contracts: [expect.objectContaining({ path: "api/openapi.yaml" }), expect.objectContaining({ path: "proto/x.proto", enabled: false })] })] }));
    await userEvent.click(screen.getByRole("button", { name: "Разобрать" }));
    expect(await screen.findByText(/Версия git недоступна/)).toBeInTheDocument();
    expect(screen.getByText(/найдено 3/)).toBeInTheDocument();
    expect(screen.getByText("proto/x.proto:7")).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("button", { name: "Открыть файл" })[0]);
    expect(api.openPath).toHaveBeenCalledWith("/p/orders", "/p/orders/../orders-service/api/openapi.yaml");
  });
});
```

- [ ] **Step 2: `ScanReportView.tsx`**

```tsx
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Diagnostic, ScanReport } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";

function DiagnosticList({ items, project, clone, title }: { items: Diagnostic[]; project: string; clone: string; title: string }) {
  const { t } = useTranslation();
  if (items.length === 0) return null;
  return (
    <details open={title === t("service.errors")}>
      <summary>
        {title} ({items.length})
      </summary>
      <ul>
        {items.map((d, i) => (
          <li key={i}>
            <code>{d.line ? `${d.path}:${d.line}` : d.path}</code> {d.message}{" "}
            <button type="button" className="link" onClick={() => api.openPath(project, joinProjectPath(clone, d.path))}>
              {t("service.openFile")}
            </button>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** Итог разбора сервиса: версия, счётчики, предупреждения и ошибки по файлу и строке. */
export function ScanReportView({ report, project, clone }: { report: ScanReport; project: string; clone: string }) {
  const { t } = useTranslation();
  const s = report.summary;
  return (
    <section className="panel">
      {report.git.available ? (
        <p>
          {t("service.gitVersion", { version: report.git.version })} <code className="muted">{report.git.commit.slice(0, 12)}</code>
        </p>
      ) : (
        <p className="warn">{t("service.gitUnavailable", { error: report.git.error ?? "" })}</p>
      )}
      {report.cancelled && <p className="warn">{t("service.cancelled")}</p>}
      <p>{t("service.counts", { contracts: report.contracts, found: report.operationsFound, tables: report.tablesFound })}</p>
      {!report.cancelled && <p>{t("service.summary", { added: s.added.length, changed: s.changed.length, removed: s.removed.length, restored: s.restored.length, unchanged: s.unchanged })}</p>}
      <DiagnosticList items={report.errors} project={project} clone={clone} title={t("service.errors")} />
      <DiagnosticList items={report.warnings} project={project} clone={clone} title={t("service.warnings")} />
    </section>
  );
}
```

- [ ] **Step 3: `ServicePanel.tsx`**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { ContractRef, Manifest, ScanProgress, ScanReport } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { ScanReportView } from "./ScanReportView";

export function ServicePanel({ path, serviceId }: { path: string; serviceId: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  useHelpChapter(tabId({ kind: "service", path, serviceId }), "project");
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectOpen(path) });
  const service = project.data?.manifest.services.find((s) => s.id === serviceId);
  const missing = project.data?.missingServices.includes(serviceId) ?? false;
  const discovered = useQuery({ queryKey: ["contracts", path, serviceId], queryFn: () => api.serviceDiscoverContracts(path, serviceId), enabled: !!service && !missing });
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [report, setReport] = useState<ScanReport | null>(null);
  const save = useMutation({
    mutationFn: (m: Manifest) => api.projectSave(path, m),
    onSuccess: (p) => qc.setQueryData(["project", path], p),
  });
  const scan = useMutation({
    mutationFn: () => api.serviceScan(path, serviceId, setProgress),
    onSuccess: (r) => {
      setReport(r[0] ?? null);
      setProgress(null);
      void qc.invalidateQueries({ queryKey: ["registry", path] });
      void qc.invalidateQueries({ queryKey: ["scans", path] });
    },
    onError: () => setProgress(null),
  });

  if (project.error) return <ErrorBanner error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <p className="empty">{t("common.loading")}</p>;
  if (!service) return <p className="empty">{t("service.notInManifest", { id: serviceId })}</p>;
  const clone = joinProjectPath(path, service.path);
  // Список контрактов: явный из манифеста, иначе найденный автопоиском (все включены).
  const explicit = service.contracts.length > 0;
  const contracts: ContractRef[] = explicit ? service.contracts : (discovered.data ?? []);
  const toggle = (c: ContractRef, enabled: boolean) => {
    const next = contracts.map((x) => (x.path === c.path && x.kind === c.kind ? { ...x, enabled } : { ...x, enabled: x.enabled ?? true }));
    const manifest = structuredClone(project.data.manifest);
    manifest.services = manifest.services.map((s) => (s.id === serviceId ? { ...s, contracts: next } : s));
    save.mutate(manifest);
  };

  return (
    <div className="page">
      <div className="toolbar">
        <h2>{service.name || service.id}</h2>
        <button type="button" disabled={scan.isPending || missing || project.data.readOnly} onClick={() => scan.mutate()}>
          {t("service.scan")}
        </button>
        {scan.isPending && (
          <button type="button" onClick={() => api.scanCancel(path, serviceId)}>
            {t("common.cancel")}
          </button>
        )}
        <button type="button" onClick={() => api.openPath(path, clone)} disabled={missing}>
          {t("project.openFolder")}
        </button>
      </div>
      <p>
        <code>{clone}</code> {missing && <span className="warn">{t("project.serviceMissing")}</span>}
      </p>
      <h3>
        {t("service.contracts")} <Hint k="hints.service.contracts" label={t("service.contracts")} />
      </h3>
      {discovered.error && <ErrorBanner error={discovered.error} onRetry={() => discovered.refetch()} />}
      {!explicit && discovered.isPending && !missing && <p className="muted">{t("common.loading")}</p>}
      {contracts.length === 0 && !discovered.isPending && <p className="empty">{t("service.noContracts")}</p>}
      <ul className="tree">
        {contracts.map((c) => (
          <li key={`${c.kind}:${c.path}`} className="row">
            <input type="checkbox" id={`c-${c.kind}-${c.path}`} checked={c.enabled ?? true} disabled={save.isPending || project.data.readOnly} onChange={(e) => toggle(c, e.target.checked)} />
            <label htmlFor={`c-${c.kind}-${c.path}`}>{c.path}</label>
            <span className="muted">{c.kind}</span>
            <button type="button" className="link" onClick={() => api.openPath(path, joinProjectPath(clone, c.path))}>
              {t("service.openFile")}
            </button>
          </li>
        ))}
      </ul>
      {!explicit && contracts.length > 0 && <p className="muted">{t("service.autoFound")}</p>}
      {save.error && <ErrorBanner error={save.error} />}
      {progress && <p className="muted">{t("project.scanProgress", { service: progress.service, done: progress.done, total: progress.total, file: progress.file })}</p>}
      {scan.error && <ErrorBanner error={scan.error} />}
      {report && <ScanReportView report={report} project={path} clone={clone} />}
    </div>
  );
}
```

В `TabContent.tsx`: `case "service": return <ServicePanel path={tab.path} serviceId={tab.serviceId} />;`. Ключи `service.*` (en/ru): `scan` «Scan»/«Разобрать», `contracts` «Contracts»/«Контракты», `noContracts` «No contracts found: add openapi/proto/asyncapi/avsc/sql files or Spring sources to the clone»/«Контракты не найдены: добавьте в клон файлы openapi/proto/asyncapi/avsc/sql или исходники Spring», `autoFound` «Found automatically; switching one off writes the list into the manifest»/«Найдены автоматически; выключение любого запишет список в манифест», `openFile` «Open file»/«Открыть файл», `gitVersion` «Version {{version}}»/«Версия {{version}}», `gitUnavailable` «Git version unavailable ({{error}}): operations are recorded as unversioned»/«Версия git недоступна ({{error}}): операции записаны как unversioned», `cancelled` «Scan cancelled: nothing was written»/«Разбор отменён: ничего не записано», `counts` «Contracts: {{contracts}}, operations found {{found}}, tables {{tables}}»/«Контрактов: {{contracts}}, найдено {{found}} операций, таблиц {{tables}}», `summary` «New {{added}}, changed {{changed}}, removed {{removed}}, restored {{restored}}, unchanged {{unchanged}}»/«Новых {{added}}, изменённых {{changed}}, удалённых {{removed}}, вернувшихся {{restored}}, без изменений {{unchanged}}», `errors` «Errors»/«Ошибки», `warnings` «Warnings»/«Предупреждения», `notInManifest` «Service {{id}} is not in the manifest»/«Сервиса {{id}} нет в манифесте»; `hints.service.contracts` «OpenAPI/Swagger, Protobuf, AsyncAPI, Avro and JSON Schema, Flyway/Liquibase migrations, Spring controllers. Each file is parsed on its own: an error in one does not stop the rest.»/«OpenAPI/Swagger, Protobuf, AsyncAPI, Avro и JSON Schema, миграции Flyway/Liquibase, контроллеры Spring. Каждый файл разбирается отдельно: ошибка в одном не останавливает остальные.»

- [ ] **Step 4: Run tests**

Run: `pnpm test && pnpm i18n:check`
Expected: `ServicePanel` (1) PASS и остальные без регрессий. В `pnpm tauri dev` на фикстуре `dev/fixtures/orders-openapi` разбор показывает 2 операции и предупреждение про callbacks.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(ui): вкладка «Сервис» — найденные контракты с переключателями, разбор с прогрессом и отчёт

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 17: Фронтенд — вкладка «Реестр операций»

**Files:**
- Create: `src/components/registry/{RegistryPanel,OperationDetails,SchemaTree,ManualOperationForm}.tsx`
- Modify: `src/components/TabContent.tsx`, `src/locales/{en,ru}.json`, `src/styles.css`
- Test: `src/components/registry/RegistryPanel.test.tsx`, `src/components/registry/SchemaTree.test.tsx`

**Interfaces:**
- Consumes: `api.registryList/registryGet/registryUpdateManual/registryAddManual/registryDelete/registryScans/registryDiff/openPath/projectOpen`, `useDebouncedValue` из `src/lib`.
- Produces: `RegistryPanel({ path })`, `SchemaTree({ schema })`.

- [ ] **Step 1: `SchemaTree.tsx` и тест**

```tsx
import { useTranslation } from "react-i18next";
import type { SchemaSummary } from "../../api/types";

/** Сжатая схема деревом: имя, тип, обязательность, enum, пример, пометка обрезки. */
export function SchemaTree({ schema }: { schema: SchemaSummary }) {
  const { t } = useTranslation();
  return (
    <ul className="schema-tree">
      <li>
        <code>{schema.name}</code> <span className="muted">{schema.type}</span>
        {schema.required && <span className="warn" title={t("registry.required")}> *</span>}
        {schema.enum && schema.enum.length > 0 && <span className="muted"> [{schema.enum.join(" | ")}]</span>}
        {schema.example !== undefined && <span className="muted"> = {schema.example}</span>}
        {schema.truncated && <span className="muted"> {t("registry.truncated")}</span>}
        {schema.children && schema.children.length > 0 && (
          <ul>
            {schema.children.map((c, i) => (
              <li key={`${c.name}-${i}`}>
                <SchemaTree schema={c} />
              </li>
            ))}
          </ul>
        )}
      </li>
    </ul>
  );
}
```

`SchemaTree.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { SchemaTree } from "./SchemaTree";

it("renders nested fields with markers", () => {
  render(<SchemaTree schema={{ name: "body", type: "object", children: [{ name: "id", type: "string", required: true, example: "A-1" }, { name: "status", type: "string", enum: ["NEW", "PAID"] }, { name: "deep", type: "object", truncated: true }] }} />);
  expect(screen.getByText("id").parentElement).toHaveTextContent("id string * = A-1");
  expect(screen.getByText("status").parentElement).toHaveTextContent("[NEW | PAID]");
  expect(screen.getByText("deep").parentElement).toHaveTextContent("обрезано");
});
```

- [ ] **Step 2: `ManualOperationForm.tsx`**

```tsx
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Address, OpKind, Operation } from "../../api/types";
import { ErrorBanner } from "../common/ErrorBanner";

const KINDS: OpKind[] = ["http", "kafka_produce", "kafka_consume", "grpc", "sql"];

function emptyAddress(kind: OpKind): Address {
  switch (kind) {
    case "http":
      return { type: "http", method: "GET", path: "/" };
    case "grpc":
      return { type: "grpc", service: "", method: "" };
    case "sql":
      return { type: "sql", name: "" };
    default:
      return { type: "kafka", topic: "" };
  }
}

/** Ручная операция: сервис, вид, адрес по виду, имя и описание. `id` считает Rust. */
export function ManualOperationForm({ path, services, onDone, onCancel }: { path: string; services: string[]; onDone: (op: Operation) => void; onCancel: () => void }) {
  const { t } = useTranslation();
  const [service, setService] = useState(services[0] ?? "");
  const [kind, setKind] = useState<OpKind>("sql");
  const [address, setAddress] = useState<Address>(emptyAddress("sql"));
  const [name, setName] = useState("");
  const [summary, setSummary] = useState("");
  const add = useMutation({
    mutationFn: () => api.registryAddManual(path, { id: "", service, kind, name, summary, address, source: { kind: "manual", path: "" } }),
    onSuccess: onDone,
  });
  const changeKind = (k: OpKind) => {
    setKind(k);
    setAddress(emptyAddress(k));
  };
  const field = (label: string, value: string, set: (v: string) => void) => (
    <label>
      {label}
      <input aria-label={label} value={value} onChange={(e) => set(e.target.value)} />
    </label>
  );
  return (
    <form
      className="form panel"
      onSubmit={(e) => {
        e.preventDefault();
        add.mutate();
      }}
    >
      <h3>{t("registry.addManual")}</h3>
      <div className="row">
        <label>
          {t("registry.service")}
          <select aria-label={t("registry.service")} value={service} onChange={(e) => setService(e.target.value)}>
            {services.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </label>
        <label>
          {t("registry.kind")}
          <select aria-label={t("registry.kind")} value={kind} onChange={(e) => changeKind(e.target.value as OpKind)}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`registry.kinds.${k}`)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="row">
        {address.type === "http" && field(t("registry.method"), address.method, (v) => setAddress({ ...address, method: v.toUpperCase() }))}
        {address.type === "http" && field(t("registry.path"), address.path, (v) => setAddress({ ...address, path: v }))}
        {address.type === "kafka" && field(t("registry.topic"), address.topic, (v) => setAddress({ ...address, topic: v }))}
        {address.type === "grpc" && field(t("registry.grpcService"), address.service, (v) => setAddress({ ...address, service: v }))}
        {address.type === "grpc" && field(t("registry.grpcMethod"), address.method, (v) => setAddress({ ...address, method: v }))}
        {address.type === "sql" && field(t("registry.sqlName"), address.name, (v) => setAddress({ ...address, name: v }))}
      </div>
      <div className="row">
        {field(t("registry.name"), name, setName)}
        {field(t("registry.summary"), summary, setSummary)}
      </div>
      {add.error && <ErrorBanner error={add.error} />}
      <div className="row">
        <button type="submit" disabled={add.isPending || !service}>
          {t("common.save")}
        </button>
        <button type="button" onClick={onCancel}>
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
```

- [ ] **Step 3: `OperationDetails.tsx`**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Address, Manual, Operation } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { ErrorBanner } from "../common/ErrorBanner";
import { SchemaTree } from "./SchemaTree";

export function addressLabel(a: Address): string {
  switch (a.type) {
    case "http":
      return `${a.method} ${a.path}`;
    case "kafka":
      return a.topic;
    case "grpc":
      return `${a.service}/${a.method}`;
    case "sql":
      return a.name;
  }
}

/** Правая панель реестра: адрес, источник, версии, схемы и ручные поля. */
export function OperationDetails({ path, id, clones, onDeleted }: { path: string; id: string; clones: Record<string, string>; onDeleted: () => void }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const op = useQuery({ queryKey: ["operation", path, id], queryFn: () => api.registryGet(path, id) });
  const [manual, setManual] = useState<Manual>({});
  useEffect(() => {
    if (op.data) setManual({ tags: op.data.manual?.tags ?? [], notes: op.data.manual?.notes ?? "", owner: op.data.manual?.owner ?? "" });
  }, [op.data]);
  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: ["registry", path] });
    void qc.invalidateQueries({ queryKey: ["operation", path, id] });
  };
  const save = useMutation({ mutationFn: (m: Manual) => api.registryUpdateManual(path, id, m), onSuccess: invalidate });
  const remove = useMutation({
    mutationFn: async () => {
      if (!(await ask(t("registry.deleteConfirm", { id }), { kind: "warning" }))) return false;
      await api.registryDelete(path, id);
      return true;
    },
    onSuccess: (deleted) => {
      if (deleted) {
        invalidate();
        onDeleted();
      }
    },
  });

  if (op.error) return <ErrorBanner error={op.error} onRetry={() => op.refetch()} />;
  const o: Operation | undefined = op.data;
  if (!o) return <p className="empty">{t("common.loading")}</p>;
  const clone = clones[o.service];
  const sourcePath = o.source.kind === "manual" || !clone ? null : joinProjectPath(clone, o.source.path);

  return (
    <div className="details-pane">
      <h3>
        <code>{o.id}</code>
      </h3>
      <p>
        <span className={o.status === "removed" ? "muted" : ""}>{t(`registry.status.${o.status}`)}</span> · {t(`registry.kinds.${o.kind}`)} · <code>{addressLabel(o.address)}</code>
        {o.streaming && <span className="muted"> · {t("registry.streaming", { mode: o.streaming })}</span>}
        {o.topic_guessed && <span className="warn"> · {t("registry.topicGuessed")}</span>}
      </p>
      <p>
        <strong>{o.name}</strong> {o.summary && <span className="muted">{o.summary}</span>}
      </p>
      <p>
        {t("registry.source", { kind: o.source.kind })}{" "}
        {sourcePath && (
          <>
            <code>{o.source.line ? `${o.source.path}:${o.source.line}` : o.source.path}</code>{" "}
            <button type="button" className="link" onClick={() => api.openPath(path, sourcePath)}>
              {t("service.openFile")}
            </button>
          </>
        )}
      </p>
      <p className="muted">{t("registry.seen", { first: o.first_seen.version, last: o.last_seen.version, at: o.last_seen.at.slice(0, 10) })}</p>
      {o.request && (
        <section>
          <h4>{t("registry.request")}</h4>
          <SchemaTree schema={o.request} />
        </section>
      )}
      {o.response && (
        <section>
          <h4>{t("registry.response")}</h4>
          <SchemaTree schema={o.response} />
        </section>
      )}
      {o.responses &&
        Object.entries(o.responses).map(([code, s]) => (
          <section key={code}>
            <h4>{t("registry.responseCode", { code })}</h4>
            <SchemaTree schema={s} />
          </section>
        ))}
      <section className="form">
        <h4>{t("registry.manual")}</h4>
        <label>
          {t("registry.tags")}
          <input aria-label={t("registry.tags")} value={(manual.tags ?? []).join(", ")} onChange={(e) => setManual({ ...manual, tags: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) })} />
        </label>
        <label>
          {t("registry.owner")}
          <input aria-label={t("registry.owner")} value={manual.owner ?? ""} onChange={(e) => setManual({ ...manual, owner: e.target.value })} />
        </label>
        <label>
          {t("registry.notes")}
          <textarea aria-label={t("registry.notes")} value={manual.notes ?? ""} onChange={(e) => setManual({ ...manual, notes: e.target.value })} />
        </label>
        {save.error && <ErrorBanner error={save.error} />}
        {remove.error && <ErrorBanner error={remove.error} />}
        <div className="row">
          <button type="button" disabled={save.isPending} onClick={() => save.mutate(manual)}>
            {t("common.save")}
          </button>
          <button type="button" className="danger" disabled={remove.isPending} onClick={() => remove.mutate()}>
            {o.source.kind === "manual" ? t("common.delete") : t("registry.deleteRemoved")}
          </button>
        </div>
      </section>
    </div>
  );
}
```

- [ ] **Step 4: `RegistryPanel.tsx`**

```tsx
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { OpKind, OperationRow, RegistryFilter, SourceKind, Status } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { useDebouncedValue } from "../../lib/useDebouncedValue";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { ManualOperationForm } from "./ManualOperationForm";
import { OperationDetails } from "./OperationDetails";

const KINDS: OpKind[] = ["http", "kafka_produce", "kafka_consume", "grpc", "sql"];
const SOURCES: SourceKind[] = ["openapi", "proto", "asyncapi", "schema", "sql", "spring", "manual"];
const ROW_HEIGHT = 32;

type Mark = "added" | "removed" | "changed";

export function RegistryPanel({ path }: { path: string }) {
  const { t } = useTranslation();
  useHelpChapter(tabId({ kind: "registry", path }), "registry");
  const [service, setService] = useState("");
  const [kind, setKind] = useState<OpKind | "">("");
  const [source, setSource] = useState<SourceKind | "">("");
  const [status, setStatus] = useState<Status | "">("active");
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, 200);
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [diffOn, setDiffOn] = useState(false);
  const filter: RegistryFilter = useMemo(
    () => ({ service: service || undefined, kind: kind || undefined, source: source || undefined, status: status || undefined, query: debounced || undefined }),
    [service, kind, source, status, debounced],
  );
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectOpen(path) });
  const rows = useQuery({ queryKey: ["registry", path, filter], queryFn: () => api.registryList(path, filter) });
  const scans = useQuery({ queryKey: ["scans", path], queryFn: () => api.registryScans(path) });
  const [from, to] = [scans.data?.[1]?.file, scans.data?.[0]?.file];
  const diff = useQuery({ queryKey: ["diff", path, from, to], queryFn: () => api.registryDiff(path, from!, to!), enabled: diffOn && !!from && !!to });
  const marks = useMemo(() => {
    const m = new Map<string, Mark>();
    if (!diffOn || !diff.data) return m;
    diff.data.added.forEach((id) => m.set(id, "added"));
    diff.data.removed.forEach((id) => m.set(id, "removed"));
    diff.data.changed.forEach((id) => m.set(id, "changed"));
    return m;
  }, [diffOn, diff.data]);
  const clones = useMemo(() => Object.fromEntries((project.data?.manifest.services ?? []).map((s) => [s.id, joinProjectPath(path, s.path)])), [project.data, path]);
  const parentRef = useRef<HTMLDivElement>(null);
  const list: OperationRow[] = rows.data ?? [];
  const virtual = useVirtualizer({ count: list.length, getScrollElement: () => parentRef.current, estimateSize: () => ROW_HEIGHT, overscan: 10, initialRect: { width: 800, height: 600 } });

  return (
    <div className="page registry">
      <div className="toolbar">
        <h2>
          {t("registry.title")} <Hint k="hints.registry.title" label={t("registry.title")} />
        </h2>
        <input aria-label={t("registry.search")} placeholder={t("registry.search")} value={query} onChange={(e) => setQuery(e.target.value)} />
        <select aria-label={t("registry.service")} value={service} onChange={(e) => setService(e.target.value)}>
          <option value="">{t("registry.anyService")}</option>
          {(project.data?.manifest.services ?? []).map((s) => (
            <option key={s.id} value={s.id}>
              {s.id}
            </option>
          ))}
        </select>
        <select aria-label={t("registry.kind")} value={kind} onChange={(e) => setKind(e.target.value as OpKind | "")}>
          <option value="">{t("registry.anyKind")}</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>
              {t(`registry.kinds.${k}`)}
            </option>
          ))}
        </select>
        <select aria-label={t("registry.sourceLabel")} value={source} onChange={(e) => setSource(e.target.value as SourceKind | "")}>
          <option value="">{t("registry.anySource")}</option>
          {SOURCES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <select aria-label={t("registry.statusLabel")} value={status} onChange={(e) => setStatus(e.target.value as Status | "")}>
          <option value="">{t("registry.anyStatus")}</option>
          <option value="active">{t("registry.status.active")}</option>
          <option value="removed">{t("registry.status.removed")}</option>
        </select>
        <label>
          <input type="checkbox" checked={diffOn} disabled={!from || !to} onChange={(e) => setDiffOn(e.target.checked)} /> {t("registry.diffToggle")}
        </label>
        <button type="button" onClick={() => setAdding(true)} disabled={!project.data || project.data.readOnly}>
          {t("registry.addManual")}
        </button>
        <span className="muted">{t("registry.count", { count: list.length })}</span>
      </div>
      {diffOn && scans.data && from && to && <p className="muted">{t("registry.diffBetween", { from: scans.data[1]?.at.slice(0, 16), to: scans.data[0]?.at.slice(0, 16) })}</p>}
      {rows.error && <ErrorBanner error={rows.error} onRetry={() => rows.refetch()} />}
      {adding && project.data && (
        <ManualOperationForm
          path={path}
          services={project.data.manifest.services.map((s) => s.id)}
          onDone={(op) => {
            setAdding(false);
            setSelected(op.id);
            void rows.refetch();
          }}
          onCancel={() => setAdding(false)}
        />
      )}
      <div className="registry-body">
        <div className="vlist" ref={parentRef} role="grid" aria-rowcount={list.length}>
          <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
            {virtual.getVirtualItems().map((v) => {
              const r = list[v.index];
              const mark = marks.get(r.id);
              return (
                <div
                  key={r.id}
                  role="row"
                  aria-selected={selected === r.id}
                  tabIndex={0}
                  className={["msg-row", selected === r.id ? "active" : "", mark ? `diff-${mark}` : "", r.status === "removed" ? "muted" : ""].join(" ")}
                  style={{ position: "absolute", top: v.start, height: ROW_HEIGHT, width: "100%" }}
                  onClick={() => setSelected(r.id)}
                  onKeyDown={(e) => e.key === "Enter" && setSelected(r.id)}
                >
                  <code>{r.id}</code>
                  <span className="muted">{r.name}</span>
                  <span className="muted">{r.source}</span>
                  <span className="muted">{r.lastVersion}</span>
                  {mark && <span className={`sev sev-${mark}`}>{t(`registry.marks.${mark}`)}</span>}
                </div>
              );
            })}
          </div>
          {rows.data && list.length === 0 && <p className="empty">{t("registry.empty")}</p>}
        </div>
        {selected && <OperationDetails path={path} id={selected} clones={clones} onDeleted={() => setSelected(null)} />}
      </div>
    </div>
  );
}
```

CSS в `styles.css` (добавить в конец):

```css
.registry-body { display: flex; gap: 12px; min-height: 0; flex: 1; }
.registry-body .vlist { flex: 1; overflow: auto; border: 1px solid var(--border); }
.registry-body .details-pane { width: 42%; overflow: auto; }
.msg-row { display: grid; grid-template-columns: 1fr auto auto auto auto; gap: 8px; align-items: center; padding: 0 8px; border-bottom: 1px solid var(--border); cursor: pointer; }
.msg-row.active { background: var(--selection, #e8eef7); }
.diff-added { border-left: 3px solid #16a34a; }
.diff-removed { border-left: 3px solid #8a8f98; }
.diff-changed { border-left: 3px solid #f59e0b; }
.schema-tree ul { margin: 0 0 0 16px; padding: 0; list-style: none; }
button.danger { color: #dc2626; }
```

В `TabContent.tsx`: `case "registry": return <RegistryPanel path={tab.path} />;`.

- [ ] **Step 5: Тест панели** — `RegistryPanel.test.tsx`:

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { RegistryPanel } from "./RegistryPanel";

const op = { id: "http:orders:POST /v1/orders", service: "orders", kind: "http", name: "createOrder", summary: "Create", address: { type: "http", method: "POST", path: "/v1/orders" }, request: { name: "request", type: "object", children: [{ name: "body", type: "object", required: true }] }, source: { kind: "openapi", path: "api/openapi.yaml", line: 4 }, fingerprint: "sha256:x", status: "active", first_seen: { version: "v1", commit: "", at: "2026-10-31T10:00:00Z" }, last_seen: { version: "v2", commit: "", at: "2026-11-01T10:00:00Z" }, manual: { tags: ["critical"] } };
const api = {
  projectOpen: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "P", services: [{ id: "orders", name: "", path: "../orders", contracts: [] }], environments: [] }, problems: [], readOnly: false, missingServices: [] })),
  registryList: vi.fn(async () => [
    { id: op.id, service: "orders", kind: "http", name: "createOrder", summary: "", source: "openapi", status: "active", lastVersion: "v2", topicGuessed: false },
    { id: "kafka_produce:orders.created", service: "orders", kind: "kafka_produce", name: "orders.created", summary: "", source: "schema", status: "active", lastVersion: "v2", topicGuessed: true },
  ]),
  registryGet: vi.fn(async () => op),
  registryUpdateManual: vi.fn(async () => op),
  registryScans: vi.fn(async () => [{ file: "registry/scans/b.yaml", at: "2026-11-01T10:00:00Z", service: "orders", version: "v2", added: 1, changed: 0, removed: 0, restored: 0 }, { file: "registry/scans/a.yaml", at: "2026-10-31T10:00:00Z", service: "orders", version: "v1", added: 1, changed: 0, removed: 0, restored: 0 }]),
  registryDiff: vi.fn(async () => ({ added: ["kafka_produce:orders.created"], removed: [], changed: [op.id] })),
  openPath: vi.fn(async () => undefined),
};
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(async () => true) }));

describe("RegistryPanel", () => {
  it("lists operations, shows details with schema and saves manual fields", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    expect(await screen.findByText("Операций: 2")).toBeInTheDocument();
    await userEvent.click(screen.getByText("createOrder"));
    expect(await screen.findByText("POST /v1/orders")).toBeInTheDocument();
    expect(screen.getByText("body").parentElement).toHaveTextContent("object *");
    await userEvent.click(screen.getByRole("button", { name: "Открыть файл" }));
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/../orders/api/openapi.yaml");
    const owner = screen.getByLabelText("Владелец");
    await userEvent.type(owner, "team-orders");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.registryUpdateManual).toHaveBeenCalledWith("/p", op.id, expect.objectContaining({ tags: ["critical"], owner: "team-orders" }));
  });

  it("marks rows from the diff of the last two scans", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    const toggle = await screen.findByRole("checkbox", { name: /Дифф/ });
    await userEvent.click(toggle);
    expect(await screen.findByText("изменена")).toBeInTheDocument();
    expect(screen.getByText("новая")).toBeInTheDocument();
    expect(api.registryDiff).toHaveBeenCalledWith("/p", "registry/scans/a.yaml", "registry/scans/b.yaml");
  });
});
```

Ключи `registry.*` (en / ru): `title` «Operation registry»/«Реестр операций», `search` «Search by id or name»/«Поиск по id или имени», `service` «Service»/«Сервис», `anyService` «All services»/«Все сервисы», `kind` «Kind»/«Вид», `anyKind` «All kinds»/«Все виды», `sourceLabel` «Source»/«Источник», `anySource` «All sources»/«Все источники», `statusLabel` «Status»/«Статус», `anyStatus` «All statuses»/«Все статусы», `status.active` «active»/«активна», `status.removed` «removed»/«удалена», `kinds.http` «HTTP», `kinds.kafka_produce` «Kafka produce»/«Kafka: отправка», `kinds.kafka_consume` «Kafka consume»/«Kafka: чтение», `kinds.grpc` «gRPC», `kinds.sql` «SQL», `diffToggle` «Diff with the previous scan»/«Дифф с прошлым разбором», `diffBetween` «Changes between {{from}} and {{to}}»/«Изменения между {{from}} и {{to}}», `marks.added` «new»/«новая», `marks.removed` «removed»/«удалена», `marks.changed` «changed»/«изменена», `addManual` «Add manually»/«Добавить вручную», `count` «Operations: {{count}}»/«Операций: {{count}}», `empty` «No operations: scan a service or add one manually»/«Операций нет: разберите сервис или добавьте вручную», `required` «required»/«обязательное», `truncated` «(truncated)»/«(обрезано)», `method` «Method»/«Метод», `path` «Path»/«Путь», `topic` «Topic»/«Топик», `grpcService` «gRPC service (package.Service)»/«gRPC-сервис (package.Service)», `grpcMethod` «Method»/«Метод», `sqlName` «Operation name»/«Имя операции», `name` «Name»/«Имя», `summary` «Description»/«Описание», `streaming` «streaming: {{mode}}»/«стриминг: {{mode}}», `topicGuessed` «topic guessed from the file name»/«топик угадан по имени файла», `source` «Source: {{kind}}»/«Источник: {{kind}}», `seen` «First seen in {{first}}, last in {{last}} ({{at}})»/«Впервые в {{first}}, последний раз в {{last}} ({{at}})», `request` «Request»/«Запрос», `response` «Response»/«Ответ», `responseCode` «Response {{code}}»/«Ответ {{code}}», `manual` «Manual fields»/«Ручные поля», `tags` «Tags (comma-separated)»/«Теги (через запятую)», `owner` «Owner»/«Владелец», `notes` «Notes»/«Заметки», `deleteConfirm` «Delete operation {{id}} from the registry?»/«Удалить операцию {{id}} из реестра?», `deleteRemoved` «Delete from the registry»/«Удалить из реестра»; `hints.registry.title` «Every operation of the system under test with its request and response schemas. Rescanning merges changes; manual fields survive.»/«Каждая операция тестируемой системы со схемами запроса и ответа. Повторный разбор сливает изменения; ручные поля сохраняются.»

Для `registry.count` с `{{count}}` в `ru.json` нужны формы `count_one` / `count_few` / `count_many` («Операций: {{count}}» во всех трёх), в `en.json` — `count_one` / `count_other`; тест ждёт «Операций: 2» (форма `few`).

- [ ] **Step 6: Run tests**

Run: `pnpm test && pnpm i18n:check`
Expected: `SchemaTree` (1), `RegistryPanel` (2) PASS. В `pnpm tauri dev` реестр на 5 000 операций (сгенерировать `registry/operations.yaml` скриптом или размножить фикстуру) прокручивается без задержек.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(ui): реестр операций — фильтры, виртуализированная таблица, схемы, ручные поля, дифф разборов

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 18: Фронтенд — вкладка «Нагрузка ПРОМ»: шаблон, предпросмотр импорта

**Files:**
- Create: `src/components/load/{ProdLoadPanel,ImportPreviewView,templateLabels}.tsx`
- Modify: `src/components/TabContent.tsx`, `src/locales/{en,ru}.json`
- Test: `src/components/load/ProdLoadPanel.test.tsx`, `src/components/load/templateLabels.test.ts`

**Interfaces:**
- Consumes: `api.prodLoadGet/prodLoadExportTemplate/prodLoadPreviewImport/prodLoadApplyImport/registryList/projectOpen`, диалоги `save`/`open` из `@tauri-apps/plugin-dialog`.
- Produces: `ProdLoadPanel({ path })`, `templateLabels(t)`.

- [ ] **Step 1: `templateLabels.ts` и тест**

```ts
import type { TFunction } from "i18next";
import type { TemplateLabels } from "../../api/types";

const COLUMN_KEYS = ["id", "service", "kind", "name", "summary", "avg", "peak", "errorPct", "reqBytes", "respBytes", "p95", "comment"] as const;
const PARAM_KEYS = ["stand", "periodFrom", "periodTo", "system", "filledBy"] as const;

/** Подписи листов и колонок на языке интерфейса (решение плана 5). */
export function templateLabels(t: TFunction): TemplateLabels {
  return {
    sheetOperations: t("template.sheets.operations"),
    sheetParams: t("template.sheets.params"),
    sheetHelp: t("template.sheets.help"),
    columns: COLUMN_KEYS.map((k) => t(`template.columns.${k}`)),
    notes: COLUMN_KEYS.map((k) => t(`template.notes.${k}`, { defaultValue: "" })),
    params: PARAM_KEYS.map((k) => t(`template.params.${k}`)),
    helpLines: t("template.help", { returnObjects: true }) as string[],
  };
}
```

`templateLabels.test.ts`:

```ts
import { i18n } from "../../i18n";
import { templateLabels } from "./templateLabels";

it("builds 12 columns, 12 notes and 5 params in the current language", () => {
  const l = templateLabels(i18n.t.bind(i18n));
  expect(l.columns).toHaveLength(12);
  expect(l.notes).toHaveLength(12);
  expect(l.params).toHaveLength(5);
  expect(l.columns[0]).toBe("ID");
  expect(l.notes[5]).toContain("сообщений/с");
  expect(Array.isArray(l.helpLines) && l.helpLines.length > 0).toBe(true);
});
```

Словарь `template` (en / ru): `sheets.operations` «Operations»/«Операции», `sheets.params` «Parameters»/«Параметры», `sheets.help` «Help»/«Справка»; `columns`: `id` «ID», `service` «Service»/«Сервис», `kind` «Kind»/«Тип», `name` «Operation»/«Операция», `summary` «Description»/«Описание», `avg` «Average RPS»/«Среднее RPS», `peak` «Peak RPS»/«Пиковое RPS», `errorPct` «Errors, %»/«Доля ошибок, %», `reqBytes` «Request size, bytes»/«Размер запроса, байт», `respBytes` «Response size, bytes»/«Размер ответа, байт», `p95` «p95 in production, ms»/«p95 ПРОМ, мс», `comment` «Comment»/«Комментарий»; `notes`: `avg` «For kafka_consume rows: messages per second»/«Для строк kafka_consume: сообщений/с», `peak` «Peak over the observation period; not less than the average»/«Пик за период наблюдения; не меньше среднего», `p95` «For kafka_consume rows: allowed consumer lag, seconds»/«Для строк kafka_consume: допустимый лаг потребителя, с», `respBytes` «Leave empty for Kafka rows»/«Для строк Kafka оставьте пустым»; `params`: `stand` «Production stand»/«Стенд ПРОМ», `periodFrom` «Observation period from»/«Период наблюдения с», `periodTo` «Observation period to»/«Период наблюдения по», `system` «Source of the figures (monitoring)»/«Источник цифр (мониторинг)», `filledBy` «Filled by»/«Заполнил»; `help` — массив строк: «Fill columns F–K on the Operations sheet with figures from production monitoring.», «Do not change column A (ID): the app matches rows by it.», «Rows you leave empty keep their previous figures in the project.», «Peak must not be below average; errors are a percentage from 0 to 100.» / «Заполните колонки F–K листа «Операции» цифрами из мониторинга ПРОМ.», «Колонку A (ID) не меняйте: по ней приложение сопоставляет строки.», «Пустые строки сохраняют прежние цифры в проекте.», «Пик не меньше среднего; доля ошибок — процент от 0 до 100.»

- [ ] **Step 2: `ImportPreviewView.tsx`**

```tsx
import { useTranslation } from "react-i18next";
import type { Figures, ImportPreview } from "../../api/types";

function figures(f: Figures | null): string {
  if (!f) return "—";
  const parts = [`${f.avg}/${f.peak}`];
  if (f.error_pct !== undefined) parts.push(`${f.error_pct}%`);
  if (f.p95_ms !== undefined) parts.push(`p95 ${f.p95_ms}`);
  if (f.lag_max_s !== undefined) parts.push(`lag ${f.lag_max_s}s`);
  return parts.join(" · ");
}

/** Предпросмотр импорта: счётчики, ошибки по строкам, дифф цифр; «Применить» только без ошибок. */
export function ImportPreviewView({ preview, onApply, onCancel, busy }: { preview: ImportPreview; onApply: () => void; onCancel: () => void; busy: boolean }) {
  const { t } = useTranslation();
  return (
    <section className="panel import-preview">
      <h3>{t("prodLoad.previewTitle")}</h3>
      <p>{t("prodLoad.previewCounts", { filled: preview.filled.length, empty: preview.empty.length, unknown: preview.unknown_ids.length, invalid: preview.invalid.length })}</p>
      <p className="muted">{t("prodLoad.sourceLine", { stand: preview.source.stand || "—", from: preview.source.period_from || "—", to: preview.source.period_to || "—", by: preview.source.filled_by || "—" })}</p>
      {preview.invalid.length > 0 && (
        <table className="import-warnings">
          <thead>
            <tr>
              <th>{t("prodLoad.row")}</th>
              <th>ID</th>
              <th>{t("prodLoad.column")}</th>
              <th>{t("prodLoad.value")}</th>
              <th>{t("prodLoad.reason")}</th>
            </tr>
          </thead>
          <tbody>
            {preview.invalid.map((c, i) => (
              <tr key={i}>
                <td>{c.row}</td>
                <td>
                  <code>{c.id}</code>
                </td>
                <td>{c.column}</td>
                <td>{c.value}</td>
                <td>{t(`prodLoad.reasons.${c.reason}`)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {preview.unknown_ids.length > 0 && (
        <details>
          <summary>{t("prodLoad.unknown", { count: preview.unknown_ids.length })}</summary>
          <ul>
            {preview.unknown_ids.map((id) => (
              <li key={id}>
                <code>{id}</code>
              </li>
            ))}
          </ul>
        </details>
      )}
      {preview.diff.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>ID</th>
              <th>{t("prodLoad.before")}</th>
              <th>{t("prodLoad.after")}</th>
            </tr>
          </thead>
          <tbody>
            {preview.diff.map((d) => (
              <tr key={d.id}>
                <td>
                  <code>{d.id}</code>
                </td>
                <td className="muted">{figures(d.old)}</td>
                <td>{figures(d.new)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="row">
        <button type="button" disabled={!preview.result || busy} onClick={onApply}>
          {t("prodLoad.apply")}
        </button>
        <button type="button" onClick={onCancel}>
          {t("common.cancel")}
        </button>
      </div>
    </section>
  );
}
```

- [ ] **Step 3: `ProdLoadPanel.tsx`**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { ImportPreview } from "../../api/types";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { ImportPreviewView } from "./ImportPreviewView";
import { templateLabels } from "./templateLabels";

const XLSX = [{ name: "Excel", extensions: ["xlsx"] }];

export function ProdLoadPanel({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  useHelpChapter(tabId({ kind: "prod-load", path }), "prod-load");
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectOpen(path) });
  const prod = useQuery({ queryKey: ["prodLoad", path], queryFn: () => api.prodLoadGet(path) });
  const rows = useQuery({ queryKey: ["registry", path, { status: "active" }], queryFn: () => api.registryList(path, { status: "active" }) });
  const [prefill, setPrefill] = useState(true);
  const [exported, setExported] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ file: string; data: ImportPreview } | null>(null);
  const exportTemplate = useMutation({
    mutationFn: async () => {
      const name = project.data?.manifest.name ?? "project";
      const date = new Date().toISOString().slice(0, 10);
      const file = await saveDialog({ defaultPath: `prod-load-${name}-${date}.xlsx`, filters: XLSX });
      if (!file) return null;
      const count = await api.prodLoadExportTemplate(path, file, templateLabels(t), prefill);
      return { file, count };
    },
    onSuccess: (r) => r && setExported(t("prodLoad.exported", { count: r.count, file: r.file })),
  });
  const importTemplate = useMutation({
    mutationFn: async () => {
      const file = await openDialog({ multiple: false, filters: XLSX });
      if (!file) return null;
      return { file, data: await api.prodLoadPreviewImport(path, file) };
    },
    onSuccess: (p) => setPreview(p),
  });
  const apply = useMutation({
    mutationFn: (file: string) => api.prodLoadApplyImport(path, file),
    onSuccess: (p) => {
      qc.setQueryData(["prodLoad", path], p);
      void qc.invalidateQueries({ queryKey: ["profileComputed", path] });
      setPreview(null);
    },
  });

  if (prod.error) return <ErrorBanner error={prod.error} onRetry={() => prod.refetch()} />;
  const data = prod.data;
  const list = rows.data ?? [];
  if (!data) return <p className="empty">{t("common.loading")}</p>;
  const uncovered = list.filter((r) => !data.operations[r.id]);

  return (
    <div className="page">
      <div className="toolbar">
        <h2>
          {t("prodLoad.title")} <Hint k="hints.prodLoad.title" label={t("prodLoad.title")} />
        </h2>
        <button type="button" disabled={exportTemplate.isPending || list.length === 0} onClick={() => exportTemplate.mutate()}>
          {t("prodLoad.export")}
        </button>
        <label>
          <input type="checkbox" checked={prefill} onChange={(e) => setPrefill(e.target.checked)} /> {t("prodLoad.prefill")}
        </label>
        <button type="button" disabled={importTemplate.isPending} onClick={() => importTemplate.mutate()}>
          {t("prodLoad.import")}
        </button>
      </div>
      {exported && <p className="notice">{exported}</p>}
      {exportTemplate.error && <ErrorBanner error={exportTemplate.error} />}
      {importTemplate.error && <ErrorBanner error={importTemplate.error} />}
      {apply.error && <ErrorBanner error={apply.error} />}
      {preview && <ImportPreviewView preview={preview.data} busy={apply.isPending} onApply={() => apply.mutate(preview.file)} onCancel={() => setPreview(null)} />}
      {data.source.imported_at && <p className="muted">{t("prodLoad.importedAt", { at: data.source.imported_at.slice(0, 16), stand: data.source.stand || "—", file: data.source.file })}</p>}
      {rows.error && <ErrorBanner error={rows.error} onRetry={() => rows.refetch()} />}
      <table>
        <thead>
          <tr>
            <th>ID</th>
            <th>{t("template.columns.kind")}</th>
            <th>{t("template.columns.avg")}</th>
            <th>{t("template.columns.peak")}</th>
            <th>{t("template.columns.errorPct")}</th>
            <th>{t("template.columns.p95")}</th>
            <th>{t("template.columns.comment")}</th>
          </tr>
        </thead>
        <tbody>
          {list.map((r) => {
            const f = data.operations[r.id];
            return (
              <tr key={r.id} className={f ? "" : "muted"}>
                <td>
                  <code>{r.id}</code>
                </td>
                <td>{t(`registry.kinds.${r.kind}`)}</td>
                <td>{f?.avg ?? "—"}</td>
                <td>{f?.peak ?? "—"}</td>
                <td>{f?.error_pct ?? "—"}</td>
                <td>{f ? (r.kind === "kafka_consume" ? (f.lag_max_s !== undefined ? `${f.lag_max_s} s` : "—") : (f.p95_ms ?? "—")) : "—"}</td>
                <td>{f?.comment ?? ""}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {uncovered.length > 0 && <p className="warn">{t("prodLoad.uncovered", { count: uncovered.length })}</p>}
    </div>
  );
}
```

В `TabContent.tsx`: `case "prod-load": return <ProdLoadPanel path={tab.path} />;`. Ключи `prodLoad.*` (en / ru): `title` «Production load»/«Нагрузка ПРОМ», `export` «Export template…»/«Выгрузить шаблон…», `prefill` «with current figures»/«с текущими цифрами», `import` «Import…»/«Импортировать…», `exported` «Template with {{count}} operations saved to {{file}}»/«Шаблон с {{count}} операциями сохранён в {{file}}», `importedAt` «Imported {{at}} from stand {{stand}}, copy {{file}}»/«Импортировано {{at}} со стенда {{stand}}, копия {{file}}», `uncovered` «Operations without production figures: {{count}}»/«Операций без цифр ПРОМ: {{count}}», `previewTitle` «Import preview»/«Предпросмотр импорта», `previewCounts` «Filled {{filled}}, empty {{empty}}, unknown IDs {{unknown}}, invalid {{invalid}}»/«Заполнено {{filled}}, пустых {{empty}}, неизвестных ID {{unknown}}, с ошибками {{invalid}}», `sourceLine` «Stand {{stand}}, period {{from}} — {{to}}, filled by {{by}}»/«Стенд {{stand}}, период {{from}} — {{to}}, заполнил {{by}}», `row` «Row»/«Строка», `column` «Column»/«Колонка», `value` «Value»/«Значение», `reason` «Problem»/«Проблема», `reasons.not_a_number` «not a number»/«не число», `reasons.negative` «negative»/«отрицательное», `reasons.not_an_integer` «not an integer»/«не целое», `reasons.percent_out_of_range` «percent outside 0–100»/«процент вне 0–100», `reasons.peak_below_avg` «peak below average»/«пик меньше среднего», `reasons.avg_missing` «average is missing»/«нет среднего», `unknown` «Unknown IDs ({{count}}), skipped»/«Неизвестные ID ({{count}}), пропущены», `before` «Before»/«Было», `after` «After»/«Станет», `apply` «Apply»/«Применить»; `hints.prodLoad.title` «The template is an .xlsx with locked IDs. The production team fills in the figures; import matches rows by ID and shows the diff before writing load/prod-load.yaml.»/«Шаблон — .xlsx с защищёнными ID. Команда ПРОМ вписывает цифры; импорт сопоставляет строки по ID и показывает дифф до записи load/prod-load.yaml.»

- [ ] **Step 4: Тест** — `ProdLoadPanel.test.tsx`:

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { ProdLoadPanel } from "./ProdLoadPanel";

const preview = { filled: ["http:o:POST /a"], empty: [], unknown_ids: ["http:o:GET /old"], invalid: [{ row: 3, id: "kafka_consume:t", column: "peak", value: "5", reason: "peak_below_avg" }], diff: [{ id: "http:o:POST /a", old: null, new: { avg: 1, peak: 2 } }], source: { stand: "prod", period_from: "", period_to: "", system: "", filled_by: "", imported_at: "", file: "" }, result: null };
const api = {
  projectOpen: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "orders", services: [], environments: [] }, problems: [], readOnly: false, missingServices: [] })),
  prodLoadGet: vi.fn(async () => ({ format: 1, source: { stand: "", period_from: "", period_to: "", system: "", filled_by: "", imported_at: "", file: "" }, operations: { "http:o:POST /a": { avg: 1, peak: 2 } } })),
  registryList: vi.fn(async () => [
    { id: "http:o:POST /a", service: "o", kind: "http", name: "a", summary: "", source: "openapi", status: "active", lastVersion: "v", topicGuessed: false },
    { id: "kafka_consume:t", service: "o", kind: "kafka_consume", name: "t", summary: "", source: "schema", status: "active", lastVersion: "v", topicGuessed: false },
  ]),
  prodLoadExportTemplate: vi.fn(async () => 2),
  prodLoadPreviewImport: vi.fn(async () => preview),
  prodLoadApplyImport: vi.fn(),
};
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(async () => "/out/t.xlsx"), open: vi.fn(async () => "/in/filled.xlsx") }));

describe("ProdLoadPanel", () => {
  it("exports a template with labels and shows uncovered operations", async () => {
    renderWithClient(<ProdLoadPanel path="/p" />);
    expect(await screen.findByText("Операций без цифр ПРОМ: 1")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Выгрузить шаблон…" }));
    expect(await screen.findByText(/сохранён в \/out\/t.xlsx/)).toBeInTheDocument();
    const [, file, labels, prefill] = api.prodLoadExportTemplate.mock.calls[0];
    expect(file).toBe("/out/t.xlsx");
    expect(labels.columns).toHaveLength(12);
    expect(prefill).toBe(true);
  });

  it("previews an import and blocks apply while there are invalid cells", async () => {
    renderWithClient(<ProdLoadPanel path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "Импортировать…" }));
    expect(await screen.findByText(/Заполнено 1, пустых 0, неизвестных ID 1, с ошибками 1/)).toBeInTheDocument();
    expect(screen.getByText("пик меньше среднего")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Применить" })).toBeDisabled();
  });
});
```

- [ ] **Step 5: Run tests**

Run: `pnpm test && pnpm i18n:check`
Expected: `templateLabels` (1), `ProdLoadPanel` (2) PASS. В `pnpm tauri dev`: выгрузить шаблон, заполнить в LibreOffice пару строк, импортировать — предпросмотр показывает дифф, «Применить» пишет `load/prod-load.yaml` и копию в `load/imports/`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(ui): вкладка «Нагрузка ПРОМ» — выгрузка шаблона, предпросмотр импорта с ошибками и диффом

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 19: Фронтенд — вкладка «Профиль нагрузки» с графиком фаз

**Files:**
- Create: `src/components/profile/{ProfilePanel,PhaseEditor,TimelineChart}.tsx`
- Modify: `src/components/TabContent.tsx`, `src/locales/{en,ru}.json`, `src/styles.css`
- Test: `src/components/profile/ProfilePanel.test.tsx`

**Interfaces:**
- Consumes: `api.profileGet/profileSave/profileCompute`, `uplot`.
- Produces: `ProfilePanel({ path })`, `PhaseEditor({ phases, onChange })`, `TimelineChart({ points, total })`.

- [ ] **Step 1: `TimelineChart.tsx`** (по образцу `$O/src/components/resources/UsageChart.tsx`)

```tsx
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import type { Point } from "../../api/types";

const HEIGHT = 220;

function themeColor(name: string, fallback: string): string {
  if (typeof getComputedStyle !== "function") return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

/** График профиля: ось X — минуты, левая ось — % цели, правая — суммарный RPS. Ширина следует за контейнером. */
export function TimelineChart({ points, total }: { points: Point[]; total: number }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el || points.length === 0) return;
    const muted = themeColor("--muted", "#6b7280");
    const axis = { stroke: muted, grid: { stroke: themeColor("--border", "#e3e5e8") }, ticks: { stroke: muted } };
    const plot = new uPlot(
      {
        width: el.clientWidth || 600,
        height: HEIGHT,
        scales: { x: { time: false }, pct: {}, rps: {} },
        axes: [
          { ...axis, label: t("profile.chartMinutes") },
          { ...axis, scale: "pct", label: "%", values: (_u: uPlot, v: number[]) => v.map((x) => `${x}%`) },
          { ...axis, scale: "rps", side: 1, label: "RPS", values: (_u: uPlot, v: number[]) => v.map((x) => x.toFixed(0)) },
        ],
        series: [
          {},
          { label: "%", scale: "pct", stroke: themeColor("--chart-1", "#3b82f6"), width: 2, value: (_u: uPlot, v: number | null) => (v == null ? "—" : `${v}%`) },
          { label: "RPS", scale: "rps", stroke: themeColor("--warn", "#f59e0b"), width: 1, dash: [4, 4], value: (_u: uPlot, v: number | null) => (v == null ? "—" : v.toFixed(1)) },
        ],
      },
      [points.map((p) => p.minute), points.map((p) => p.pct), points.map((p) => (p.pct / 100) * total)] as uPlot.AlignedData,
      el,
    );
    const resize = new ResizeObserver(() => plot.setSize({ width: el.clientWidth || 600, height: HEIGHT }));
    resize.observe(el);
    return () => {
      resize.disconnect();
      plot.destroy();
    };
  }, [points, total, t]);
  return <div className="usage-chart" ref={ref} data-testid="timeline-chart" />;
}
```

- [ ] **Step 2: `PhaseEditor.tsx`**

```tsx
import { useTranslation } from "react-i18next";
import type { Phase } from "../../api/types";

const KINDS: Phase["kind"][] = ["ramp", "steady", "steps"];

function blank(kind: Phase["kind"]): Phase {
  switch (kind) {
    case "ramp":
      return { kind, minutes: 10, to_pct: 100 };
    case "steady":
      return { kind, minutes: 30 };
    case "steps":
      return { kind, step_pct: 25, step_minutes: 10, max_pct: 200 };
  }
}

/** Список фаз: вид, числовые поля по виду, перестановка и удаление. */
export function PhaseEditor({ phases, onChange }: { phases: Phase[]; onChange: (p: Phase[]) => void }) {
  const { t } = useTranslation();
  const set = (i: number, p: Phase) => onChange(phases.map((x, j) => (j === i ? p : x)));
  const move = (i: number, d: -1 | 1) => {
    const j = i + d;
    if (j < 0 || j >= phases.length) return;
    const next = [...phases];
    [next[i], next[j]] = [next[j], next[i]];
    onChange(next);
  };
  const num = (i: number, p: Phase, key: string, label: string) => (
    <label key={key}>
      {label}
      <input type="number" aria-label={`${label} ${i + 1}`} min={0} step="any" value={(p as unknown as Record<string, number>)[key]} onChange={(e) => set(i, { ...p, [key]: Number(e.target.value) } as Phase)} />
    </label>
  );
  return (
    <ol className="steps">
      {phases.map((p, i) => (
        <li key={i} className="row">
          <select aria-label={t("profile.phaseKind")} value={p.kind} onChange={(e) => set(i, blank(e.target.value as Phase["kind"]))}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`profile.phases.${k}`)}
              </option>
            ))}
          </select>
          {p.kind === "ramp" && [num(i, p, "minutes", t("profile.minutes")), num(i, p, "to_pct", t("profile.toPct"))]}
          {p.kind === "steady" && num(i, p, "minutes", t("profile.minutes"))}
          {p.kind === "steps" && [num(i, p, "step_pct", t("profile.stepPct")), num(i, p, "step_minutes", t("profile.stepMinutes")), num(i, p, "max_pct", t("profile.maxPct"))]}
          <button type="button" className="icon" aria-label={t("profile.moveUp")} onClick={() => move(i, -1)}>
            ↑
          </button>
          <button type="button" className="icon" aria-label={t("profile.moveDown")} onClick={() => move(i, 1)}>
            ↓
          </button>
          <button type="button" className="icon" aria-label={t("common.delete")} onClick={() => onChange(phases.filter((_, j) => j !== i))}>
            ×
          </button>
        </li>
      ))}
      <li>
        <button type="button" onClick={() => onChange([...phases, blank("steady")])}>
          {t("profile.addPhase")}
        </button>
      </li>
    </ol>
  );
}
```

- [ ] **Step 3: `ProfilePanel.tsx`**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Basis, Override, ProfileFile } from "../../api/types";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { PhaseEditor } from "./PhaseEditor";
import { TimelineChart } from "./TimelineChart";

function OverrideTable({ title, rows, onChange }: { title: string; rows: Record<string, Override>; onChange: (r: Record<string, Override>) => void }) {
  const { t } = useTranslation();
  const [key, setKey] = useState("");
  const setRow = (k: string, o: Override) => onChange({ ...rows, [k]: o });
  return (
    <section>
      <h4>{title}</h4>
      <table>
        <tbody>
          {Object.entries(rows).map(([k, o]) => (
            <tr key={k}>
              <td>
                <code>{k}</code>
              </td>
              <td>
                <input type="number" step="any" aria-label={`k_env ${k}`} value={o.k_env ?? ""} onChange={(e) => setRow(k, { ...o, k_env: e.target.value === "" ? undefined : Number(e.target.value) })} />
              </td>
              <td>
                <input type="number" step="any" aria-label={`k_growth ${k}`} value={o.k_growth ?? ""} onChange={(e) => setRow(k, { ...o, k_growth: e.target.value === "" ? undefined : Number(e.target.value) })} />
              </td>
              <td>
                <button type="button" className="icon" aria-label={t("common.delete")} onClick={() => onChange(Object.fromEntries(Object.entries(rows).filter(([x]) => x !== k)))}>
                  ×
                </button>
              </td>
            </tr>
          ))}
          <tr>
            <td>
              <input aria-label={t("profile.overrideKey")} placeholder={t("profile.overrideKey")} value={key} onChange={(e) => setKey(e.target.value)} />
            </td>
            <td colSpan={3}>
              <button
                type="button"
                disabled={!key.trim()}
                onClick={() => {
                  setRow(key.trim(), {});
                  setKey("");
                }}
              >
                {t("profile.addOverride")}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </section>
  );
}

export function ProfilePanel({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  useHelpChapter(tabId({ kind: "profile", path }), "profile");
  const file = useQuery({ queryKey: ["profile", path], queryFn: () => api.profileGet(path) });
  const [draft, setDraft] = useState<ProfileFile | null>(null);
  const [name, setName] = useState("nominal");
  const [newName, setNewName] = useState("");
  useEffect(() => {
    if (file.data) {
      setDraft(structuredClone(file.data));
      if (!(name in file.data.profiles)) setName(Object.keys(file.data.profiles)[0] ?? "");
    }
  }, [file.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const save = useMutation({
    mutationFn: (p: ProfileFile) => api.profileSave(path, p),
    onSuccess: (p) => {
      qc.setQueryData(["profile", path], p);
      void qc.invalidateQueries({ queryKey: ["profileComputed", path] });
    },
  });
  const computed = useQuery({ queryKey: ["profileComputed", path, name, file.data], queryFn: () => api.profileCompute(path, name), enabled: !!file.data && !!name && name in (file.data?.profiles ?? {}) });

  if (file.error) return <ErrorBanner error={file.error} onRetry={() => file.refetch()} />;
  if (!draft || !file.data) return <p className="empty">{t("common.loading")}</p>;
  const dirty = JSON.stringify(draft) !== JSON.stringify(file.data);
  const profile = draft.profiles[name];
  const setProfile = (patch: Partial<NonNullable<typeof profile>>) => profile && setDraft({ ...draft, profiles: { ...draft.profiles, [name]: { ...profile, ...patch } } });
  const setCoef = (patch: Partial<ProfileFile["coefficients"]>) => setDraft({ ...draft, coefficients: { ...draft.coefficients, ...patch } });
  const c = computed.data;

  return (
    <div className="page">
      <div className="toolbar">
        <h2>
          {t("profile.title")} <Hint k="hints.profile.title" label={t("profile.title")} />
        </h2>
        <button type="button" disabled={!dirty || save.isPending} onClick={() => save.mutate(draft)}>
          {t("common.save")}
        </button>
        <button type="button" disabled={!dirty} onClick={() => setDraft(structuredClone(file.data!))}>
          {t("project.revert")}
        </button>
        {dirty && <span className="muted">{t("profile.unsaved")}</span>}
      </div>
      {save.error && <ErrorBanner error={save.error} />}
      <section className="form">
        <h3>
          {t("profile.coefficients")} <Hint k="hints.profile.coefficients" label={t("profile.coefficients")} />
        </h3>
        <div className="row">
          <label>
            k_env
            <input type="number" step="any" aria-label="k_env" value={draft.coefficients.k_env} onChange={(e) => setCoef({ k_env: Number(e.target.value) })} />
          </label>
          <label>
            k_growth
            <input type="number" step="any" aria-label="k_growth" value={draft.coefficients.k_growth} onChange={(e) => setCoef({ k_growth: Number(e.target.value) })} />
          </label>
        </div>
        <OverrideTable title={t("profile.serviceOverrides")} rows={draft.coefficients.overrides.services} onChange={(services) => setCoef({ overrides: { ...draft.coefficients.overrides, services } })} />
        <OverrideTable title={t("profile.operationOverrides")} rows={draft.coefficients.overrides.operations} onChange={(operations) => setCoef({ overrides: { ...draft.coefficients.overrides, operations } })} />
      </section>
      <section className="form">
        <h3>{t("profile.profiles")}</h3>
        <div className="row">
          <select aria-label={t("profile.profiles")} value={name} onChange={(e) => setName(e.target.value)}>
            {Object.keys(draft.profiles).map((n) => (
              <option key={n}>{n}</option>
            ))}
          </select>
          <input aria-label={t("profile.newName")} placeholder={t("profile.newName")} value={newName} onChange={(e) => setNewName(e.target.value)} />
          <button
            type="button"
            disabled={!newName.trim() || newName.trim() in draft.profiles}
            onClick={() => {
              setDraft({ ...draft, profiles: { ...draft.profiles, [newName.trim()]: { basis: "avg", phases: [{ kind: "ramp", minutes: 10, to_pct: 100 }, { kind: "steady", minutes: 30 }] } } });
              setName(newName.trim());
              setNewName("");
            }}
          >
            {t("profile.addProfile")}
          </button>
          <button type="button" disabled={Object.keys(draft.profiles).length <= 1 || !profile} onClick={() => setDraft({ ...draft, profiles: Object.fromEntries(Object.entries(draft.profiles).filter(([n]) => n !== name)) })}>
            {t("profile.removeProfile")}
          </button>
        </div>
        {profile && (
          <>
            <label>
              {t("profile.basis")}
              <select aria-label={t("profile.basis")} value={profile.basis} onChange={(e) => setProfile({ basis: e.target.value as Basis })}>
                <option value="avg">{t("profile.basisAvg")}</option>
                <option value="peak">{t("profile.basisPeak")}</option>
              </select>
            </label>
            <PhaseEditor phases={profile.phases} onChange={(phases) => setProfile({ phases })} />
          </>
        )}
      </section>
      {computed.error && <ErrorBanner error={computed.error} onRetry={() => computed.refetch()} />}
      {c && (
        <section>
          <h3>{t("profile.targets", { profile: c.profile, total: c.total.toFixed(1), minutes: c.total_minutes })}</h3>
          {dirty && <p className="muted">{t("profile.computedFromSaved")}</p>}
          <TimelineChart points={c.timeline} total={c.total} />
          <table>
            <thead>
              <tr>
                <th>ID</th>
                <th>{t("registry.service")}</th>
                <th>{t("profile.basisValue")}</th>
                <th>k_env</th>
                <th>k_growth</th>
                <th>{t("profile.target")}</th>
                <th>{t("profile.share")}</th>
              </tr>
            </thead>
            <tbody>
              {c.targets.map((x) => (
                <tr key={x.id}>
                  <td>
                    <code>{x.id}</code>
                  </td>
                  <td>{x.service}</td>
                  <td>{x.basis_value}</td>
                  <td>{x.k_env}</td>
                  <td>{x.k_growth}</td>
                  <td>{x.target.toFixed(2)}</td>
                  <td>{(x.share * 100).toFixed(1)}%</td>
                </tr>
              ))}
            </tbody>
          </table>
          {c.uncovered.length > 0 && (
            <details>
              <summary className="warn">{t("profile.uncovered", { count: c.uncovered.length })}</summary>
              <ul>
                {c.uncovered.map((id) => (
                  <li key={id}>
                    <code>{id}</code>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </section>
      )}
    </div>
  );
}
```

В `TabContent.tsx`: `case "profile": return <ProfilePanel path={tab.path} />;`. CSS: `.usage-chart { width: 100%; margin: 8px 0; }`. Ключи `profile.*` (en / ru): `title` «Load profile»/«Профиль нагрузки», `unsaved` «unsaved changes»/«есть несохранённые изменения», `coefficients` «Coefficients»/«Коэффициенты», `serviceOverrides` «Overrides by service»/«Переопределения по сервисам», `operationOverrides` «Overrides by operation»/«Переопределения по операциям», `overrideKey` «service id or operation id»/«id сервиса или операции», `addOverride` «Add»/«Добавить», `profiles` «Profiles»/«Профили», `newName` «new profile name»/«имя нового профиля», `addProfile` «Add profile»/«Добавить профиль», `removeProfile` «Remove profile»/«Удалить профиль», `basis` «Basis»/«Основа», `basisAvg` «average production load»/«средняя нагрузка ПРОМ», `basisPeak` «peak production load»/«пиковая нагрузка ПРОМ», `phaseKind` «Phase kind»/«Вид фазы», `phases.ramp` «ramp»/«разгон», `phases.steady` «steady»/«полка», `phases.steps` «steps»/«ступени», `minutes` «minutes»/«минут», `toPct` «to %»/«до %», `stepPct` «step %»/«шаг %», `stepMinutes` «minutes per step»/«минут на ступень», `maxPct` «max %»/«максимум %», `moveUp` «Move up»/«Выше», `moveDown` «Move down»/«Ниже», `addPhase` «Add phase»/«Добавить фазу», `targets` «Targets of {{profile}}: {{total}} RPS in total, {{minutes}} min»/«Цели профиля {{profile}}: всего {{total}} RPS, {{minutes}} мин», `computedFromSaved` «Computed from the saved file; save to recompute»/«Посчитано по сохранённому файлу; сохраните, чтобы пересчитать», `basisValue` «Basis RPS»/«Основа RPS», `target` «Target RPS»/«Цель RPS», `share` «Share»/«Доля», `uncovered` «Not covered by production figures: {{count}}»/«Не покрыто цифрами ПРОМ: {{count}}», `chartMinutes` «minutes»/«минуты»; `hints.profile.title` «Target RPS = production figure × k_env × k_growth; the share is its part of the sum. Phases set how the load changes over time; the runner (stage L2) executes them as is.»/«Цель RPS = цифра ПРОМ × k_env × k_growth; доля — её часть от суммы. Фазы задают изменение нагрузки во времени; раннер (этап L2) исполняет их как есть.», `hints.profile.coefficients` «k_env — the stand's capacity relative to production, k_growth — the growth margin. An operation override wins over a service one, a service one over the global.»/«k_env — мощность стенда относительно ПРОМ, k_growth — запас роста. Переопределение операции сильнее сервисного, сервисное — общего.»

- [ ] **Step 4: Тест** — `ProfilePanel.test.tsx`:

```tsx
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { ProfilePanel } from "./ProfilePanel";

const file = { format: 1, coefficients: { k_env: 1, k_growth: 1, overrides: { services: {}, operations: {} } }, profiles: { nominal: { basis: "avg", phases: [{ kind: "ramp", minutes: 10, to_pct: 100 }, { kind: "steady", minutes: 60 }] }, peak: { basis: "peak", phases: [{ kind: "steady", minutes: 30 }] } } };
const api = {
  profileGet: vi.fn(async () => file),
  profileSave: vi.fn(async (_p: string, f: unknown) => f),
  profileCompute: vi.fn(async (_p: string, name: string) => ({ profile: name, basis: "avg", targets: [{ id: "http:o:POST /a", service: "o", kind: "http", basis_value: 100, k_env: 1, k_growth: 1, target: 100, share: 1 }], total: 100, uncovered: ["http:o:GET /b"], timeline: [{ minute: 0, pct: 0 }, { minute: 10, pct: 100 }, { minute: 70, pct: 100 }], total_minutes: 70 })),
};
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("uplot", () => ({ default: class { setSize() {} destroy() {} } }));
vi.mock("uplot/dist/uPlot.min.css", () => ({}));

describe("ProfilePanel", () => {
  it("shows targets, edits a phase and saves the file", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    expect(await screen.findByText(/Цели профиля nominal: всего 100.0 RPS, 70 мин/)).toBeInTheDocument();
    expect(screen.getByText("100.0%")).toBeInTheDocument();
    expect(screen.getByText("Не покрыто цифрами ПРОМ: 1")).toBeInTheDocument();
    await userEvent.clear(screen.getByLabelText("k_env"));
    await userEvent.type(screen.getByLabelText("k_env"), "0.5");
    await userEvent.click(screen.getByRole("button", { name: "Добавить фазу" }));
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.profileSave).toHaveBeenCalledWith("/p", expect.objectContaining({ coefficients: expect.objectContaining({ k_env: 0.5 }), profiles: expect.objectContaining({ nominal: expect.objectContaining({ phases: [expect.anything(), expect.anything(), { kind: "steady", minutes: 30 }] }) }) }));
  });

  it("switches profiles and recomputes", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    await userEvent.selectOptions(screen.getByLabelText("Профили"), "peak");
    expect(await screen.findByText(/Цели профиля peak/)).toBeInTheDocument();
    expect(api.profileCompute).toHaveBeenLastCalledWith("/p", "peak");
  });
});
```

- [ ] **Step 5: Run tests**

Run: `pnpm test && pnpm i18n:check && pnpm build`
Expected: `ProfilePanel` (2) PASS. В `pnpm tauri dev` график фаз перерисовывается при смене профиля и ширины окна; правая ось показывает суммарный RPS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(ui): вкладка «Профиль нагрузки» — коэффициенты, фазы, цели и график на uPlot

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 20: Документация, руководство, CI, приёмка L1

**Files:**
- Create: `README.md`, `README.ru.md`, `CHANGELOG.md`, `docs/admin.md`, `docs/admin/{policy.json,policy.reg,policy.mobileconfig}`, `THIRD_PARTY.md`
- Modify: `docs/guide/{en,ru}/{getting-started,project,registry,prod-load,profile,updates}.md`, `.github/workflows/ci.yml`, `src/locales/{en,ru}.json` (ключи, на которые ссылается руководство)
- Test: `pnpm i18n:check` (главы и ссылки `[[ui:…]]`), полный прогон CI-команд, ручная приёмка по §15 спеки

**Interfaces:**
- Consumes: всё выше.
- Produces: релизная готовность L1 `0.1.0`.

- [ ] **Step 1: Руководство**

Главы на обоих языках по образцу `$G/docs/guide/en/getting-started.md`: заголовок `#`, разделы `## … {#anchor}`, ссылки на элементы интерфейса `[[ui:ключ]]` (их проверяет `i18n-check`). Содержание:

`getting-started.md` — что такое Glafira for Load Testing (два абзаца из `about.summary`), установка (адрес релизов `https://github.com/vvnovg/glafira-load-releases/releases`), «Первый проект» (шаги: [[ui:sidebar.newProject]] → имя и папка → [[ui:project.addService]] → выбрать клон → [[ui:service.scan]] → [[ui:sidebar.registry]]), «Язык интерфейса» ([[ui:sidebar.settings]], [[ui:settings.language.label]]), «Справка» (F1).

`project.md` — «Папка проекта» (список файлов §3.1 спеки и что коммитить), «Сервисы» (клон, версия git и `unversioned`, автопоиск контрактов по видам с масками §5.1, выключение контракта записывает список в манифест), «Стенды» (адреса без паролей), «Проблемы манифеста» (режим только для чтения).

`registry.md` — «Что такое операция» (`id` по правилам, виды), «Схемы» (сжатие: глубина 6, 200 полей, звёздочка — обязательное, `ref:` — цикл, `unresolved` — не найден), «Повторный разбор» (новые/изменённые/удалённые/вернувшиеся, ручные поля сохраняются), «Дифф» ([[ui:registry.diffToggle]]), «Ручные операции» ([[ui:registry.addManual]], SQL-операции и таблицы из миграций), «Предупреждения импортёров» (константы в Spring, callbacks, oneOf, угаданный топик).

`prod-load.md` — «Шаблон» ([[ui:prodLoad.export]], защищённые колонки, что заполнять, kafka_consume), «Импорт» ([[ui:prodLoad.import]], предпросмотр: заполнено/пусто/неизвестно/ошибки, дифф, [[ui:prodLoad.apply]], копия в `load/imports/`), «Файл load/prod-load.yaml» (поля `avg`, `peak`, …).

`profile.md` — «Коэффициенты» (формула, приоритет переопределений), «Профили и фазы» (ramp/steady/steps, три профиля по умолчанию), «Цели и доли» (таблица, не покрыто), «График», «Файл load/profile.yaml».

`updates.md` — из `$G`, с именем продукта; про обновления — «появятся в следующих версиях», политика `updatesEnabled`.

- [ ] **Step 2: README, CHANGELOG, admin**

`README.md` (en) по структуре `$G/README.md`: название, абзац из `about.summary`, пометка про товарные знаки (Apache Kafka), «Install» (релизы), «Features» (пять пунктов: реестр из контрактов, Excel-шаблон, импорт с предпросмотром, профиль с графиком, всё в git), «Stages» (таблица L1–L7 из концепции, L1 — done), «Development» (требования: Rust stable, Node 22, pnpm; команды `pnpm install`, `pnpm tauri dev`, `pnpm test`, `cargo test --workspace`, `pnpm i18n:check`), «License and support». `README.ru.md` — то же по-русски плюс «Где хранятся данные» (`settings.json` в каталоге данных приложения; проекты — там, где лежат) и «Политика администратора» (ссылка на `docs/admin.md`).

`CHANGELOG.md`:

```markdown
# Changelog

## 0.1.0 — 2026-11-xx

Stage L1 «Skeleton and operation registry»: load project as a folder in git; services from local git clones with
version from `git describe`; contract importers (OpenAPI/Swagger, Protobuf, AsyncAPI, Avro/JSON Schema,
Flyway/Liquibase migrations, Spring annotations); operation registry with merge, snapshots and diff; Excel
production-load template with locked IDs, import preview and diff; load profile with coefficients, phases, targets
and a timeline chart. Languages: English, Russian.
```

`docs/admin.md` — из `$G/docs/admin.md`: имена файлов установщиков `Glafira_for_Load_Testing_<версия>_…`, бинарь `glafira-load-testing`, домен политики `io.github.vvnovg.glafira.load`, реестр `HKLM\SOFTWARE\Policies\Glafira\LoadTesting`, Linux `/etc/io.github.vvnovg.glafira.load/policy.json`; ключи `updatesEnabled`, `updateUrl`, `language`; раздел «Где хранятся данные» (настройки приложения и список открытых проектов — в каталоге данных; проекты НТ — в репозиториях пользователя; секретов в L1 нет). `docs/admin/policy.*` — копии из `$G` с заменой идентификаторов.

`THIRD_PARTY.md`: `node scripts/third-party.mjs` (скрипт берёт `cargo metadata` workspace и `pnpm licenses`); если скрипт ждёт один манифест, передать `--manifest src-tauri/Cargo.toml` или поправить путь в скрипте (workspace-граф общий).

- [ ] **Step 3: CI и проверки**

`.github/workflows/ci.yml` — убедиться, что job `linux` выполняет по порядку: `pnpm install --frozen-lockfile`, `pnpm test`, `pnpm test:scripts`, `pnpm i18n:check`, `pnpm build`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny --manifest-path src-tauri/Cargo.toml --config deny.toml check licenses advisories`, `cargo test --workspace` (с `git` в PATH раннера тест `git::version` проходит); job `platforms` — `pnpm test`, `pnpm build`, `cargo test --workspace --lib`. `rust-cache` с `workspaces: . -> target`.

Run локально всю цепочку:

```bash
pnpm install --frozen-lockfile && pnpm test && pnpm test:scripts && pnpm i18n:check && pnpm build && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: всё зелёное. Затем `cargo deny --manifest-path src-tauri/Cargo.toml --config deny.toml check licenses advisories`: новые крейты (`serde_yaml_ng`, `rust_xlsxwriter`, `calamine`, `sqlparser`, `roxmltree`, `globset`, `ignore`, `protobuf` не используется) под MIT/Apache-2.0; если `cargo deny` найдёт лицензию вне белого списка (например, `Unicode-3.0` у зависимостей `calamine`), добавить её в `deny.toml` с комментарием.

- [ ] **Step 4: Ручная приёмка по §15 спеки** (`pnpm tauri dev`, проект в отдельной папке внутри временного git-репозитория)

1. «Новый проект…» → папка и имя → проект в дереве; добавить сервис с путём `dev/fixtures/orders-openapi` и второй с `dev/fixtures/orders-proto`; сохранить.
2. На вкладке сервиса видны найденные контракты; «Разобрать» → отчёт с версией `unversioned` (fixtures не репозиторий) и операциями; реестр показывает `http:…` и `grpc:…` со схемами; «Открыть файл» открывает контракт.
3. Поменять фикстуру (добавить поле в `schemas/order.yaml`), разобрать снова → в отчёте «изменённых 1», в реестре с включённым диффом строка помечена «изменена», снимки в `registry/scans/`.
4. «Выгрузить шаблон…» → открыть в LibreOffice: ID защищены, F–L редактируются; заполнить две строки, одну с ошибкой (пик меньше среднего) → «Импортировать…» показывает ошибку и блокирует «Применить»; исправить → применить → `load/prod-load.yaml` и копия в `load/imports/`.
5. Профиль: задать `k_env = 0.5`, добавить фазу, сохранить → цели, доли, «не покрыто», график; `load/profile.yaml` читаем в git diff.
6. Переключить язык на английский: все экраны переведены; F1 на каждом экране открывает свою главу; Tab/Enter проходят по дереву, таблице и формам.
7. `git status` в папке проекта показывает только ожидаемые файлы (`runs/` в `.gitignore`).

Найденные расхождения чинить в соответствующих модулях с тестом на каждое.

- [ ] **Step 5: Commit и тег**

```bash
git add -A
git commit -m "docs: руководство L1 на en/ru, README, администрирование, CHANGELOG 0.1.0, CI workspace

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
git tag v0.1.0
```

Публикация установщиков (job `platforms` по тегу, подписание, `latest.json`) — этап L7; тег фиксирует состояние L1.

---

## Самопроверка плана

**Покрытие спеки L1:** §1 объём — Tasks 1–20; §2 заимствования — Tasks 1–2; §2.1 идентификаторы — Task 1; §3 проект (папка, манифест, проверки, открытые проекты) — Tasks 3, 12, 15; §4 версия git — Task 12; §5.1 поиск — Task 13; §5.2 импортёры — Tasks 6–10; §5.3 разбор с прогрессом — Tasks 13, 16; §6 реестр, слияние, снимки, дифф — Tasks 3–4, 13, 17; §7 шаблон и импорт — Tasks 5, 11, 14, 18; §8 профиль и график — Tasks 5, 14, 19; §9 экраны — Tasks 15–19; §10 IPC — Tasks 12–15; §11 ошибки — Tasks 1, 12–14; §12 структура — Tasks 1–2; §13 тесты и CI — в каждой задаче и Task 20; §14 документация — Task 20; §15 критерии — Task 20 шаг 4.

**Имена и типы:** `ProjectDir::{open, root, file, read_text, write_text, write_bytes, manifest, save_manifest, registry, save_registry, prod_load, save_prod_load, profile, save_profile, view, service_root}` используются в Tasks 13–14 так, как объявлены в Task 12; `scan_service_blocking(dir, service_id, git, &progress, &cancel)` — одинаково в Tasks 13 (определение, тесты, команда); `preview_import(rows, source, known, current)` — Tasks 5, 11, 14; `TemplateLabels` — одинаковые поля в Rust (`camelCase` через serde) и TS; команды в `generate_handler!` совпадают с `api.*` по именам и аргументам (`serviceId`, `onProgress`, `profileName`, `prefill`).

**Проверенные ожидания строк в тестах:** OpenAPI `/v1/orders:` — строка 4; proto `rpc Create` — строка 39; AsyncAPI v2 `orders.created:` — строка 7; Spring `@PostMapping` — 9, `@GetMapping(Paths.SEARCH)` — 23, `kafkaTemplate.send` — 11; Liquibase-таблица `invoices` — 4 колонки после `addColumn`.
