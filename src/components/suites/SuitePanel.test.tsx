import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { checkReport, suiteView } from "../../test/suite";
import { SuitePanel } from "./SuitePanel";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted; реализации — в beforeEach.
const api = vi.hoisted(() => ({
  suiteGet: vi.fn(),
  suiteCheck: vi.fn(),
  suiteCreate: vi.fn(),
  registryList: vi.fn(async () => []),
  openPath: vi.fn(async () => undefined),
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
// Фокус окна Tauri: обработчик `onFocusChanged` сохраняется, чтобы тест мог «вернуть» фокус.
const win = vi.hoisted(() => ({
  handler: null as ((e: { payload: boolean }) => void) | null,
  unlisten: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onFocusChanged: vi.fn(async (h: (e: { payload: boolean }) => void) => {
      win.handler = h;
      return () => {
        win.handler = null;
        win.unlisten();
      };
    }),
  }),
}));

describe("SuitePanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.suiteGet.mockImplementation(async () => suiteView());
    api.suiteCheck.mockImplementation(async () => checkReport());
  });

  it("shows the tree, the YAML of the selected step and opens the file", async () => {
    renderWithClient(<SuitePanel path="/p" name="orders" />);
    expect(await screen.findByRole("heading", { name: /Сьют orders/ })).toBeInTheDocument();
    expect(api.suiteGet).toHaveBeenCalledWith("/p", "orders");
    expect(api.suiteCheck).toHaveBeenCalledWith("/p", "orders");
    await userEvent.click(screen.getByRole("button", { name: /create http:orders:POST/ }));
    expect(screen.getByTestId("step-yaml")).toHaveTextContent("- id: create");
    await userEvent.click(screen.getAllByRole("button", { name: "Открыть файл" })[0]);
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/suites/orders/suite.yaml");
  });

  it("shows the check report and the coverage, and re-reads both on Check", async () => {
    renderWithClient(<SuitePanel path="/p" name="orders" />);
    expect(await screen.findByText("operation http:orders:POST /v1/orders is not in the registry")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "suites/orders/suite.yaml:8" }));
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/suites/orders/suite.yaml");
    expect(screen.getByRole("heading", { name: /Покрытие профиля nominal/ })).toBeInTheDocument();
    expect(screen.getAllByTestId("delta")[1]).toHaveClass("warn");
    await userEvent.click(screen.getByRole("button", { name: "Проверить" }));
    await waitFor(() => expect(api.suiteGet).toHaveBeenCalledTimes(2));
    expect(api.suiteCheck).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(screen.getByRole("button", { name: "Проверить" })).toBeEnabled());
  });

  it("re-reads the file when the window regains focus even though the app turns that off globally", async () => {
    // Как во `App`: перезапрос по фокусу выключен по умолчанию — вкладка включает его своим запросам.
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
    render(
      <QueryClientProvider client={client}>
        <SuitePanel path="/p" name="orders" />
      </QueryClientProvider>,
    );
    await screen.findByText("operation http:orders:POST /v1/orders is not in the registry");
    api.suiteGet.mockImplementation(async () => ({ ...suiteView(), name: "orders", suite: { ...suiteView().suite!, scenarios: [{ ...suiteView().suite!.scenarios[0], id: "order-flow-2" }] } }));
    // TanStack Query слушает `visibilitychange`: имитируем возврат в окно.
    window.dispatchEvent(new Event("visibilitychange"));
    expect(await screen.findByText("order-flow-2")).toBeInTheDocument();
    await waitFor(() => expect(api.suiteCheck).toHaveBeenCalledTimes(2));
  });

  it("re-reads the file when the Tauri window regains focus and unsubscribes on unmount", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
    const { unmount } = render(
      <QueryClientProvider client={client}>
        <SuitePanel path="/p" name="orders" />
      </QueryClientProvider>,
    );
    await screen.findByText("operation http:orders:POST /v1/orders is not in the registry");
    await waitFor(() => expect(win.handler).not.toBeNull());
    expect(api.suiteGet).toHaveBeenCalledTimes(1);
    api.suiteGet.mockImplementation(async () => ({ ...suiteView(), suite: { ...suiteView().suite!, scenarios: [{ ...suiteView().suite!.scenarios[0], id: "order-flow-3" }] } }));
    // Уход из окна (в редактор) ничего не перечитывает.
    act(() => win.handler?.({ payload: false }));
    expect(api.suiteGet).toHaveBeenCalledTimes(1);
    // Возврат фокуса — без `visibilitychange`, как в окне Tauri.
    act(() => win.handler?.({ payload: true }));
    expect(await screen.findByText("order-flow-3")).toBeInTheDocument();
    await waitFor(() => expect(api.suiteCheck).toHaveBeenCalledTimes(2));
    unmount();
    await waitFor(() => expect(win.unlisten).toHaveBeenCalledTimes(1));
  });

  it("falls back to the findings of suite_get and explains the missing coverage", async () => {
    api.suiteGet.mockImplementation(async () => ({ ...suiteView(), suite: null, findings: [{ level: "error", file: "suites/orders/suite.yaml", line: 3, message: "unknown field `scenario`" }] }));
    api.suiteCheck.mockImplementation(async () => ({ findings: [{ level: "error", file: "suites/orders/suite.yaml", line: 3, message: "unknown field `scenario`" }], coverage: null, profile: "nominal" }));
    renderWithClient(<SuitePanel path="/p" name="orders" />);
    expect(await screen.findByText("unknown field `scenario`")).toBeInTheDocument();
    expect(screen.getByText("Сьют не разобран: смотрите отчёт проверки")).toBeInTheDocument();
    expect(await screen.findByText("Покрытия пока нет: сначала исправьте ошибки")).toBeInTheDocument();
  });

  it("opens the create-suite dialog from the toolbar", async () => {
    renderWithClient(<SuitePanel path="/p" name="orders" />);
    await userEvent.click(await screen.findByRole("button", { name: "Создать сьют…" }));
    expect(screen.getByRole("dialog", { name: "Новый сьют" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Отмена" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
