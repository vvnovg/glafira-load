import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { useWorkspace } from "../../state/workspace";
import { ServicePanel } from "./ServicePanel";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const { api, state } = vi.hoisted(() => {
  const state = {
    project: {
      path: "/p/orders",
      manifest: { format: 1, name: "Orders", services: [{ id: "orders", name: "Orders", path: "../orders-service", contracts: [] as unknown[] } as Record<string, unknown>], environments: [] },
      problems: [],
      readOnly: false,
      missingServices: [] as string[],
    },
  };
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
    projectView: vi.fn(async () => state.project),
    projectSave: vi.fn(async (_p: string, manifest: unknown) => ({ ...state.project, manifest })),
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
  return { api, state };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => "/p/orders-service/src/new.proto") }));

const service = { id: "orders", name: "Orders", path: "../orders-service", contracts: [] as unknown[] };

describe("ServicePanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    state.project.manifest.services = [{ ...service }];
    state.project.readOnly = false;
    state.project.missingServices = [];
    useWorkspace.setState({ tabs: [], activeId: null });
  });

  it("shows discovered contracts, toggles one into the manifest, scans and renders the report", async () => {
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    const proto = await screen.findByLabelText("proto/x.proto");
    expect(proto).toBeChecked();
    await userEvent.click(proto);
    expect(api.projectSave).toHaveBeenCalledWith(
      "/p/orders",
      expect.objectContaining({ services: [expect.objectContaining({ contracts: [expect.objectContaining({ path: "api/openapi.yaml" }), expect.objectContaining({ path: "proto/x.proto", enabled: false })] })] }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Разобрать" }));
    expect(await screen.findByText(/Версия git недоступна/)).toBeInTheDocument();
    expect(screen.getByText(/найдено 3/)).toBeInTheDocument();
    expect(screen.getByText("proto/x.proto:7")).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("button", { name: "Открыть файл" })[0]);
    expect(api.openPath).toHaveBeenCalledWith("/p/orders", "/p/orders/../orders-service/api/openapi.yaml");
  });

  it("opens the scan snapshot and the registry tab from the report", async () => {
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    await screen.findByLabelText("proto/x.proto");
    await userEvent.click(screen.getByRole("button", { name: "Разобрать" }));
    await userEvent.click(await screen.findByRole("button", { name: "Открыть снимок разбора" }));
    expect(api.openPath).toHaveBeenCalledWith("/p/orders", "/p/orders/registry/scans/2026-10-31-120000.yaml");
    await userEvent.click(screen.getByRole("button", { name: "Открыть реестр" }));
    expect(useWorkspace.getState().tabs.map((t) => t.id)).toEqual(["registry:/p/orders"]);
  });

  it("edits kind, include and removes a contract of an explicit list", async () => {
    state.project.manifest.services = [{ ...service, contracts: [{ kind: "proto", path: "proto/x.proto" }, { kind: "sql", path: "db/V1.sql", enabled: false }] }];
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    expect(await screen.findByLabelText("db/V1.sql")).not.toBeChecked();
    await userEvent.type(screen.getByLabelText("Папки include контракта proto/x.proto"), "proto, shared");
    await userEvent.tab();
    expect(api.projectSave).toHaveBeenLastCalledWith("/p/orders", expect.objectContaining({ services: [expect.objectContaining({ contracts: [{ kind: "proto", path: "proto/x.proto", include: ["proto", "shared"] }, { kind: "sql", path: "db/V1.sql", enabled: false }] })] }));
    await userEvent.click(screen.getByRole("button", { name: "Убрать контракт db/V1.sql" }));
    expect(api.projectSave).toHaveBeenLastCalledWith("/p/orders", expect.objectContaining({ services: [expect.objectContaining({ contracts: [expect.objectContaining({ path: "proto/x.proto" })] })] }));
  });

  it("adds a contract by dialog relative to the service folder", async () => {
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    await screen.findByLabelText("proto/x.proto");
    await userEvent.click(screen.getByRole("button", { name: "Выбрать файл…" }));
    expect(screen.getByLabelText(/Путь нового контракта/)).toHaveValue("/p/orders-service/src/new.proto");
  });

  it("merges discovered contracts into an explicit list without duplicates", async () => {
    state.project.manifest.services = [{ ...service, contracts: [{ kind: "openapi", path: "api/openapi.yaml" }] }];
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    await screen.findByLabelText("api/openapi.yaml");
    await userEvent.click(screen.getByRole("button", { name: "Найти контракты" }));
    await vi.waitFor(() => expect(api.projectSave).toHaveBeenCalled());
    const saved = api.projectSave.mock.calls[0][1] as { services: { contracts: { path: string }[] }[] };
    expect(saved.services[0].contracts.map((c) => c.path)).toEqual(["api/openapi.yaml", "proto/x.proto"]);
  });

  it("edits the Kafka topic map", async () => {
    state.project.manifest.services = [{ ...service, contracts: [{ kind: "sql", path: "db/V1.sql" }], kafka: { topics: { "orders.created": "schemas/order.avsc" } } }];
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    expect(await screen.findByLabelText("Схема топика orders.created")).toHaveValue("schemas/order.avsc");
    await userEvent.selectOptions(screen.getByLabelText("Направление топика orders.created"), "produce");
    expect(api.projectSave).toHaveBeenLastCalledWith("/p/orders", expect.objectContaining({ services: [expect.objectContaining({ kafka: { topics: { "orders.created": { schema: "schemas/order.avsc", direction: "produce" } } } })] }));
    await userEvent.type(screen.getByLabelText("Новый топик"), "orders.paid");
    await userEvent.type(screen.getByLabelText("Файл схемы нового топика"), "schemas/paid.avsc");
    await userEvent.click(screen.getByRole("button", { name: "Добавить топик" }));
    expect(api.projectSave).toHaveBeenLastCalledWith("/p/orders", expect.objectContaining({ services: [expect.objectContaining({ kafka: { topics: { "orders.created": { schema: "schemas/order.avsc", direction: "produce" }, "orders.paid": "schemas/paid.avsc" } } })] }));
    await userEvent.click(screen.getByRole("button", { name: "Убрать топик orders.created" }));
    expect(api.projectSave).toHaveBeenLastCalledWith("/p/orders", expect.objectContaining({ services: [expect.objectContaining({ kafka: { topics: { "orders.paid": "schemas/paid.avsc" } } })] }));
  });

  it("warns about a missing folder and disables scanning", async () => {
    state.project.missingServices = ["orders"];
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    expect(await screen.findByText(/Папка сервиса не найдена/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Разобрать" })).toBeDisabled();
    expect(api.serviceDiscoverContracts).not.toHaveBeenCalled();
    expect(within(document.body).queryByText("proto/x.proto")).toBeNull();
  });

  it("marks a row that would duplicate another contract and does not save it", async () => {
    state.project.manifest.services = [{ ...service, contracts: [{ kind: "sql", path: "db/V1.sql" }, { kind: "sql", path: "db/V2.sql" }] }];
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    const path = await screen.findByLabelText("Путь контракта db/V2.sql");
    await userEvent.clear(path);
    await userEvent.type(path, "db/V1.sql");
    await userEvent.tab();
    expect(path).toHaveAttribute("aria-invalid", "true");
    expect(path).toHaveValue("db/V1.sql");
    expect(screen.getByText("Такой вид и путь уже есть в списке: не сохранено")).toBeInTheDocument();
    expect(api.projectSave).not.toHaveBeenCalled();
    // Вернули прежний путь — отметка снимается.
    await userEvent.clear(path);
    await userEvent.type(path, "db/V2.sql");
    await userEvent.tab();
    expect(path).not.toHaveAttribute("aria-invalid");
    expect(screen.queryByText("Такой вид и путь уже есть в списке: не сохранено")).toBeNull();
  });

  it("does not write the manifest when discovery finds nothing new", async () => {
    state.project.manifest.services = [{ ...service, contracts: [{ kind: "openapi", path: "api/openapi.yaml" }, { kind: "proto", path: "proto/x.proto" }] }];
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    await screen.findByLabelText("api/openapi.yaml");
    await userEvent.click(screen.getByRole("button", { name: "Найти контракты" }));
    await vi.waitFor(() => expect(api.serviceDiscoverContracts).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByRole("button", { name: "Найти контракты" })).toBeEnabled());
    expect(api.projectSave).not.toHaveBeenCalled();
  });

  it("does not write an empty contract list when nothing was found", async () => {
    api.serviceDiscoverContracts.mockResolvedValue([]);
    renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    expect(await screen.findByText(/Контракты не найдены/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Найти контракты" }));
    await waitFor(() => expect(api.serviceDiscoverContracts).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByRole("button", { name: "Найти контракты" })).toBeEnabled());
    expect(api.projectSave).not.toHaveBeenCalled();
  });

  it("merges discovered contracts into the latest manifest, not the one captured before the search", async () => {
    state.project.manifest.services = [{ ...service, contracts: [{ kind: "openapi", path: "api/openapi.yaml" }] }];
    const { client } = renderWithClient(<ServicePanel path="/p/orders" serviceId="orders" />);
    await screen.findByLabelText("api/openapi.yaml");
    let finish: (v: unknown) => void = () => undefined;
    api.serviceDiscoverContracts.mockImplementationOnce(() => new Promise((r) => (finish = r)) as never);
    await userEvent.click(screen.getByRole("button", { name: "Найти контракты" }));
    // Пока шёл поиск, манифест сохранили с топиком Kafka.
    const latest = { ...state.project, manifest: { ...state.project.manifest, services: [{ ...service, contracts: [{ kind: "openapi", path: "api/openapi.yaml" }], kafka: { topics: { "orders.paid": "schemas/paid.avsc" } } }] } };
    act(() => client.setQueryData(["project", "/p/orders"], latest));
    await act(async () => finish([{ kind: "proto", path: "proto/x.proto" }]));
    await vi.waitFor(() => expect(api.projectSave).toHaveBeenCalled());
    expect(api.projectSave).toHaveBeenCalledWith(
      "/p/orders",
      expect.objectContaining({ services: [expect.objectContaining({ contracts: [{ kind: "openapi", path: "api/openapi.yaml" }, { kind: "proto", path: "proto/x.proto" }], kafka: { topics: { "orders.paid": "schemas/paid.avsc" } } })] }),
    );
  });
});
