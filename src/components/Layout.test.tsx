import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderWithClient } from "../test/render";
import { useWorkspace } from "../state/workspace";
import { Layout, projectLabel } from "./Layout";

vi.mock("../api/client", () => ({
  api: {
    getAppInfo: vi.fn(async () => ({ name: "Glafira", version: "0.1.0", commit: "x", os: "macos", arch: "aarch64", dataDir: "/d", logDir: "/l" })),
    getLanguageSettings: vi.fn(async () => ({ effective: "ru", chosen: null, policy: null, systemMatch: null, reason: "default", languages: ["en", "ru"] })),
    getPolicyStatus: vi.fn(async () => ({ managed: false, source: null, updatesEnabled: false, updateUrl: null, language: null, errors: [] })),
    checkUpdate: vi.fn(async () => ({ status: "disabled", reason: { key: "errors.updates.notConfigured", params: {}, text: "x" } })),
  },
  toAppError: (e: unknown) => e,
  isAppError: () => false,
}));

describe("Layout", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("opens the registry tab with the project folder name and closes it", async () => {
    renderWithClient(<Layout />);
    act(() => useWorkspace.getState().open({ kind: "registry", path: "/home/u/orders-load" }));
    expect(await screen.findByRole("tab", { name: "orders-load · Реестр" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Закрыть вкладку" }));
    expect(screen.queryByRole("tab")).toBeNull();
    expect(projectLabel("C:\\work\\p\\")).toBe("p");
  });
});
