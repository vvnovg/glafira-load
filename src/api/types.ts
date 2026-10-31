// Типы IPC-контракта с Rust. Поля — camelCase, как сериализует serde.
// Типы команд проекта добавляются вместе с командами.

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

export type Environment = "local" | "dev" | "test" | "stage" | "prod";

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
