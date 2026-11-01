import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { useWorkspace } from "../../state/workspace";
import { ProjectTree } from "./ProjectTree";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const api = vi.hoisted(() => ({
  projectRecent: vi.fn(async () => [
    { path: "/p/orders", name: "Orders platform", missing: false },
    { path: "/p/gone", name: null, missing: true },
  ]),
  projectOpen: vi.fn(async (path: string) => ({ path, manifest: { format: 1, name: "Orders platform", services: [{ id: "orders", name: "Orders", path: "../orders", contracts: [] }], environments: [] }, problems: [], readOnly: false, missingServices: [] })),
  projectView: vi.fn(async (path: string) => ({ path, manifest: { format: 1, name: "Orders platform", services: [{ id: "orders", name: "Orders", path: "../orders", contracts: [] }], environments: [] }, problems: [], readOnly: false, missingServices: [] })),
  projectClose: vi.fn(async () => undefined),
  suiteList: vi.fn(async (_path: string) => [{ name: "orders-flow", file: "suites/orders-flow/suite.yaml", scenarios: 2 }]),
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => "/p/new") }));

describe("ProjectTree", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorkspace.setState({ tabs: [], activeId: null });
  });

  it("lists recent projects, expands sections and opens tabs", async () => {
    renderWithClient(<ProjectTree />);
    const orders = await screen.findByRole("treeitem", { name: /Orders platform/ });
    await userEvent.click(within(orders).getByRole("button", { name: "Orders platform" }));
    expect(useWorkspace.getState().tabs.map((t) => t.id)).toEqual(["project:/p/orders"]);
    await userEvent.click(await screen.findByRole("button", { name: "Реестр операций" }));
    expect(useWorkspace.getState().activeId).toBe("registry:/p/orders");
    await userEvent.click(screen.getByRole("button", { name: "Orders" }));
    expect(useWorkspace.getState().activeId).toBe("service:/p/orders:orders");
    expect(screen.getByText("Папка не найдена")).toBeInTheDocument();
    expect(orders).toHaveAttribute("aria-expanded", "true");
    const gone = screen.getByRole("treeitem", { name: /gone/ });
    expect(gone).not.toHaveAttribute("aria-expanded");
    expect(gone).toHaveClass("muted");
    expect(gone).not.toHaveClass("tree-error");
    // Раскрытие узла только читает манифест: список недавних не переупорядочивается.
    expect(api.projectView).toHaveBeenCalledWith("/p/orders");
    expect(api.projectOpen).not.toHaveBeenCalled();
  });

  it("lists suites under the project and opens the suite and run tabs", async () => {
    renderWithClient(<ProjectTree />);
    await screen.findByRole("treeitem", { name: /Orders platform/ });
    const suites = await screen.findByRole("treeitem", { name: "Сьюты" });
    await userEvent.click(await within(suites).findByRole("button", { name: "orders-flow" }));
    expect(api.suiteList).toHaveBeenCalledWith("/p/orders");
    // Пропавший проект не раскрывается — его сьюты не запрашиваются.
    expect(api.suiteList).toHaveBeenCalledTimes(1);
    expect(useWorkspace.getState().activeId).toBe("suite:/p/orders:orders-flow");
    await userEvent.click(screen.getByRole("button", { name: "Прогон" }));
    expect(useWorkspace.getState().activeId).toBe("run:/p/orders");
  });

  it("says when the project has no suites yet", async () => {
    api.suiteList.mockResolvedValueOnce([]);
    renderWithClient(<ProjectTree />);
    expect(await screen.findByText("сьютов пока нет: создайте из реестра")).toBeInTheDocument();
  });

  it("removes a missing project from the list and closes its tabs", async () => {
    renderWithClient(<ProjectTree />);
    useWorkspace.getState().open({ kind: "registry", path: "/p/gone" });
    const gone = await screen.findByRole("treeitem", { name: /gone/ });
    await userEvent.click(within(gone).getByRole("button", { name: "Убрать из списка" }));
    expect(api.projectClose).toHaveBeenCalledWith("/p/gone");
    expect(useWorkspace.getState().tabs).toEqual([]);
  });

  it("opens a project folder through the dialog", async () => {
    renderWithClient(<ProjectTree />);
    await userEvent.click(await screen.findByRole("button", { name: "Открыть проект…" }));
    expect(api.projectOpen).toHaveBeenCalledWith("/p/new");
    expect(useWorkspace.getState().activeId).toBe("project:/p/new");
  });
});
