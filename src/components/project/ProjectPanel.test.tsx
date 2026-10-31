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
    projectSave: vi.fn(async (_p: string, manifest: unknown) => ({ ...project, manifest })),
    projectCreate: vi.fn(async (path: string, name: string) => ({ ...project, path, manifest: { ...project.manifest, name, services: [] } })),
    openPath: vi.fn(async () => undefined),
    serviceScan: vi.fn(async () => []),
  };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => "/p/created") }));

describe("ProjectPanel", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("edits the manifest and saves it", async () => {
    renderWithClient(<ProjectPanel path="/p/orders" />);
    expect(await screen.findByLabelText("Имя проекта")).toHaveValue("Orders");
    expect(screen.getByText(/Папка сервиса не найдена/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Добавить стенд" }));
    const ids = screen.getAllByLabelText("ID стенда");
    await userEvent.type(ids[ids.length - 1], "stage");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.projectSave).toHaveBeenCalledWith("/p/orders", expect.objectContaining({ environments: [expect.objectContaining({ id: "test" }), expect.objectContaining({ id: "stage", kind: "dev" })] }));
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
});
