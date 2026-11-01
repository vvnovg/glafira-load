import { vi } from "vitest";
import type { AgentSummary, OpTick, Tick } from "../api/types";
import { applyTick, emptyRun, pctAt, type ProjectRun, RING, type StartParams, useRunStore } from "./run";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const api = vi.hoisted(() => ({ runStart: vi.fn(), runStop: vi.fn(async () => undefined) }));
vi.mock("../api/client", () => ({ api, toAppError: (e: unknown) => e }));

const timeline = [
  { minute: 0, pct: 0 },
  { minute: 10, pct: 100 },
  { minute: 70, pct: 100 },
];
const params: StartParams = { suite: "orders", profile: "nominal", env: "test", mode: "run", maxInFlight: null, timeline, total: 200 };
const op = (id: string, n: number, p95: number, err: Record<string, number> = {}, dropped = 0): OpTick => ({ id, n, err, dropped, p50: 1, p90: 2, p95, p99: 4, max: 5, bytesIn: 0, bytesOut: 0, lag: null });
const tick = (t: number, ops: OpTick[]): Tick => ({ t, ops, scenarios: [] });
const summary: AgentSummary = { status: "completed", ops: [], scenarios: [], seconds: 600 };

describe("pctAt", () => {
  it("interpolates the timeline linearly by minutes and clamps at both ends", () => {
    expect(pctAt(timeline, 0)).toBe(0);
    expect(pctAt(timeline, 300)).toBe(50);
    expect(pctAt(timeline, 600)).toBe(100);
    expect(pctAt(timeline, 5000)).toBe(100);
    expect(pctAt([], 10)).toBe(0);
  });
});

