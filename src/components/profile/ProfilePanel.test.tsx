import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { ProfilePanel } from "./ProfilePanel";

// vi.mock поднимается над импортами — моки создаются там же через vi.hoisted.
const { api, plot, state } = vi.hoisted(() => {
  const state = {
    readOnly: false,
    file: {
      format: 1,
      // Пустые карты переопределений serde не пишет — их может не быть вовсе.
      coefficients: { k_env: 1, k_growth: 1, overrides: {} } as Record<string, unknown>,
      profiles: {
        nominal: { basis: "avg", phases: [{ kind: "ramp", minutes: 10, to_pct: 100 }, { kind: "steady", minutes: 60 }] },
        peak: { basis: "peak", phases: [{ kind: "steady", minutes: 30 }] },
      },
    },
  };
  const plot = { created: 0, destroyed: 0, observed: 0, disconnected: 0, opts: null as null | { hooks?: { draw?: ((u: unknown) => void)[] } } };
  const api = {
    projectView: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "P", services: [], environments: [] }, problems: [], readOnly: state.readOnly, missingServices: [] })),
    profileGet: vi.fn(async () => state.file),
    profileSave: vi.fn(async (_p: string, f: unknown) => f),
    profileCompute: vi.fn(async (_p: string, name: string) => ({
      profile: name,
      basis: "avg",
      targets: [{ id: "http:o:POST /a", service: "o", kind: "http", basis_value: 100, k_env: 1, k_growth: 1, target: 100, share: 1 }],
      total: 100,
      uncovered: ["http:o:GET /b"],
      timeline: [{ minute: 0, pct: 0 }, { minute: 10, pct: 100 }, { minute: 70, pct: 100 }],
      total_minutes: 70,
    })),
  };
  return { api, plot, state };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("uplot", () => ({
  default: class {
    constructor(opts: typeof plot.opts) {
      plot.created++;
      plot.opts = opts;
    }
    setSize() {}
    destroy() {
      plot.destroyed++;
    }
  },
}));
vi.mock("uplot/dist/uPlot.min.css", () => ({}));

// jsdom не знает ResizeObserver.
class FakeResizeObserver {
  observe() {
    plot.observed++;
  }
  disconnect() {
    plot.disconnected++;
  }
}
vi.stubGlobal("ResizeObserver", FakeResizeObserver);

