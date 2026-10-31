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
  | "SCAN_BUSY";

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
