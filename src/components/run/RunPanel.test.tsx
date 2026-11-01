import { act, screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import type { OpTick } from "../../api/types";
import { emptyRun, type StartParams, useRunStore } from "../../state/run";
import { renderWithClient } from "../../test/render";
import { RunPanel } from "./RunPanel";

const api = vi.hoisted(() => ({
  projectView: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "P", services: [], environments: [{ id: "test", kind: "test", agents: [] }] }, problems: [], readOnly: false, missingServices: [] })),
  suiteList: vi.fn(async () => [{ name: "orders", file: "suites/orders/suite.yaml", scenarios: 1 }]),
  profileGet: vi.fn(async () => ({ format: 1, coefficients: { k_env: 1, k_growth: 1, overrides: {} }, profiles: { nominal: { basis: "avg", phases: [] } } })),
  profileCompute: vi.fn(async () => ({ profile: "nominal", basis: "avg", targets: [], total: 100, uncovered: [], timeline: [{ minute: 0, pct: 100 }, { minute: 10, pct: 100 }], total_minutes: 10 })),
  suiteCheck: vi.fn(async () => ({ findings: [], coverage: null, profile: "nominal" })),
  agentProbe: vi.fn(async () => []),
  runList: vi.fn(async () => []),
  runGet: vi.fn(async () => ({ meta: {}, summary: null, smoke: { scenarios: [{ id: "order-flow", steps: [{ id: "create", operation: "http:a", ok: true }] }] } })),
  openPath: vi.fn(async () => undefined),
  runStart: vi.fn(),
  runStop: vi.fn(async () => undefined),
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("uplot", () => ({ default: class { setData() {} setSize() {} destroy() {} } }));
vi.mock("uplot/dist/uPlot.min.css", () => ({}));
vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });

const params = (mode: "run" | "smoke"): StartParams => ({ suite: "orders", profile: "nominal", env: "test", mode, maxInFlight: null, timeline: [{ minute: 0, pct: 100 }, { minute: 10, pct: 100 }], total: 100 });
const op = (id: string, n: number, err: Record<string, number> = {}): OpTick => ({ id, n, err, dropped: 0, p50: 1, p90: 2, p95: 3, p99: 4, max: 5, bytesIn: 0, bytesOut: 0, lag: null });
const summary = { status: "completed" as const, ops: [{ id: "http:a", n: 30, rpsAvg: 30, rpsPeak: 30, err: {}, errPct: 0, dropped: 0, p50: 1, p90: 2, p95: 3, p99: 4, max: 5, target: 100, achievedPct: 30 }], scenarios: [], seconds: 1 };

describe("RunPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useRunStore.setState({ byProject: {}, closeRequested: false });
  });

  it("shows live counters, charts and the operations table from store events, then the summary", async () => {
    renderWithClient(<RunPanel path="/p" />);
    await screen.findByRole("button", { name: "Запустить" });
    expect(screen.queryByTestId("rps-chart")).toBeNull();
    act(() => {
      useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "starting", params: params("run") } } });
      const { onEvent } = useRunStore.getState();
      onEvent("/p", { type: "started", run_id: "r1", dir: "/p/runs/x" });
      onEvent("/p", { type: "tick", t: 60, ops: [op("http:a", 30, { HTTP_5xx: 2 })], scenarios: [] });
      onEvent("/p", { type: "phase", t: 60, index: 0, pct: 100 });
      onEvent("/p", { type: "error", t: 60, code: "HTTP_5xx", operation: "http:a", scenario: "s", step: "create", agent: "local", message: "500 Internal", request: null, response: null });
    });
    expect(screen.getByText("Фаза 1: 100%")).toBeInTheDocument();
    expect(screen.getByText("1,0 из 10 мин")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Ход таймлайна" })).toHaveAttribute("value", "60");
    expect(screen.getByText("Запросов: 30")).toBeInTheDocument();
    expect(screen.getByText("Ошибок: 2")).toHaveClass("warn");
    expect(screen.getByTestId("rps-chart")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Сортировать по за секунду" })).toBeInTheDocument();
    expect(screen.getAllByText("http:a").length).toBeGreaterThan(0);
    expect(screen.getByText("Последние ошибки")).toBeInTheDocument();
    expect(screen.getByText(/500 Internal/)).toBeInTheDocument();
    expect(api.runList).toHaveBeenCalledTimes(1);
    act(() => useRunStore.getState().onEvent("/p", { type: "done", run_id: "r1", dir: "/p/runs/x", status: "completed", summary }));
    expect(await screen.findByText("Итог")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Открыть папку" })).toBeInTheDocument();
    // Завершённый прогон появляется в списке прошлых.
    await waitFor(() => expect(api.runList).toHaveBeenCalledTimes(2));
  });

  it("shows the smoke report instead of charts after a smoke run", async () => {
    renderWithClient(<RunPanel path="/p" />);
    await screen.findByRole("button", { name: "Запустить" });
    act(() => {
      useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "done", params: params("smoke"), dir: "/p/runs/smoke-x", result: { run_id: "s1", dir: "/p/runs/smoke-x", status: "failed", summary } } } });
    });
    expect(await screen.findByText("Результат смока")).toBeInTheDocument();
    expect(api.runGet).toHaveBeenCalledWith("/p", "/p/runs/smoke-x");
    expect(screen.queryByTestId("rps-chart")).toBeNull();
    expect(screen.queryByText("Итог")).toBeNull();
  });
});
