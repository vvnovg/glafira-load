import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { RegistryPanel } from "./RegistryPanel";

// vi.mock поднимается над импортами — объекты моков создаются там же через vi.hoisted.
const { api, ask, op, twoRows, state } = vi.hoisted(() => {
  const op = {
    id: "http:orders:POST /v1/orders",
    service: "orders",
    kind: "http",
    name: "createOrder",
    summary: "Create",
    address: { type: "http", method: "POST", path: "/v1/orders" },
    request: { name: "request", type: "object", children: [{ name: "body", type: "object", required: true }] },
    source: { kind: "openapi", path: "api/openapi.yaml", line: 4 },
    fingerprint: "sha256:x",
    status: "active",
    first_seen: { version: "v1", commit: "", at: "2026-10-31T10:00:00Z" },
    last_seen: { version: "v2", commit: "", at: "2026-11-01T10:00:00Z" },
    manual: { tags: ["critical"] },
  };
  const twoRows = [
    { id: op.id, service: "orders", kind: "http", name: "createOrder", summary: "", source: "openapi", status: "active", lastVersion: "v2", topicGuessed: false },
    { id: "kafka_produce:orders.created", service: "orders", kind: "kafka_produce", name: "orders.created", summary: "", source: "schema", status: "active", lastVersion: "v2", topicGuessed: true },
  ];
  const state = { rows: twoRows as unknown[], readOnly: false };
  const api = {
    projectView: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "P", services: [{ id: "orders", name: "", path: "../orders", contracts: [] }], environments: [] }, problems: [], readOnly: state.readOnly, missingServices: [] })),
    registryList: vi.fn(async () => state.rows),
    registryGet: vi.fn(async () => op),
    registryUpdateManual: vi.fn(async () => op),
    registryAddManual: vi.fn(async (_p: string, d: { id: string }) => ({ ...op, id: "sql:orders:x", draft: d })),
    registryDelete: vi.fn(async () => undefined),
    registryScans: vi.fn(async () => [
      { file: "registry/scans/c.yaml", at: "2026-11-02T10:00:00Z", service: "orders", version: "v3", added: 0, changed: 1, removed: 0, restored: 0 },
      { file: "registry/scans/b.yaml", at: "2026-11-01T10:00:00Z", service: "orders", version: "v2", added: 1, changed: 0, removed: 0, restored: 0 },
      { file: "registry/scans/a.yaml", at: "2026-10-31T10:00:00Z", service: "orders", version: "v1", added: 1, changed: 0, removed: 0, restored: 0 },
    ]),
    registryDiff: vi.fn(async () => ({ added: ["kafka_produce:orders.created"], removed: [], changed: [op.id] })),
    registryTables: vi.fn(async () => ({ orders: [{ name: "orders", columns: [{ name: "id", dataType: "bigint", nullable: false }] }] })),
    openPath: vi.fn(async () => undefined),
  };
  return { api, ask: vi.fn(async () => true), op, twoRows, state };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask }));

