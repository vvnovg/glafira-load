import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../test/render";
import { useWorkspace } from "../state/workspace";
import { baseName } from "../lib/paths";
import { Layout } from "./Layout";

vi.mock("../api/client", () => ({
  api: {
    getAppInfo: vi.fn(async () => ({ name: "Glafira", version: "0.1.0", commit: "x", os: "macos", arch: "aarch64", dataDir: "/d", logDir: "/l" })),
    getLanguageSettings: vi.fn(async () => ({ effective: "ru", chosen: null, policy: null, systemMatch: null, reason: "default", languages: ["en", "ru"] })),
    getPolicyStatus: vi.fn(async () => ({ managed: false, source: null, updatesEnabled: false, updateUrl: null, language: null, errors: [] })),
    checkUpdate: vi.fn(async () => ({ status: "disabled", reason: { key: "errors.updates.notConfigured", params: {}, text: "x" } })),
    projectRecent: vi.fn(async () => []),
    suiteGet: vi.fn(async () => ({ name: "orders", file: "suites/orders/suite.yaml", text: "", suite: null, findings: [], lines: {} })),
    suiteCheck: vi.fn(async () => ({ findings: [], coverage: null, profile: "nominal" })),
  },
  toAppError: (e: unknown) => e,
  isAppError: () => false,
}));
// Запрос закрытия окна во время прогона приходит событием Tauri.
const events = vi.hoisted(() => ({ handlers: new Map<string, () => void>() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, h: () => void) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  }),
}));

describe("Layout", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("opens the registry tab with the project folder name and closes it", async () => {
    renderWithClient(<Layout />);
    act(() => useWorkspace.getState().open({ kind: "registry", path: "/home/u/orders-load" }));
    expect(await screen.findByRole("tab", { name: "orders-load · Реестр" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Закрыть вкладку" }));
    expect(screen.queryByRole("tab")).toBeNull();
    expect(baseName("C:\\work\\p\\")).toBe("p");
  });

  it("titles the suite and run tabs and renders their panels", async () => {
    renderWithClient(<Layout />);
    act(() => useWorkspace.getState().open({ kind: "suite", path: "/home/u/orders-load", name: "orders" }));
    // Вкладка сьюта — ленивый чанк: панель появляется после загрузки.
    expect(await screen.findByRole("heading", { name: /^Сьют orders/ })).toBeInTheDocument();
    act(() => useWorkspace.getState().open({ kind: "run", path: "/home/u/orders-load" }));
    expect(await screen.findByRole("tab", { name: "orders-load · orders" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "orders-load · Прогон" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "Прогон" })).toBeInTheDocument();
  });

  it("asks what to do with the run when the window is closed, even without the run tab", async () => {
    renderWithClient(<Layout />);
    await waitFor(() => expect(events.handlers.has("run-close-requested")).toBe(true));
    act(() => events.handlers.get("run-close-requested")?.());
    expect(screen.getByRole("alertdialog", { name: "Прогон идёт" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Отмена" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });
});
