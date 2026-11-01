import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { OpSummary } from "../../api/types";
import { renderWithClient } from "../../test/render";
import { sortRows, toggleSort } from "./sorting";
import { SummaryTable } from "./SummaryTable";

const api = vi.hoisted(() => ({ openPath: vi.fn(async () => undefined) }));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));

const op = (id: string, n: number, p95: number, target: number | null): OpSummary => ({ id, n, rpsAvg: n / 10, rpsPeak: n / 5, err: {}, errPct: 0, dropped: 0, p50: 1, p90: 2, p95, p99: 4, max: 5, target, achievedPct: target ? 100 : null });
const summary = { status: "stopped" as const, ops: [op("http:a", 100, 40, 10), op("http:b", 300, 20, null), op("http:c", 200, 30, 20)], scenarios: [], seconds: 600 };

const ids = () => within(screen.getByRole("table")).getAllByRole("row").slice(1).map((r) => within(r).getAllByRole("cell")[0].textContent);

describe("sorting", () => {
  it("sorts numbers and strings, nulls last, and toggles the direction", () => {
    const rows = [{ id: "b", v: 2 as number | null }, { id: "a", v: null }, { id: "c", v: 1 as number | null }];
    expect(sortRows(rows, { key: "v", dir: "asc" }).map((r) => r.id)).toEqual(["c", "b", "a"]);
    expect(sortRows(rows, { key: "v", dir: "desc" }).map((r) => r.id)).toEqual(["b", "c", "a"]);
    expect(sortRows(rows, { key: "id", dir: "asc" }).map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(toggleSort({ key: "id", dir: "asc" }, "id")).toEqual({ key: "id", dir: "desc" });
    expect(toggleSort({ key: "id", dir: "desc" }, "v")).toEqual({ key: "v", dir: "asc" });
  });
});

describe("SummaryTable", () => {
  beforeEach(() => vi.clearAllMocks());

  it("sorts by a column on click and opens the run folder", async () => {
    renderWithClient(<SummaryTable summary={summary} path="/p" dir="runs/2026-11-02T14-30-00" />);
    expect(screen.getByText(/остановлен · 600 с/)).toBeInTheDocument();
    expect(ids()).toEqual(["http:a", "http:b", "http:c"]);
    await userEvent.click(screen.getByRole("button", { name: "Сортировать по p95, мс" }));
    expect(ids()).toEqual(["http:b", "http:c", "http:a"]);
    await userEvent.click(screen.getByRole("button", { name: "Сортировать по p95, мс" }));
    expect(ids()).toEqual(["http:a", "http:c", "http:b"]);
    await userEvent.click(screen.getByRole("button", { name: "Сортировать по цель RPS" }));
    expect(ids()).toEqual(["http:a", "http:c", "http:b"]);
    expect(screen.getByRole("columnheader", { name: /цель RPS/ })).toHaveAttribute("aria-sort", "ascending");
    // Пустая цель — прочерк.
    expect(within(screen.getAllByRole("row")[3]).getAllByRole("cell")[10]).toHaveTextContent("—");
    await userEvent.click(screen.getByRole("button", { name: "Открыть папку" }));
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/runs/2026-11-02T14-30-00");
  });

  it("opens an absolute run folder as is", async () => {
    renderWithClient(<SummaryTable summary={summary} path="/p" dir="/p/runs/x" />);
    await userEvent.click(screen.getByRole("button", { name: "Открыть папку" }));
    expect(api.openPath).toHaveBeenCalledWith("/p", "/p/runs/x");
  });
});
