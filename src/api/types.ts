// Типы IPC-контракта с Rust. DTO приложения — camelCase, модель проекта — snake_case,
// как в файлах проекта; имена полей и теги перечислений — как сериализует serde.

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
  | "SCAN_BUSY"
  | "SUITE_INVALID"
  | "AGENT_UNREACHABLE"
  | "AGENT_BUSY"
  | "AGENT_VERSION"
  | "SECRET_MISSING"
  | "TOKEN_MISSING"
  | "RUN_BUSY";

/** Текст из Rust: ключ словаря, параметры и английский текст (запасной вариант). */
export interface LocalizedText {
  key: string;
  params: Record<string, string>;
  text: string;
}

export interface AppError {
  code: ErrorCode;
  /** Ключ словаря; у технического текста без перевода — `errors.generic` с `params.message`. */
  key?: string;
  params?: Record<string, string>;
  /** Английский текст (запасной вариант перевода). */
  message: string;
  details?: string;
  retryable: boolean;
}

/** Вид окружения для значка и цвета (`local` — без стенда). Стенд проекта — `Environment`. */
export type EnvBadge = "local" | "dev" | "test" | "stage" | "prod";

export interface PolicyStatus {
  managed: boolean;
  source: string | null;
  updatesEnabled: boolean;
  updateUrl: string | null;
  language: string | null;
  errors: LocalizedText[];
}

export interface AppInfo {
  name: string;
  version: string;
  commit: string;
  os: string;
  arch: string;
  dataDir: string;
  logDir: string;
}

export type UpdateCheck =
  | { status: "disabled"; reason: LocalizedText }
  | { status: "upToDate" }
  | { status: "available"; version: string; notes: string | null; notesI18n: Record<string, string>; date: string | null };

export type LanguageReason = "policy" | "user" | "system" | "default";

export interface LanguageSettings {
  system: string | null;
  systemMatch: string | null;
  chosen: string | null;
  policy: string | null;
  effective: string;
  reason: LanguageReason;
  languages: string[];
}

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
/** Агент нагрузки стенда (спека L2 §4.2); `weight` ≥ 1, serde всегда пишет его в ответ (по умолчанию 1). Токен — в хранилище ОС, не здесь. */
export interface Agent { id: string; url: string; weight: number }
export interface Environment {
  id: string;
  kind: EnvKind;
  http?: string;
  kafka?: string;
  grpc?: string;
  sql?: string;
  /** Schema Registry стенда для `encoding: avro`. */
  schema_registry?: string;
  /** Пустой список serde не пишет — поля может не быть. */
  agents?: Agent[];
}
export interface Manifest { format: number; name: string; services: Service[]; environments: Environment[] }
export type ManifestProblem =
  | { kind: "bad_format"; format: number }
  | { kind: "empty_name" }
  | { kind: "bad_service_id" | "duplicate_service_id" | "empty_service_path" | "bad_environment_id" | "duplicate_environment_id"; id: string }
  /** Проблемы агентов несут и стенд: одинаковый ID агента допустим в разных стендах. */
  | { kind: "bad_agent_id" | "duplicate_agent_id" | "bad_agent_url" | "bad_agent_weight"; env: string; id: string };

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
/** Ответ `prod_load_preview_import`: поля `ImportPreview` плюс строки с цифрами без ID, пропущенные при чтении. */
export interface TemplatePreview extends ImportPreview { skipped_rows: number }

export interface Override { k_env?: number; k_growth?: number }
/** Пустые карты переопределений serde не пишет — поля могут отсутствовать. */
export interface Coefficients { k_env: number; k_growth: number; overrides: { services?: Record<string, Override>; operations?: Record<string, Override> } }
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

// ---- Сьют (L2): snake_case, как `glafira-load-model::suite`; поля с `skip_serializing_if` — необязательные ----

export type DataMode = "slice" | "shared";
export interface DataSet { path: string; mode: DataMode }
export type Arrival = "poisson" | "constant";
export interface HttpDefaults { timeout_ms: number; headers: Record<string, string> }
export interface Defaults { http: HttpDefaults; kafka: Record<string, string>; arrival: Arrival; max_in_flight: number }
export type OnFail = "stop" | "continue";
/** Число, список чисел или класс `2xx`. */
export type StatusExpect = number | number[] | string;
export type Matcher = "exists" | { equals: unknown } | { matches: string } | { gt: number } | { lt: number };
/** Пустые проверки serde не пишет: у шага без `expect` поля нет вовсе. */
export interface Expect { status?: StatusExpect; jsonpath?: Record<string, Matcher>; schema?: "response"; max_ms?: number }
export interface HttpStep {
  operation: string;
  path?: Record<string, string>;
  query?: Record<string, string>;
  headers?: Record<string, string>;
  body?: unknown;
  body_text?: string;
  timeout_ms?: number;
}
export type Encoding = "json" | "avro";
export interface ProduceStep { operation: string; key?: string; value?: unknown; value_text?: string; headers?: Record<string, string>; encoding: Encoding }
export interface Match { key?: string; jsonpath?: Record<string, string> }
export interface ConsumeStep { operation: string; match?: Match; timeout_ms: number }
/** Ключ шага — вариант `Action` (serde `flatten` + `rename_all = "snake_case"`). */
export type StepAction = { http: HttpStep } | { kafka_produce: ProduceStep } | { kafka_consume: ConsumeStep };
/** `id` по умолчанию — `step<N>` (1-based), его вычисляет потребитель. */
export type Step = StepAction & { id?: string; expect?: Expect; extract?: Record<string, string> };
export interface Scenario { id: string; drives?: string; on_fail: OnFail; steps: Step[] }
export interface Suite { format: number; name: string; secrets?: string[]; data?: Record<string, DataSet>; defaults: Defaults; scenarios: Scenario[] }
export type Level = "error" | "warning";
export interface Finding { level: Level; file: string; line: number | null; message: string }
export interface CoverageRow { id: string; kind: OpKind; target: number; planned: number; delta: number }
export interface Coverage { rows: CoverageRow[]; uncovered: string[] }