describe("ProfilePanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    state.readOnly = false;
    plot.created = 0;
    plot.destroyed = 0;
    plot.observed = 0;
    plot.disconnected = 0;
  });

  it("shows targets, edits a phase and saves the file", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    expect(await screen.findByText(/Цели профиля nominal: всего 100.0 RPS, 70 мин/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText("k_env")).toBeEnabled());
    expect(screen.getByText("100.0%")).toBeInTheDocument();
    expect(screen.getByText("Не покрыто цифрами ПРОМ: 1")).toBeInTheDocument();
    await userEvent.clear(screen.getByLabelText("k_env"));
    await userEvent.type(screen.getByLabelText("k_env"), "0.5");
    await userEvent.click(screen.getByRole("button", { name: "Добавить фазу" }));
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.profileSave).toHaveBeenCalledWith(
      "/p",
      expect.objectContaining({
        coefficients: expect.objectContaining({ k_env: 0.5 }),
        profiles: expect.objectContaining({ nominal: expect.objectContaining({ phases: [expect.anything(), expect.anything(), { kind: "steady", minutes: 30 }] }) }),
      }),
    );
  });

  it("switches profiles and recomputes", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    await userEvent.selectOptions(screen.getByLabelText("Профили"), "peak");
    expect(await screen.findByText(/Цели профиля peak/)).toBeInTheDocument();
    expect(api.profileCompute).toHaveBeenLastCalledWith("/p", "peak");
  });

  it("creates the chart once per result and destroys it on unmount", async () => {
    const { unmount } = renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    expect(plot.created).toBeGreaterThan(0);
    unmount();
    expect(plot.destroyed).toBe(plot.created);
    expect(plot.disconnected).toBe(plot.observed);
  });

  it("draws phase boundaries of the computed profile on the chart", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    // nominal: разгон 10 минут, затем полка — граница одна, на 10-й минуте.
    const ctx = { save: vi.fn(), restore: vi.fn(), beginPath: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(), setLineDash: vi.fn(), strokeStyle: "", lineWidth: 0 };
    const draw = plot.opts?.hooks?.draw?.[0];
    expect(draw).toBeTypeOf("function");
    draw?.({ ctx, bbox: { top: 5, height: 100 }, valToPos: (v: number) => v * 3 });
    expect(ctx.moveTo.mock.calls).toEqual([[30.5, 5]]);
    expect(ctx.lineTo.mock.calls).toEqual([[30.5, 105]]);
  });

  it("adds a service override when the file has no override maps", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    await userEvent.type(screen.getByLabelText("Переопределения по сервисам: id сервиса или операции"), "orders");
    await userEvent.click(screen.getAllByRole("button", { name: "Добавить" })[0]);
    await userEvent.type(screen.getByLabelText("Переопределения по сервисам: k_env orders"), "2");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.profileSave).toHaveBeenCalledWith("/p", expect.objectContaining({ coefficients: expect.objectContaining({ overrides: { services: { orders: { k_env: 2 } } } }) }));
  });

  it("blocks saving of values the backend rejects", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    fireEvent.change(screen.getByLabelText("k_growth"), { target: { value: "-1" } });
    expect(screen.getByLabelText("k_growth")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("Исправьте выделенные значения, чтобы сохранить")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
  });

  it("rejects a steps phase with a zero step", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    await userEvent.selectOptions(screen.getByLabelText("Вид фазы 2"), "steps");
    expect(screen.getByLabelText("шаг % 2")).not.toHaveAttribute("aria-invalid");
    await userEvent.clear(screen.getByLabelText("шаг % 2"));
    await userEvent.type(screen.getByLabelText("шаг % 2"), "0");
    expect(screen.getByLabelText("шаг % 2")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
  });

  it("renames and removes profiles, reorders phases", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    await userEvent.type(screen.getByLabelText("имя нового профиля"), "base");
    await userEvent.click(screen.getByRole("button", { name: "Переименовать профиль" }));
    expect(screen.getByLabelText("Профили")).toHaveValue("base");
    await userEvent.click(screen.getByRole("button", { name: "Выше 2" }));
    await userEvent.click(screen.getByRole("button", { name: "Удалить профиль" }));
    expect(screen.queryByRole("option", { name: "base" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("Профили")).toHaveValue("peak");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    const saved = api.profileSave.mock.calls[0][1] as { profiles: Record<string, unknown> };
    expect(Object.keys(saved.profiles)).toEqual(["peak"]);
  });

  it("disables editing and saving in a read-only project", async () => {
    state.readOnly = true;
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    expect(await screen.findByText("Только чтение")).toBeInTheDocument();
    expect(screen.getByLabelText("k_env")).toBeDisabled();
    expect(screen.getByLabelText("минут 1")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Добавить фазу" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Добавить профиль" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
    // Профили по-прежнему можно переключать и смотреть.
    expect(screen.getByLabelText("Профили")).toBeEnabled();
  });

  it("does not turn a cleared field into zero and blocks saving", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    const minutes = screen.getByLabelText("минут 1");
    await waitFor(() => expect(minutes).toBeEnabled());
    await userEvent.clear(minutes);
    expect(minutes).toHaveValue(null);
    expect(minutes).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("Исправьте выделенные значения, чтобы сохранить")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Сохранить" })).toBeDisabled();
    await userEvent.clear(screen.getByLabelText("k_env"));
    expect(screen.getByLabelText("k_env")).toHaveAttribute("aria-invalid", "true");
    await userEvent.click(screen.getByRole("button", { name: "Отменить правки" }));
    expect(minutes).toHaveValue(10);
    expect(screen.getByLabelText("k_env")).toHaveValue(1);
    expect(api.profileSave).not.toHaveBeenCalled();
  });

  it("lets a negative target percent be typed", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    const toPct = screen.getByLabelText("до % 1");
    await waitFor(() => expect(toPct).toBeEnabled());
    await userEvent.clear(toPct);
    await userEvent.type(toPct, "-20");
    expect(toPct).toHaveValue(-20);
    expect(toPct).not.toHaveAttribute("aria-invalid");
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    expect(api.profileSave).toHaveBeenCalledWith("/p", expect.objectContaining({ profiles: expect.objectContaining({ nominal: expect.objectContaining({ phases: [{ kind: "ramp", minutes: 10, to_pct: -20 }, expect.anything()] }) }) }));
  });

  it("asks to save a new profile before computing and keeps the previous targets meanwhile", async () => {
    renderWithClient(<ProfilePanel path="/p" />);
    await screen.findByText(/Цели профиля nominal/);
    await waitFor(() => expect(screen.getByLabelText("имя нового профиля")).toBeEnabled());
    await userEvent.type(screen.getByLabelText("имя нового профиля"), "fresh");
    await userEvent.click(screen.getByRole("button", { name: "Добавить профиль" }));
    expect(screen.getByText("Профиль ещё не сохранён: сохраните, чтобы посчитать цели")).toBeInTheDocument();
    expect(screen.queryByText(/Цели профиля/)).not.toBeInTheDocument();
    // Сохранение меняет файл (и ключ расчёта), но прошлые цели не пропадают на время пересчёта.
    await userEvent.selectOptions(screen.getByLabelText("Профили"), "nominal");
    expect(await screen.findByText(/Цели профиля nominal/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText(/Цели профиля nominal/).closest("section")).not.toHaveAttribute("aria-busy"));
    const calls = api.profileCompute.mock.calls.length;
    let finish: (v: unknown) => void = () => undefined;
    api.profileCompute.mockImplementationOnce(() => new Promise((r) => (finish = r)) as never);
    await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
    await waitFor(() => expect(api.profileCompute).toHaveBeenCalledTimes(calls + 1));
    expect(screen.getByText(/Цели профиля nominal/).closest("section")).toHaveAttribute("aria-busy", "true");
    finish({ profile: "nominal", basis: "avg", targets: [], total: 50, uncovered: [], timeline: [], total_minutes: 70 });
    expect(await screen.findByText(/Цели профиля nominal: всего 50.0 RPS/)).toBeInTheDocument();
  });
});
