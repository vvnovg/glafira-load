import { render, screen } from "@testing-library/react";
import { checkReport } from "../../test/suite";
import { CoverageTable, isOff } from "./CoverageTable";

describe("CoverageTable", () => {
  it("shows target, planned and delta per operation and highlights a delta over 10 % of the target", () => {
    const coverage = checkReport().coverage!;
    render(<CoverageTable coverage={coverage} profile="nominal" />);
    expect(screen.getByRole("heading", { name: /Покрытие профиля nominal/ })).toBeInTheDocument();
    const deltas = screen.getAllByTestId("delta");
    expect(deltas.map((d) => d.textContent)).toEqual(["0", "+50"]);
    expect(deltas[0]).not.toHaveClass("warn");
    expect(deltas[1]).toHaveClass("warn");
    expect(screen.getByText("Не покрыто: 1")).toBeInTheDocument();
    expect(screen.getByText("kafka_consume:orders.created")).toBeInTheDocument();
  });

  it("isOff compares |delta| with 10 % of the target", () => {
    expect(isOff({ id: "a", kind: "http", target: 100, planned: 108, delta: 8 })).toBe(false);
    expect(isOff({ id: "a", kind: "http", target: 100, planned: 89, delta: -11 })).toBe(true);
    expect(isOff({ id: "a", kind: "http", target: 0, planned: 0, delta: 0 })).toBe(false);
    expect(isOff({ id: "a", kind: "http", target: 0, planned: 3, delta: 3 })).toBe(true);
  });
});
