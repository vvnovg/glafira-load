import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  AppInfo,
  Computed,
  ContractRef,
  LanguageSettings,
  Manifest,
  Manual,
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
  ScanMeta,
  ScanProgress,
  ScanReport,
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
  prodLoadExportTemplate: (path: string, file: string, labels: TemplateLabels, prefill: boolean) =>
    call<number>("prod_load_export_template", { path, file, labels, prefill }),
  prodLoadPreviewImport: (path: string, file: string) => call<TemplatePreview>("prod_load_preview_import", { path, file }),
  prodLoadApplyImport: (path: string, file: string) => call<ProdLoad>("prod_load_apply_import", { path, file }),
  prodLoadGet: (path: string) => call<ProdLoad>("prod_load_get", { path }),
  profileGet: (path: string) => call<ProfileFile>("profile_get", { path }),
  profileSave: (path: string, profile: ProfileFile) => call<ProfileFile>("profile_save", { path, profile }),
  profileCompute: (path: string, profileName: string) => call<Computed>("profile_compute", { path, profileName }),
};
