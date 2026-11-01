import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { checkReport } from "../../test/suite";
import { CheckReport, hasErrors } from "./CheckReport";

describe("CheckReport", () => {
  it("lists findings with level, file:line and message; a click opens the file", async () => {
    const onOpen = vi.fn();
    const findings = checkReport().findings;
    render(<CheckReport findings={findings} onOpen={onOpen} />);
    expect(screen.getByText("ошибка")).toHaveClass("sev-error");
    expect(screen.getByText("предупреждение")).toHaveClass("sev-warning");
    await userEvent.click(screen.getByRole("button", { name: "suites/orders/suite.yaml:8" }));
    expect(onOpen).toHaveBeenCalledWith("suites/orders/suite.yaml");
    // Без строки — только файл.
    expect(screen.getByRole("button", { name: "suites/orders/suite.yaml" })).toBeInTheDocument();
    expect(screen.getByText("operation http:orders:GET /v1/orders/{id} is removed")).toBeInTheDocument();
    expect(hasErrors(findings)).toBe(true);
    expect(hasErrors([findings[1]])).toBe(false);
  });

  it("says when there is nothing to report", () => {
    render(<CheckReport findings={[]} onOpen={() => undefined} />);
    expect(screen.getByText("Проблем не найдено")).toBeInTheDocument();
  });
});
