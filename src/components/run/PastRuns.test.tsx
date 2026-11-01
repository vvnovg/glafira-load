import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { RunListEntry } from "../../api/types";
import { renderWithClient } from "../../test/render";
import { PastRuns } from "./PastRuns";

/** Строка `run_list`: `dir` приходит с бэкенда (`runs/<папка>`), как и `run_dir_name` его называет. */
const entry = (started_at: string, mode: "run" | "smoke", status: RunListEntry["status"]): RunListEntry => ({
  dir: `runs/${mode === "smoke" ? "smoke-" : ""}${started_at.slice(0, 19).replace(/:/g, "-")}`,
  run_id: started_at,
  suite: "orders",
  profile: "nominal",
  env: "test",
  mode,
  agents: [],
  started_at,
  ...(status ? { ended_at: started_at, status } : {}),
});
const runs = [entry("2026-11-02T14:30:00Z", "run", "completed"), entry("2026-11-01T09:00:00Z", "smoke", "failed"), entry("2026-10-31T08:00:00Z", "run", undefined)];

const api = vi.hoisted(() => ({
  runList: vi.fn(),
  runGet: vi.fn(async (_p: string, dir: string) =>
    dir.includes("smoke")
      ? { dir, meta: {}, summary: null, smoke: { scenarios: [{ id: "order-flow", steps: [{ id: "create", operation: "http:orders:POST /v1/orders", ok: true }, { id: "get", operation: "http:orders:GET /v1/orders/{id}", ok: false, code: "EXPECT_STATUS", message: "expected 200, got 404" }] }] } }
      : dir.includes("10-31")
        ? { dir, meta: {}, summary: null, smoke: null }
        : { dir, meta: {}, summary: { status: "completed", ops: [{ id: "http:a", n: 10, rpsAvg: 1, rpsPeak: 2, err: {}, errPct: 0, dropped: 0, p50: 1, p90: 2, p95: 3, p99: 4, max: 5, target: null, achievedPct: null }], scenarios: [], seconds: 60 }, smoke: null },
  ),
  openPath: vi.fn(async () => undefined),
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));

describe("PastRuns", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.runList.mockResolvedValue(runs);
  });

  it("lists the runs with date, suite, profile and status, and shows the summary of the chosen one", async () => {
    renderWithClient(<PastRuns path="/p" />);
    expect(await screen.findByText("2026-11-02 14:30")).toBeInTheDocument();
    expect(screen.getByText("не завершён")).toBeInTheDocument();
    expect(screen.getAllByText("orders")).toHaveLength(3);
    expect(screen.getByText("смок")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "2026-11-02 14:30" }));
    expect(api.runGet).toHaveBeenCalledWith("/p", "runs/2026-11-02T14-30-00");
    expect(await screen.findByText("Итог")).toBeInTheDocument();
    expect(screen.getByText("http:a")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "2026-11-02 14:30" })).toHaveAttribute("aria-current", "true");
    await userEvent.click(screen.getByRole("button", { name: "Открыть папку" }));
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/runs/2026-11-02T14-30-00");
  });

  it("shows the smoke report of a smoke run", async () => {
    renderWithClient(<PastRuns path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "2026-11-01 09:00" }));
    expect(api.runGet).toHaveBeenCalledWith("/p", "runs/smoke-2026-11-01T09-00-00");
    expect(await screen.findByText("Результат смока")).toBeInTheDocument();
    expect(screen.getByText("ок")).toBeInTheDocument();
    expect(screen.getByText("EXPECT_STATUS: expected 200, got 404")).toBeInTheDocument();
  });

  it("says when the chosen run has no summary yet", async () => {
    renderWithClient(<PastRuns path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "2026-10-31 08:00" }));
    expect(await screen.findByText("Итога нет: у прогона нет summary.yaml")).toBeInTheDocument();
  });

  it("says when there are no runs", async () => {
    api.runList.mockResolvedValueOnce([]);
    renderWithClient(<PastRuns path="/p" />);
    expect(await screen.findByText("Прогонов пока нет")).toBeInTheDocument();
  });
});