// ---- Протокол агента (L2): `glafira-load-engine::agent_api`, serde camelCase ----

export type Mode = "run" | "smoke";
export type RunStatus = "completed" | "stopped" | "incomplete" | "failed";
/** `busy` — `run_id` идущего прогона или null. */
export interface AgentInfo { version: string; os: string; arch: string; cpus: number; busy: string | null }
export interface OpTick {
  id: string;
  n: number;
  err: Record<string, number>;
  dropped: number;
  p50: number;
  p90: number;
  p95: number;
  p99: number;
  max: number;
  bytesIn: number;
  bytesOut: number;
  lag: number | null;
  /** Гистограмма секунды (для слияния перцентилей контроллером); в событиях окна обычно нет. */
  hist?: string;
}
export interface ScenarioTick { id: string; n: number; err: Record<string, number>; dropped: number; p50: number; p95: number; p99: number; max: number; hist?: string }
export interface Tick { t: number; ops: OpTick[]; scenarios: ScenarioTick[] }
export interface PhaseEvent { t: number; index: number; pct: number }
export interface ErrorSample {
  t: number;
  code: string;
  operation: string;
  scenario: string;
  step: string;
  agent: string;
  message: string;
  request: string | null;
  response: string | null;
}
export interface OpSummary {
  id: string;
  n: number;
  rpsAvg: number;
  rpsPeak: number;
  err: Record<string, number>;
  errPct: number;
  dropped: number;
  p50: number;
  p90: number;
  p95: number;
  p99: number;
  max: number;
  target: number | null;
  achievedPct: number | null;
}
export interface ScenarioSummary { id: string; n: number; err: Record<string, number>; p50: number; p95: number; p99: number; max: number }
export interface AgentSummary { status: RunStatus; ops: OpSummary[]; scenarios: ScenarioSummary[]; seconds: number }

// ---- Файлы прогона и контроллер: snake_case, как `run.yaml`; пустые поля serde опускает ----

/** `url` есть только у агентов стенда (у встроенного `local` его нет), `version` — после опроса. */
export interface RunAgent { id: string; url?: string; weight: number; version?: string }
export interface RunMeta {
  run_id: string;
  suite: string;
  profile: string;
  env: string;
  mode: Mode;
  agents: RunAgent[];
  project_commit?: string;
  /** Сервис → версия из реестра. */
  services?: Record<string, string>;
  /** RFC 3339, UTC. */
  started_at: string;
  /** Нет у идущего или недостроенного прогона. */
  ended_at?: string;
  status?: RunStatus;
}
export type AgentRunState = "ready" | "running" | "lost" | "done" | "failed";
/** `dir` — абсолютный путь папки прогона. */
export interface RunResult { run_id: string; dir: string; status: RunStatus; summary: AgentSummary }
/** События `Channel` команды `run_start` (`#[serde(tag = "type", rename_all = "snake_case")]`, newtype-варианты сплющены). */
export type ControllerEvent =
  | { type: "started"; run_id: string; dir: string }
  | ({ type: "tick" } & Tick)
  | ({ type: "phase" } & PhaseEvent)
  | { type: "agent"; id: string; state: AgentRunState }
  | ({ type: "error" } & ErrorSample)
  | ({ type: "done" } & RunResult);

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
  /** Почему реестр не менялся (ни один контракт не разобран). */
  notice: LocalizedText | null;
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

// ---- DTO приложения L2: camelCase ----

/** `file` — `suites/<имя>/suite.yaml` относительно проекта; `scenarios = 0` — сьют не разобран. */
export interface SuiteMeta { name: string; file: string; scenarios: number }
/** `lines`: `"scenarios[0]"`, `"scenarios[0].steps[1]"` → строка YAML (1-based); пусто, если не разобран. */
export interface SuiteView { name: string; file: string; text: string; suite: Suite | null; findings: Finding[]; lines: Record<string, number> }
/** `coverage` — только если сьют разобран и профиль посчитан; `profile = ""` — профилей нет. */
export interface CheckReport { findings: Finding[]; coverage: Coverage | null; profile: string }
export type AgentProbeState = "ready" | "busy" | "unreachable" | "version" | "no_token";
/** `info` есть у `ready` и `busy`; `message`: busy — run_id, version — обе версии, unreachable — причина (технический текст). */
export interface AgentStatus { id: string; url: string; state: AgentProbeState; info: AgentInfo | null; message: string | null }
export interface SmokeStep { id: string; operation: string; ok: boolean; code?: string; message?: string }
export interface SmokeScenario { id: string; steps: SmokeStep[] }
export interface SmokeReport { scenarios: SmokeScenario[] }
/** Строка `run_list`: `RunMeta` плюс `dir` — `runs/<папка>` относительно проекта (`#[serde(flatten)]`). */
export type RunListEntry = RunMeta & { dir: string };
/** `dir` — `runs/<папка>` относительно проекта. */
export interface RunDetails { dir: string; meta: RunMeta; summary: AgentSummary | null; smoke: SmokeReport | null }
