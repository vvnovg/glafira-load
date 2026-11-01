import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AgentStatus,
  AppError,
  AppInfo,
  CheckReport,
  Computed,
  ContractRef,
  ControllerEvent,
  LanguageSettings,
  Manifest,
  Manual,
  Mode,
  Operation,
  OperationDraft,
  OperationRow,
  PolicyStatus,
  ProdLoad,
  ProfileFile,
  Project,
  RecentProject,
  RegistryDiff,
  RegistryFilter,
  RunDetails,
  RunListEntry,
  RunResult,
  ScanMeta,
  ScanProgress,
  ScanReport,
  SuiteMeta,
  SuiteView,
  Table,
  TemplateLabels,
  TemplatePreview,
  UpdateCheck,
} from "./types";

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

export function toAppError(e: unknown): AppError {
  // Ошибка из Rust приходит целиком: key/params/details сохраняются как есть.
  if (isAppError(e)) return e;
  if (e instanceof Error) return { code: "INTERNAL", message: e.message, retryable: false };
  return { code: "INTERNAL", message: String(e), retryable: false };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

export const api = {
  getAppInfo: () => call<AppInfo>("get_app_info"),
  getLanguageSettings: () => call<LanguageSettings>("get_language_settings"),
  setLanguage: (lang: string | null) => call<LanguageSettings>("set_language", { lang }),
  getPolicyStatus: () => call<PolicyStatus>("get_policy_status"),
  checkUpdate: () => call<UpdateCheck>("check_update"),
  installUpdate: () => call<void>("install_update"),
  projectCreate: (path: string, name: string) => call<Project>("project_create", { path, name }),
  /** Явное открытие: проект поднимается в недавних и становится активным. */
  projectOpen: (path: string) => call<Project>("project_open", { path }),
  /** Только чтение проекта (дерево, панели): недавние и активный проект не меняются. */
  projectView: (path: string) => call<Project>("project_view", { path }),
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
  prodLoadExportTemplate: (path: string, file: string, labels: TemplateLabels, prefill: boolean) =>
    call<number>("prod_load_export_template", { path, file, labels, prefill }),
  prodLoadPreviewImport: (path: string, file: string) => call<TemplatePreview>("prod_load_preview_import", { path, file }),
  prodLoadApplyImport: (path: string, file: string) => call<ProdLoad>("prod_load_apply_import", { path, file }),
  prodLoadGet: (path: string) => call<ProdLoad>("prod_load_get", { path }),
  profileGet: (path: string) => call<ProfileFile>("profile_get", { path }),
  profileSave: (path: string, profile: ProfileFile) => call<ProfileFile>("profile_save", { path, profile }),
  profileCompute: (path: string, profileName: string) => call<Computed>("profile_compute", { path, profileName }),
  suiteList: (path: string) => call<SuiteMeta[]>("suite_list", { path }),
  suiteGet: (path: string, name: string) => call<SuiteView>("suite_get", { path, name }),
  suiteCheck: (path: string, name: string) => call<CheckReport>("suite_check", { path, name }),
  suiteCreate: (path: string, name: string, operationIds: string[]) => call<SuiteMeta>("suite_create", { path, name, operationIds }),
  /** `token = null` (или пробелы) удаляет токен из хранилища ОС. */
  agentSetToken: (path: string, envId: string, agentId: string, token: string | null) => call<void>("agent_set_token", { path, envId, agentId, token }),
  /** Опрос агентов сохранённого манифеста; порядок — порядок агентов стенда. */
  agentProbe: (path: string, envId: string) => call<AgentStatus[]>("agent_probe", { path, envId }),
  /** Прогон: события идут в `onEvent`, промис завершается итогом или ошибкой запуска. */
  runStart: (path: string, suite: string, profile: string, env: string, mode: Mode, maxInFlight: number | null, onEvent: (e: ControllerEvent) => void) => {
    const channel = new Channel<ControllerEvent>();
    channel.onmessage = onEvent;
    return call<RunResult>("run_start", { path, suite, profile, env, mode, maxInFlight, onEvent: channel });
  },
  runStop: (path: string) => call<void>("run_stop", { path }),
  runList: (path: string) => call<RunListEntry[]>("run_list", { path }),
  /** `dir` — `runs/<папка>` из `RunListEntry.dir`. */
  runGet: (path: string, dir: string) => call<RunDetails>("run_get", { path, dir }),
  /** Ответ на событие `run-close-requested`: остановить прогоны и закрыть окно. */
  confirmClose: () => call<void>("confirm_close"),
};
