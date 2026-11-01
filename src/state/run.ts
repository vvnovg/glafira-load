import { create } from "zustand";
import { api, toAppError } from "../api/client";
import type { AgentRunState, AppError, ControllerEvent, ErrorSample, Mode, PhaseEvent, Point, RunResult, Tick } from "../api/types";

export type RunState = "idle" | "starting" | "running" | "stopping" | "done" | "failed";
/** Длина кольцевого буфера рядов — последний час посекундно. */
export const RING = 3600;
/** Сколько примеров ошибок держим в памяти. */
const SAMPLES = 50;

/** Ряды графиков: секунда от старта, суммарный RPS факт и цель, p95 по операциям (null — операции не было в тике). */
export interface RunSeries { t: number[]; rps: number[]; rpsTarget: number[]; p95ByOp: Record<string, (number | null)[]> }
/** Параметры запуска; `timeline` и `total` — из `profile_compute`, по ним фронтенд рисует цель RPS. */
export interface StartParams { suite: string; profile: string; env: string; mode: Mode; maxInFlight: number | null; timeline: Point[]; total: number }
export interface ProjectRun {
  status: RunState;
  params: StartParams | null;
  runId: string | null;
  /** Папка прогона: из `started`/`done` приходит абсолютный путь. */
  dir: string | null;
  /** Ряды мутируются на месте (кольцевой буфер); `version` растёт с каждым тиком — по нему перерисовываются графики. */
  series: RunSeries;
  version: number;
  lastTick: Tick | null;
  agents: Record<string, AgentRunState>;
  requests: number;
  errors: number;
  dropped: number;
  phase: PhaseEvent | null;
  samples: ErrorSample[];
  result: RunResult | null;
  error: AppError | null;
}

export const emptyRun = (): ProjectRun => ({
  status: "idle",
  params: null,
  runId: null,
  dir: null,
  series: { t: [], rps: [], rpsTarget: [], p95ByOp: {} },
  version: 0,
  lastTick: null,
  agents: {},
  requests: 0,
  errors: 0,
  dropped: 0,
  phase: null,
  samples: [],
  result: null,
  error: null,
});

/** Процент цели в секунду `second`: линейная интерполяция таймлайна по минутам (как `scheduler::pct_at`). */
export function pctAt(timeline: Point[], second: number): number {
  if (timeline.length === 0) return 0;
  const m = second / 60;
  if (m <= timeline[0].minute) return timeline[0].pct;
  for (let i = 1; i < timeline.length; i++) {
    const a = timeline[i - 1];
    const b = timeline[i];
    if (m <= b.minute) {
      const span = b.minute - a.minute;
      return span <= 0 ? b.pct : a.pct + ((b.pct - a.pct) * (m - a.minute)) / span;
    }
  }
  return timeline[timeline.length - 1].pct;
}

function push<T>(arr: T[], v: T): void {
  arr.push(v);
  if (arr.length > RING) arr.splice(0, arr.length - RING);
}

const sum = (xs: number[]): number => xs.reduce((a, b) => a + b, 0);

/** Тик без гистограмм (`hist`): они нужны только контроллеру для слияния перцентилей. */
function withoutHist(tick: Tick): Tick {
  return {
    t: tick.t,
    ops: tick.ops.map(({ hist: _hist, ...o }) => o),
    scenarios: tick.scenarios.map(({ hist: _hist, ...s }) => s),
  };
}

/** Один тик в ряды: суммарный RPS, цель по таймлайну, p95 по операциям; счётчики запросов, ошибок и dropped. */
export function applyTick(run: ProjectRun, tick: Tick): ProjectRun {
  const s = run.series;
  const rps = sum(tick.ops.map((o) => o.n));
  push(s.t, tick.t);
  push(s.rps, rps);
  push(s.rpsTarget, (pctAt(run.params?.timeline ?? [], tick.t) / 100) * (run.params?.total ?? 0));
  const seen = new Set<string>();
  for (const o of tick.ops) {
    seen.add(o.id);
    // Операция, появившаяся не с первой секунды, получает null за прошлые секунды, чтобы ряды были одной длины.
    const arr = s.p95ByOp[o.id] ?? (s.p95ByOp[o.id] = new Array<number | null>(s.t.length - 1).fill(null));
    push(arr, o.p95);
  }
  for (const [id, arr] of Object.entries(s.p95ByOp)) if (!seen.has(id)) push(arr, null);
  return {
    ...run,
    version: run.version + 1,
    lastTick: withoutHist(tick),
    requests: run.requests + rps,
    errors: run.errors + sum(tick.ops.map((o) => sum(Object.values(o.err)))),
    dropped: run.dropped + sum(tick.ops.map((o) => o.dropped)),
  };
}

