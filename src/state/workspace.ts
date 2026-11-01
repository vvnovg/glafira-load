import { create } from "zustand";

export type TabSpec =
  /** Проект: манифест, сервисы, стенды. `path: null` — форма нового проекта. */
  | { kind: "project"; path: string | null }
  | { kind: "service"; path: string; serviceId: string }
  | { kind: "registry"; path: string }
  | { kind: "prod-load"; path: string }
  | { kind: "profile"; path: string }
  /** Сьют `suites/<name>/suite.yaml` проекта. */
  | { kind: "suite"; path: string; name: string }
  /** Прогон проекта: форма запуска, живые графики, прошлые прогоны. */
  | { kind: "run"; path: string }
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
    case "suite":
      return `suite:${spec.path}:${spec.name}`;
    case "registry":
    case "prod-load":
    case "profile":
    case "run":
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
