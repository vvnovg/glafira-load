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
    // Раскрытие узла только читает манифест: список недавних не переупорядочивается.
    expect(api.projectView).toHaveBeenCalledWith("/p/orders");
    expect(api.projectOpen).not.toHaveBeenCalled();
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
