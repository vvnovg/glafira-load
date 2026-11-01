import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { CheckReport, Environment } from "../../api/types";
import { emptyRun, useRunStore } from "../../state/run";
import { renderWithClient } from "../../test/render";
import { RunForm } from "./RunForm";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const { api, envs } = vi.hoisted(() => {
  const envs: { list: Environment[] } = { list: [] };
  return {
    envs,
    api: {
      projectView: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "P", services: [], environments: envs.list }, problems: [], readOnly: false, missingServices: [] })),
      suiteList: vi.fn(async () => [{ name: "orders", file: "suites/orders/suite.yaml", scenarios: 1 }]),
      profileGet: vi.fn(async () => ({ format: 1, coefficients: { k_env: 1, k_growth: 1, overrides: {} }, profiles: { nominal: { basis: "avg", phases: [{ kind: "steady", minutes: 10 }] } } })),
      profileCompute: vi.fn(async () => ({ profile: "nominal", basis: "avg", targets: [], total: 100, uncovered: [], timeline: [{ minute: 0, pct: 100 }, { minute: 10, pct: 100 }], total_minutes: 10 })),
      suiteCheck: vi.fn(async (): Promise<CheckReport> => ({ findings: [], coverage: null, profile: "nominal" })),
      agentProbe: vi.fn(async () => [{ id: "gen-1", url: "http://10.0.1.11:7420", state: "ready", info: { version: "0.2.0", os: "linux", arch: "x86_64", cpus: 4, busy: null }, message: null }]),
      openPath: vi.fn(async () => undefined),
      runStart: vi.fn(),
      runStop: vi.fn(async () => undefined),
    },
  };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));

const withAgent: Environment = { id: "test", kind: "test", http: "https://t", agents: [{ id: "gen-1", url: "http://10.0.1.11:7420", weight: 1 }] };
const result = { run_id: "r1", dir: "/p/runs/2026-11-02T14-30-00", status: "completed", summary: { status: "completed", ops: [], scenarios: [], seconds: 600 } };

