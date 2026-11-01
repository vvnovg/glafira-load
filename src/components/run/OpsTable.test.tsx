import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { OpTick, Tick } from "../../api/types";
import { OpsTable } from "./OpsTable";

const op = (id: string, n: number, p95: number, err: Record<string, number> = {}, dropped = 0): OpTick => ({ id, n, err, dropped, p50: 1, p90: 2, p95, p99: 4, max: 5, bytesIn: 0, bytesOut: 0, lag: null });
const tick = (ops: OpTick[]): Tick => ({ t: 1, ops, scenarios: [] });
const ids = () => within(screen.getByRole("table")).getAllByRole("row").slice(1).map((r) => within(r).getAllByRole("cell")[0].textContent);

describe("OpsTable", () => {
  it("shows the last tick per operation, keeps the sort across ticks and marks errors", async () => {
    const { rerender } = render(<OpsTable tick={tick([op("http:b", 5, 10), op("http:a", 9, 30, { HTTP_5xx: 2, TIMEOUT: 1 })])} />);
    expect(ids()).toEqual(["http:a", "http:b"]);
    await userEvent.click(screen.getByRole("button", { name: "Сортировать по ошибки" }));
    await userEvent.click(screen.getByRole("button", { name: "Сортировать по ошибки" }));
    expect(ids()).toEqual(["http:a", "http:b"]);
    expect(within(screen.getAllByRole("row")[1]).getAllByRole("cell")[3]).toHaveTextContent("3");
    expect(within(screen.getAllByRole("row")[1]).getAllByRole("cell")[3]).toHaveClass("warn");
    // Новый тик: порядок по-прежнему по ошибкам, по убыванию.
    rerender(<OpsTable tick={tick([op("http:a", 9, 30), op("http:b", 5, 10, { HTTP_4xx: 4 })])} />);
    expect(ids()).toEqual(["http:b", "http:a"]);
  });
});