/** Событие контроллера → новое состояние прогона. */
export function reduceEvent(run: ProjectRun, e: ControllerEvent): ProjectRun {
  switch (e.type) {
    case "started":
      // «Остановить», нажатая до `started`, не отменяется этим событием.
      return { ...run, status: run.status === "starting" ? "running" : run.status, runId: e.run_id, dir: e.dir };
    case "tick":
      return applyTick(run, e);
    case "phase":
      return { ...run, phase: { t: e.t, index: e.index, pct: e.pct } };
    case "agent":
      return { ...run, agents: { ...run.agents, [e.id]: e.state } };
    case "error": {
      const { type: _type, ...sample } = e;
      return { ...run, samples: [...run.samples.slice(-(SAMPLES - 1)), sample] };
    }
    case "done": {
      const { type: _type, ...result } = e;
      return { ...run, status: "done", result, runId: result.run_id, dir: result.dir };
    }
  }
}

const ACTIVE: RunState[] = ["starting", "running", "stopping"];
export const isActive = (status: RunState): boolean => ACTIVE.includes(status);

interface RunStore {
  byProject: Record<string, ProjectRun>;
  /** Окно просят закрыть во время прогона (событие Tauri `run-close-requested`); диалог — `RunCloseDialog`. */
  closeRequested: boolean;
  setCloseRequested: (v: boolean) => void;
  start: (path: string, params: StartParams) => Promise<void>;
  stop: (path: string) => Promise<void>;
  onEvent: (path: string, e: ControllerEvent) => void;
  reset: (path: string) => void;
}

/** Номер запуска по проекту: опоздавшие события прошлого прогона не попадают в новый. */
const launches = new Map<string, number>();

/** Состояние прогонов по проектам живёт в сторе: уход с вкладки и её закрытие прогон не теряют. */
export const useRunStore = create<RunStore>((set, get) => {
  const patch = (path: string, f: (r: ProjectRun) => ProjectRun) => set((s) => ({ byProject: { ...s.byProject, [path]: f(s.byProject[path] ?? emptyRun()) } }));
  return {
    byProject: {},
    closeRequested: false,
    setCloseRequested: (v) => set({ closeRequested: v }),
    onEvent: (path, e) => patch(path, (r) => reduceEvent(r, e)),
    reset: (path) => {
      launches.set(path, (launches.get(path) ?? 0) + 1);
      patch(path, () => emptyRun());
    },
    start: async (path, params) => {
      const current = get().byProject[path];
      if (current && isActive(current.status)) return;
      const launch = (launches.get(path) ?? 0) + 1;
      launches.set(path, launch);
      const mine = () => launches.get(path) === launch;
      patch(path, () => ({ ...emptyRun(), status: "starting", params }));
      try {
        // Подписка на канал живёт здесь, а не в компоненте: уход с вкладки прогон не теряет.
        const result = await api.runStart(path, params.suite, params.profile, params.env, params.mode, params.maxInFlight, (e) => {
          if (mine()) get().onEvent(path, e);
        });
        if (mine()) patch(path, (r) => ({ ...r, status: "done", result, runId: result.run_id, dir: result.dir }));
      } catch (e) {
        if (mine()) patch(path, (r) => ({ ...r, status: "failed", error: toAppError(e) }));
      }
    },
    stop: async (path) => {
      const before = get().byProject[path]?.status;
      patch(path, (r) => (r.status === "running" || r.status === "starting" ? { ...r, status: "stopping" } : r));
      try {
        await api.runStop(path);
      } catch (e) {
        // Остановка не дошла — кнопка «Остановить» снова доступна.
        if (before) patch(path, (r) => (r.status === "stopping" ? { ...r, status: before } : r));
        throw e;
      }
    },
  };
});

const EMPTY = emptyRun();

/** Состояние прогона проекта (пустое, если прогона не было). */
export const useProjectRun = (path: string): ProjectRun => useRunStore((s) => s.byProject[path] ?? EMPTY);
