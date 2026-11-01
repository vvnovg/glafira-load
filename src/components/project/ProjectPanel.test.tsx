import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { useWorkspace } from "../../state/workspace";
import { ProjectPanel } from "./ProjectPanel";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const api = vi.hoisted(() => {
  const project = { path: "/p/orders", manifest: { format: 1, name: "Orders", services: [{ id: "orders", name: "Orders", path: "../orders", contracts: [] }], environments: [{ id: "test", kind: "test", http: "https://t" }] }, problems: [], readOnly: false, missingServices: ["orders"] };
  return {
    projectOpen: vi.fn(async () => project),
    projectView: vi.fn(async () => project),
    projectSave: vi.fn(async (_p: string, manifest: unknown) => ({ ...project, manifest })),
    projectCreate: vi.fn(async (path: string, name: string) => ({ ...project, path, manifest: { ...project.manifest, name, services: [] } })),
    openPath: vi.fn(async () => undefined),
    serviceScan: vi.fn(async () => []),
  };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => "/p/created") }));

describe("ProjectPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorkspace.setState({ tabs: [], activeId: null });
  });

  it("edits the manifest and saves it", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    expect(await screen.findByLabelText("Имя проекта")).toHaveValue("Orders");
    expect(screen.getByText(/Папка сервиса не найдена/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Добавить стенд" }));
    const ids = screen.getAllByLabelText("ID стенда");
    await userEvent.type(ids[ids.length - 1], "stage");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.projectSave).toHaveBeenCalledWith("/p/orders", expect.objectContaining({ environments: [expect.objectContaining({ id: "test" }), expect.objectContaining({ id: "stage", kind: "dev" })] }));
    // Данные — из запроса только для чтения; открытие вкладки один раз отмечает проект в недавних.
    expect(api.projectView).toHaveBeenCalledWith("/p/orders");
    expect(api.projectOpen).toHaveBeenCalledTimes(1);
    expect(api.projectOpen).toHaveBeenCalledWith("/p/orders");
  });

  it("shows the environment kind badge, red for prod", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    const kind = await screen.findByLabelText("Вид");
    const badge = screen.getByTestId("env-badge");
    expect(badge).toHaveTextContent("TEST");
    await userEvent.selectOptions(kind, "prod");
    expect(badge).toHaveTextContent("PROD");
    expect(badge).toHaveStyle({ backgroundColor: "rgb(220, 38, 38)" });
  });

  it("keeps the reports of scan-all and shows why a service failed", async () => {
    const report = (service: string, notice: unknown) => ({ service, git: { version: "v1", commit: "", available: true, error: null }, contracts: 1, operationsFound: 1, tablesFound: 0, summary: { added: [], changed: [], removed: [], restored: [], unchanged: 1 }, warnings: [], errors: notice ? [{ path: "", line: null, message: "x" }] : [], scanFile: "", cancelled: false, notice });
    api.serviceScan.mockResolvedValueOnce([report("orders", null), report("billing", { key: "errors.scan.nothingParsed", params: {}, text: "No contract could be parsed; registry left unchanged" })] as never);
    renderWithClient(<ProjectPanel path="/p/orders" />);
    await userEvent.click(await screen.findByRole("button", { name: "Разобрать все сервисы" }));
    expect(await screen.findByText(/Ни один контракт не разобран/)).toBeInTheDocument();
    expect(screen.getByText(/^orders \(v1\)/)).toBeInTheDocument();
  });

  it("creates a new project and switches the tab", async () => {
    useWorkspace.getState().open({ kind: "project", path: null });
    renderWithClient(<ProjectPanel path={null} />);
    await userEvent.type(screen.getByLabelText("Имя проекта"), "New one");
    await userEvent.click(screen.getByRole("button", { name: "Выбрать папку…" }));
    await userEvent.click(await screen.findByRole("button", { name: "Создать проект" }));
    expect(api.projectCreate).toHaveBeenCalledWith("/p/created", "New one");
    expect(useWorkspace.getState().tabs.map((t) => t.id)).toEqual(["project:/p/created"]);
  });

  it("validates service and environment ids before saving", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    const id = await screen.findByLabelText("ID сервиса");
    await userEvent.type(id, " X");
    expect(id).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("Плохой ID сервиса «orders X» (a–z, 0–9, дефис)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
    await userEvent.clear(id);
    await userEvent.type(id, "orders-2");
    expect(id).not.toHaveAttribute("aria-invalid");
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Добавить стенд" }));
    const envIds = screen.getAllByLabelText("ID стенда");
    await userEvent.type(envIds[1], "test");
    expect(envIds[1]).toHaveAttribute("aria-invalid", "true");
    expect(envIds[0]).not.toHaveAttribute("aria-invalid");
    expect(screen.getByText("Повтор ID стенда «test»")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
    expect(api.projectSave).not.toHaveBeenCalled();
  });

  it("lists manifest problems returned by the backend", async () => {
    api.projectSave.mockRejectedValueOnce({ code: "INVALID_INPUT", message: "The manifest has 1 problem(s)", retryable: false, details: JSON.stringify([{ kind: "empty_service_path", id: "orders" }]) });
    renderWithClient(<ProjectPanel path="/p/orders" />);
    await userEvent.type(await screen.findByLabelText("Имя проекта"), "!");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(await screen.findByText("У сервиса «orders» нет папки")).toBeInTheDocument();
  });
  it("edits the schema registry and the agents of an environment and saves them", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    await screen.findByLabelText("Имя проекта");
    await userEvent.type(screen.getByLabelText("Адрес Schema Registry"), "https://sr.test.local");
    await userEvent.click(screen.getByRole("button", { name: "Добавить агента" }));
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
    expect(screen.getByText("Плохой ID агента «» стенда test (a–z, 0–9, дефис)")).toBeInTheDocument();
    expect(screen.getByText("У агента «» стенда test нужен адрес http:// или https://")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("ID агента"), "gen-1");
    await userEvent.type(screen.getByLabelText("Адрес агента"), "http://10.0.1.11:7420");
    // Новый агент ещё не в сохранённом манифесте: токен задаётся после сохранения.
    expect(screen.getByLabelText("Токен агента gen-1")).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.projectSave).toHaveBeenCalledWith(
      "/p/orders",
      expect.objectContaining({
        environments: [expect.objectContaining({ id: "test", schema_registry: "https://sr.test.local", agents: [{ id: "gen-1", url: "http://10.0.1.11:7420", weight: 1 }] })],
      }),
    );
    expect(await screen.findByLabelText("Токен агента gen-1")).toBeEnabled();
  });

  it("does not write an empty agent list into the manifest", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    await screen.findByLabelText("Имя проекта");
    await userEvent.click(screen.getByRole("button", { name: "Добавить агента" }));
    await userEvent.click(screen.getByRole("button", { name: "Удалить агента" }));
    // Черновик снова равен сохранённому манифесту — сохранять нечего.
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
  });
});
