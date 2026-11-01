import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { RunSeries } from "../../state/run";
import { LiveCharts } from "./LiveCharts";

// uPlot рисует в canvas, которого нет в jsdom, — как в тестах TimelineChart его заменяет заглушка.
const plot = vi.hoisted(() => ({ created: 0, destroyed: 0, setData: [] as unknown[], series: [] as number[] }));
vi.mock("uplot", () => ({
  default: class {
    constructor(opts: { series: unknown[] }) {
      plot.created++;
      plot.series.push(opts.series.length);
    }
    setData(d: unknown) {
      plot.setData.push(d);
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
  observe() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", FakeResizeObserver);

const series = (): RunSeries => ({ t: [1, 2], rps: [10, 12], rpsTarget: [11, 11], p95ByOp: { "http:a": [5, 6], "http:b": [7, 8], "http:c": [1, 1], "http:d": [1, 1], "http:e": [1, 1], "http:f": [1, 1] } });
const ops = ["http:a", "http:b", "http:c", "http:d", "http:e", "http:f"];

describe("LiveCharts", () => {
  beforeEach(() => {
    plot.created = 0;
    plot.destroyed = 0;
    plot.setData = [];
    plot.series = [];
  });

  it("creates the two charts once and feeds new ticks through setData", () => {
    const s = series();
    const { rerender, unmount } = render(<LiveCharts series={s} version={1} ops={ops} />);
    expect(screen.getByTestId("rps-chart")).toBeInTheDocument();
    expect(screen.getByTestId("p95-chart")).toBeInTheDocument();
    expect(plot.created).toBe(2);
    // RPS: x, цель, факт; p95: x + первые пять операций.
    expect(plot.series).toEqual([3, 6]);
    const before = plot.setData.length;
    s.t.push(3);
    s.rps.push(14);
    s.rpsTarget.push(11);
    rerender(<LiveCharts series={s} version={2} ops={ops} />);
    expect(plot.created).toBe(2);
    expect(plot.setData.length).toBe(before + 2);
    // Ряды стора мутируются на месте: в график уходят те же массивы с новой точкой.
    expect(plot.setData[plot.setData.length - 2]).toEqual([[1, 2, 3], [11, 11, 11], [10, 12, 14]]);
    unmount();
    expect(plot.destroyed).toBe(2);
  });

  it("limits the p95 chart to five operations and rebuilds it when the choice changes", async () => {
    render(<LiveCharts series={series()} version={1} ops={ops} />);
    expect(screen.getByRole("group", { name: "Операции на графике (до 5)" })).toBeInTheDocument();
    const boxes = screen.getAllByRole("checkbox");
    expect(boxes.map((b) => (b as HTMLInputElement).checked)).toEqual([true, true, true, true, true, false]);
    expect(boxes[5]).toBeDisabled();
    await userEvent.click(boxes[0]);
    expect(boxes[5]).toBeEnabled();
    await userEvent.click(boxes[5]);
    expect(plot.created).toBe(4);
    expect(plot.series[plot.series.length - 1]).toBe(6);
  });
});