describe("RunForm", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    envs.list = [withAgent];
    useRunStore.setState({ byProject: {}, closeRequested: false });
    api.runStart.mockResolvedValue(result);
  });

  it("checks the suite and starts the run with the chosen parameters", async () => {
    renderWithClient(<RunForm path="/p" />);
    const start = screen.getByRole("button", { name: "Запустить" });
    await waitFor(() => expect(start).toBeEnabled());
    expect(screen.getByLabelText("Сьют")).toHaveValue("orders");
    expect(screen.getByLabelText("Профиль")).toHaveValue("nominal");
    expect(screen.getByLabelText("Стенд")).toHaveValue("test");
    // Агенты стенда опрашиваются при открытии.
    expect(await screen.findByTestId("agent-state")).toHaveTextContent("готов");
    expect(api.agentProbe).toHaveBeenCalledWith("/p", "test");
    await userEvent.type(screen.getByLabelText("Потолок одновременности"), "500");
    await userEvent.click(start);
    expect(api.suiteCheck).toHaveBeenCalledWith("/p", "orders");
    await waitFor(() => expect(api.runStart).toHaveBeenCalledWith("/p", "orders", "nominal", "test", "run", 500, expect.any(Function)));
    expect(await screen.findByText("завершён")).toBeInTheDocument();
    expect(useRunStore.getState().byProject["/p"].params).toMatchObject({ total: 100, timeline: [{ minute: 0, pct: 100 }, { minute: 10, pct: 100 }] });
  });

  it("shows the live agent state from the run events", async () => {
    renderWithClient(<RunForm path="/p" />);
    await screen.findByTestId("agent-state");
    act(() => useRunStore.getState().onEvent("/p", { type: "agent", id: "gen-1", state: "lost" }));
    expect(await screen.findByTestId("agent-live")).toHaveTextContent("потерян");
    expect(screen.queryByTestId("agent-state")).toBeNull();
  });

  it("blocks a bad concurrency cap", async () => {
    renderWithClient(<RunForm path="/p" />);
    const start = screen.getByRole("button", { name: "Запустить" });
    await waitFor(() => expect(start).toBeEnabled());
    await userEvent.type(screen.getByLabelText("Потолок одновременности"), "0");
    expect(start).toBeDisabled();
    expect(screen.getByLabelText("Потолок одновременности")).toHaveAttribute("aria-invalid", "true");
  });

  it("says that a stand without agents runs on the built-in agent and smoke does not need agents", async () => {
    envs.list = [{ id: "test", kind: "test" }];
    renderWithClient(<RunForm path="/p" />);
    const start = screen.getByRole("button", { name: "Запустить" });
    await waitFor(() => expect(start).toBeEnabled());
    expect(screen.getByText("У стенда нет агентов: прогон пойдёт на агенте, встроенном в приложение")).toBeInTheDocument();
    expect(api.agentProbe).not.toHaveBeenCalled();
    await userEvent.selectOptions(screen.getByLabelText("Режим"), "smoke");
    expect(screen.getByText("Смок идёт внутри приложения: агенты стенда не используются")).toBeInTheDocument();
    await userEvent.click(start);
    await waitFor(() => expect(api.runStart).toHaveBeenCalledWith("/p", "orders", "nominal", "test", "smoke", null, expect.any(Function)));
  });

  it("blocks Start while the check reports errors and unblocks after a clean recheck", async () => {
    api.suiteCheck.mockResolvedValueOnce({ findings: [{ level: "error", file: "suites/orders/suite.yaml", line: 8, message: "operation x is not in the registry" }], coverage: null, profile: "nominal" });
    renderWithClient(<RunForm path="/p" />);
    const start = screen.getByRole("button", { name: "Запустить" });
    await waitFor(() => expect(start).toBeEnabled());
    await userEvent.click(start);
    expect(await screen.findByText("В сьюте ошибки: исправьте их, чтобы запустить")).toBeInTheDocument();
    expect(screen.getByText("operation x is not in the registry")).toBeInTheDocument();
    expect(start).toBeDisabled();
    expect(api.runStart).not.toHaveBeenCalled();
    // Строка находки открывает файл сьюта.
    await userEvent.click(screen.getByRole("button", { name: "suites/orders/suite.yaml:8" }));
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/suites/orders/suite.yaml");
    await userEvent.click(screen.getByRole("button", { name: "Проверить снова" }));
    await waitFor(() => expect(start).toBeEnabled());
    expect(screen.queryByText("В сьюте ошибки: исправьте их, чтобы запустить")).toBeNull();
  });

  it("shows RUN_BUSY, keeps Start disabled and lets Stop reset the state", async () => {
    api.runStart.mockRejectedValueOnce({ code: "RUN_BUSY", message: "A run is already in progress in this project", retryable: false });
    renderWithClient(<RunForm path="/p" />);
    const start = screen.getByRole("button", { name: "Запустить" });
    await waitFor(() => expect(start).toBeEnabled());
    await userEvent.click(start);
    expect(await screen.findByRole("status")).toHaveTextContent("В проекте уже идёт прогон");
    expect(start).toBeDisabled();
    const stop = screen.getByRole("button", { name: "Остановить" });
    expect(stop).toBeEnabled();
    await userEvent.click(stop);
    expect(api.runStop).toHaveBeenCalledWith("/p");
    await waitFor(() => expect(start).toBeEnabled());
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("disables the fields while the run is active and stops it", async () => {
    let finish: (v: unknown) => void = () => undefined;
    api.runStart.mockImplementationOnce(() => new Promise((r) => (finish = r)));
    renderWithClient(<RunForm path="/p" />);
    const start = screen.getByRole("button", { name: "Запустить" });
    await waitFor(() => expect(start).toBeEnabled());
    await userEvent.click(start);
    await waitFor(() => expect(screen.getByLabelText("Сьют")).toBeDisabled());
    expect(start).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Остановить" }));
    expect(api.runStop).toHaveBeenCalledWith("/p");
    expect(await screen.findByText("остановка…")).toBeInTheDocument();
    finish({ ...result, status: "stopped" });
    expect(await screen.findByText("завершён")).toBeInTheDocument();
    expect(screen.getByLabelText("Сьют")).toBeEnabled();
  });

  it("restores the parameters of the active run when the tab is reopened", async () => {
    const params = { suite: "orders", profile: "nominal", env: "test", mode: "smoke" as const, maxInFlight: 7, timeline: [], total: 0 };
    useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "running", params } } });
    renderWithClient(<RunForm path="/p" />);
    expect(screen.getByLabelText("Режим")).toHaveValue("smoke");
    expect(screen.getByLabelText("Потолок одновременности")).toHaveValue(7);
    await waitFor(() => expect(screen.getByLabelText("Сьют")).toHaveValue("orders"));
  });
});