beforeEach(() => {
  vi.clearAllMocks();
  state.rows = twoRows;
  state.readOnly = false;
  ask.mockResolvedValue(true);
});

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

  it("lets the tags field accept a trailing comma while typing", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByText("createOrder"));
    const tags = await screen.findByLabelText("Теги (через запятую)");
    await waitFor(() => expect(tags).toHaveValue("critical"));
    await userEvent.type(tags, ", slow");
    expect(tags).toHaveValue("critical, slow");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.registryUpdateManual).toHaveBeenCalledWith("/p", op.id, expect.objectContaining({ tags: ["critical", "slow"] }));
  });

  it("renders column headers and service, kind and guessed-topic cells in rows", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await screen.findByText("Операций: 2");
    const grid = screen.getByRole("grid");
    expect(within(grid).getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["ID", "Сервис", "Вид", "Имя", "Источник", "Версия", "Дифф"]);
    const http = screen.getByText("createOrder").closest("[role=row]") as HTMLElement;
    const kafka = screen.getByText("orders.created").closest("[role=row]") as HTMLElement;
    expect(within(http).getAllByRole("gridcell").map((c) => c.textContent)).toEqual([op.id, "orders", "HTTP", "createOrder", "openapi", "v2", ""]);
    expect(http).toHaveAttribute("aria-rowindex", "2");
    expect(within(http).queryByRole("img", { name: "топик угадан по имени файла" })).toBeNull();
    expect(within(kafka).getByText("Kafka: отправка")).toHaveClass("kind-badge");
    expect(within(kafka).getByRole("img", { name: "топик угадан по имени файла" })).toBeInTheDocument();
  });

  it("marks rows from the diff of the last two scans", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    const toggle = await screen.findByRole("checkbox", { name: /Дифф/ });
    await userEvent.click(toggle);
    expect(await screen.findByText("изменена")).toBeInTheDocument();
    expect(screen.getByText("новая")).toBeInTheDocument();
    expect(api.registryDiff).toHaveBeenCalledWith("/p", "registry/scans/b.yaml", "registry/scans/c.yaml");
  });

  it("shows removed operations while the diff is on", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    const status = await screen.findByLabelText("Статус");
    expect(status).toHaveValue("active");
    await userEvent.click(screen.getByRole("checkbox", { name: /Дифф/ }));
    expect(status).toHaveValue("");
    await waitFor(() => expect(api.registryList).toHaveBeenLastCalledWith("/p", expect.objectContaining({ status: undefined })));
  });

  it("diffs the two scans picked in the scans list", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByRole("checkbox", { name: /Дифф/ }));
    await userEvent.selectOptions(screen.getByLabelText("Сравнить от"), "registry/scans/a.yaml");
    await waitFor(() => expect(api.registryDiff).toHaveBeenLastCalledWith("/p", "registry/scans/a.yaml", "registry/scans/c.yaml"));
    expect(screen.getAllByText("добавлено 1, изменено 0, удалено 0, восстановлено 0")).toHaveLength(2);
  });

  it("passes the filter to the backend", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await screen.findByText("Операций: 2");
    await userEvent.selectOptions(screen.getByLabelText("Вид"), "kafka_produce");
    await userEvent.type(screen.getByLabelText("Поиск по id или имени"), "orders");
    await waitFor(() => expect(api.registryList).toHaveBeenLastCalledWith("/p", { service: undefined, kind: "kafka_produce", source: undefined, status: "active", query: "orders" }));
  });

  it("renders only a window of thousands of rows and keeps the total", async () => {
    state.rows = Array.from({ length: 5000 }, (_, i) => ({ id: `sql:orders:op${i}`, service: "orders", kind: "sql", name: `op${i}`, summary: "", source: "sql", status: "active", lastVersion: "v1", topicGuessed: false }));
    renderWithClient(<RegistryPanel path="/p" />);
    expect(await screen.findByText("Операций: 5000")).toBeInTheDocument();
    const rendered = screen.getAllByRole("row");
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered.length).toBeLessThan(100);
    // Строки данных плюс строка заголовков.
    expect(screen.getByRole("grid")).toHaveAttribute("aria-rowcount", "5001");
  });

  it("moves the selection with arrow keys and Enter selects", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    const first = (await screen.findByText("createOrder")).closest("[role=row]") as HTMLElement;
    first.focus();
    await userEvent.keyboard("{Enter}");
    expect(first).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard("{ArrowDown}");
    const second = screen.getByText("orders.created").closest("[role=row]") as HTMLElement;
    await waitFor(() => expect(second).toHaveAttribute("aria-selected", "true"));
    expect(second).toHaveFocus();
    await userEvent.keyboard("{Home}");
    await waitFor(() => expect(first).toHaveAttribute("aria-selected", "true"));
  });

  it("deletes an operation after confirmation", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByText("createOrder"));
    await userEvent.click(await screen.findByRole("button", { name: "Удалить из реестра" }));
    expect(ask).toHaveBeenCalledWith(expect.stringContaining(op.id), { kind: "warning" });
    await waitFor(() => expect(api.registryDelete).toHaveBeenCalledWith("/p", op.id));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Удалить из реестра" })).not.toBeInTheDocument());
  });

  it("does not delete when the confirmation is declined", async () => {
    ask.mockResolvedValue(false);
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByText("createOrder"));
    await userEvent.click(await screen.findByRole("button", { name: "Удалить из реестра" }));
    await waitFor(() => expect(ask).toHaveBeenCalled());
    expect(api.registryDelete).not.toHaveBeenCalled();
  });

  it("adds a manual operation with an id preview and validation", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "Добавить вручную" }));
    const form = screen.getByRole("heading", { name: "Добавить вручную" }).closest("form") as HTMLElement;
    const save = within(form).getByRole("button", { name: "Сохранить" });
    expect(save).toBeDisabled();
    expect(within(form).getByText("Заполните поле: Имя операции")).toBeInTheDocument();
    await userEvent.type(within(form).getByLabelText("Имя операции"), "load_orders");
    expect(within(form).getByText("sql:orders:load_orders")).toBeInTheDocument();
    await userEvent.selectOptions(within(form).getByLabelText("Вид"), "http");
    await userEvent.clear(within(form).getByLabelText("Путь"));
    await userEvent.type(within(form).getByLabelText("Путь"), "v1/items");
    expect(within(form).getByText("Путь должен начинаться с /")).toBeInTheDocument();
    expect(save).toBeDisabled();
    await userEvent.clear(within(form).getByLabelText("Путь"));
    await userEvent.type(within(form).getByLabelText("Путь"), "/v1/items");
    expect(within(form).getByText("http:orders:GET /v1/items")).toBeInTheDocument();
    await userEvent.selectOptions(within(form).getByLabelText("Вид"), "kafka_consume");
    await userEvent.type(within(form).getByLabelText("Топик"), "orders.paid");
    expect(within(form).getByText("kafka_consume:orders.paid")).toBeInTheDocument();
    await userEvent.click(save);
    await waitFor(() =>
      expect(api.registryAddManual).toHaveBeenCalledWith("/p", expect.objectContaining({ service: "orders", kind: "kafka_consume", address: { type: "kafka", topic: "orders.paid" }, source: { kind: "manual", path: "" } })),
    );
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Добавить вручную" })).not.toBeInTheDocument());
  });

  it("previews grpc ids and rejects spaces in names", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "Добавить вручную" }));
    const form = screen.getByRole("heading", { name: "Добавить вручную" }).closest("form") as HTMLElement;
    await userEvent.selectOptions(within(form).getByLabelText("Вид"), "grpc");
    await userEvent.type(within(form).getByLabelText("gRPC-сервис (package.Service)"), "pkg.Orders");
    await userEvent.type(within(form).getByLabelText("Метод"), "Get one");
    expect(within(form).getByText("Пробелы недопустимы в поле: Метод")).toBeInTheDocument();
    await userEvent.clear(within(form).getByLabelText("Метод"));
    await userEvent.type(within(form).getByLabelText("Метод"), "Get");
    expect(within(form).getByText("grpc:orders:pkg.Orders/Get")).toBeInTheDocument();
  });

  it("disables editing in a read-only project", async () => {
    state.readOnly = true;
    renderWithClient(<RegistryPanel path="/p" />);
    await userEvent.click(await screen.findByText("createOrder"));
    expect(await screen.findByRole("button", { name: "Сохранить" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Добавить вручную" })).toBeDisabled();
  });

  it("loads tables from migrations when the section is opened", async () => {
    renderWithClient(<RegistryPanel path="/p" />);
    await screen.findByText("Операций: 2");
    expect(api.registryTables).not.toHaveBeenCalled();
    await userEvent.click(screen.getByText("Таблицы из миграций"));
    expect(await screen.findByText("bigint")).toBeInTheDocument();
    expect(api.registryTables).toHaveBeenCalledWith("/p");
  });

  it("shows the empty state", async () => {
    state.rows = [];
    renderWithClient(<RegistryPanel path="/p" />);
    expect(await screen.findByText(/Операций нет/)).toBeInTheDocument();
  });
});