describe("run store", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useRunStore.setState({ byProject: {}, closeRequested: false });
  });

  it("reduces started, tick, phase, agent, error and done events", () => {
    useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "starting", params } } });
    const { onEvent } = useRunStore.getState();
    onEvent("/p", { type: "started", run_id: "r1", dir: "/p/runs/2026-11-02T14-30-00" });
    // Гистограмма секунды (`hist`) окну не нужна и в счётчики не попадает.
    onEvent("/p", { type: "tick", ...tick(300, [{ ...op("a", 10, 5, { HTTP_5xx: 2 }, 1), hist: "HISTFAAAA" }, op("b", 20, 7)]) });
    let r = useRunStore.getState().byProject["/p"];
    expect(r.status).toBe("running");
    expect(r.runId).toBe("r1");
    expect(r.series.t).toEqual([300]);
    expect(r.series.rps).toEqual([30]);
    expect(r.series.rpsTarget).toEqual([100]);
    expect(r.series.p95ByOp).toEqual({ a: [5], b: [7] });
    expect([r.requests, r.errors, r.dropped, r.version]).toEqual([30, 2, 1, 1]);
    // Операция, пропавшая из тика, получает null; появившаяся позже — null за прошлые секунды.
    onEvent("/p", { type: "tick", ...tick(301, [op("a", 5, 6), op("c", 1, 9)]) });
    r = useRunStore.getState().byProject["/p"];
    expect(r.series.p95ByOp).toEqual({ a: [5, 6], b: [7, null], c: [null, 9] });
    expect(r.requests).toBe(36);
    onEvent("/p", { type: "phase", t: 301, index: 1, pct: 100 });
    onEvent("/p", { type: "agent", id: "gen-1", state: "lost" });
    onEvent("/p", { type: "error", t: 301, code: "HTTP_5xx", operation: "a", scenario: "s", step: "create", agent: "gen-1", message: "500", request: null, response: null });
    onEvent("/p", { type: "done", run_id: "r1", dir: "/p/runs/2026-11-02T14-30-00", status: "incomplete", summary });
    r = useRunStore.getState().byProject["/p"];
    expect(r.phase).toEqual({ t: 301, index: 1, pct: 100 });
    expect(r.agents).toEqual({ "gen-1": "lost" });
    expect(r.samples.map((s) => s.code)).toEqual(["HTTP_5xx"]);
    expect(r.status).toBe("done");
    expect(r.result?.status).toBe("incomplete");
    expect(r.lastTick?.t).toBe(301);
  });

  it("keeps the stopping state when started arrives after Stop", () => {
    useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "stopping", params } } });
    useRunStore.getState().onEvent("/p", { type: "started", run_id: "r1", dir: "/p/runs/x" });
    const r = useRunStore.getState().byProject["/p"];
    expect([r.status, r.runId]).toEqual(["stopping", "r1"]);
  });

  it("keeps only the last RING seconds in the series", () => {
    let run: ProjectRun = { ...emptyRun(), status: "running", params };
    for (let t = 1; t <= RING + 100; t++) run = applyTick(run, tick(t, [op("a", 1, 1)]));
    expect(run.series.t).toHaveLength(RING);
    expect(run.series.t[0]).toBe(101);
    expect(run.series.p95ByOp.a).toHaveLength(RING);
    expect(run.requests).toBe(RING + 100);
  });

  it("start subscribes to the channel and settles on the result", async () => {
    api.runStart.mockImplementation(async (_p: string, _s: string, _pr: string, _e: string, _m: string, _mx: number | null, onEvent: (e: unknown) => void) => {
      onEvent({ type: "started", run_id: "r2", dir: "/p/runs/x" });
      onEvent({ type: "tick", ...tick(1, [op("a", 3, 1)]) });
      return { run_id: "r2", dir: "/p/runs/x", status: "completed", summary };
    });
    await useRunStore.getState().start("/p", params);
    const r = useRunStore.getState().byProject["/p"];
    expect(api.runStart).toHaveBeenCalledWith("/p", "orders", "nominal", "test", "run", null, expect.any(Function));
    expect([r.status, r.runId, r.dir, r.requests]).toEqual(["done", "r2", "/p/runs/x", 3]);
    expect(r.result?.status).toBe("completed");
  });

  it("drops late events of a previous run of the project", async () => {
    let late: (e: unknown) => void = () => undefined;
    api.runStart.mockImplementationOnce(async (...args: unknown[]) => {
      late = args[6] as (e: unknown) => void;
      return { run_id: "r1", dir: "/p/runs/a", status: "completed", summary };
    });
    await useRunStore.getState().start("/p", params);
    let finish: (v: unknown) => void = () => undefined;
    api.runStart.mockImplementationOnce(() => new Promise((r) => (finish = r)));
    const second = useRunStore.getState().start("/p", params);
    late({ type: "tick", ...tick(5, [op("a", 100, 1)]) });
    expect(useRunStore.getState().byProject["/p"].requests).toBe(0);
    finish({ run_id: "r2", dir: "/p/runs/b", status: "completed", summary });
    await second;
  });

  it("start failure keeps the error; stop marks the run as stopping", async () => {
    api.runStart.mockRejectedValueOnce({ code: "RUN_BUSY", message: "A run is already in progress in this project", retryable: false });
    await useRunStore.getState().start("/p", params);
    expect(useRunStore.getState().byProject["/p"].status).toBe("failed");
    expect(useRunStore.getState().byProject["/p"].error?.code).toBe("RUN_BUSY");
    let finish: (v: unknown) => void = () => undefined;
    api.runStart.mockImplementationOnce(() => new Promise((r) => (finish = r)));
    const started = useRunStore.getState().start("/p", params);
    expect(useRunStore.getState().byProject["/p"].status).toBe("starting");
    await useRunStore.getState().stop("/p");
    expect(api.runStop).toHaveBeenCalledWith("/p");
    expect(useRunStore.getState().byProject["/p"].status).toBe("stopping");
    finish({ run_id: "r3", dir: "/p/runs/y", status: "stopped", summary });
    await started;
    expect(useRunStore.getState().byProject["/p"].status).toBe("done");
    // Второй start во время идущего прогона — ничего не делает.
    useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "running", params } } });
    await useRunStore.getState().start("/p", params);
    expect(api.runStart).toHaveBeenCalledTimes(2);
  });

  it("returns to running when the stop request fails", async () => {
    useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "running", params } } });
    api.runStop.mockRejectedValueOnce({ code: "INTERNAL", message: "boom", retryable: false });
    await expect(useRunStore.getState().stop("/p")).rejects.toMatchObject({ code: "INTERNAL" });
    expect(useRunStore.getState().byProject["/p"].status).toBe("running");
  });
});
