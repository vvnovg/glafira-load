import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { SUITE_TEXT, suiteView } from "../../test/suite";
import { checksCount, drivesOf, nextLine, sliceYaml, stepAction, stepId, SuiteTree } from "./SuiteTree";

describe("sliceYaml", () => {
  it("cuts the step from its line to the next key line and trims trailing blanks", () => {
    expect(sliceYaml(SUITE_TEXT, 6, 10)).toBe(`      - id: create\n        http:\n          operation: "http:orders:POST /v1/orders"\n        expect: { status: 201 }`);
    // Последний шаг: до конца текста, пустая хвостовая строка отброшена.
    expect(sliceYaml(SUITE_TEXT, 10, 14)).toBe(`      - http:\n          operation: "http:orders:GET /v1/orders/{id}"\n        expect: { status: 200, max_ms: 800 }`);
    expect(sliceYaml(SUITE_TEXT, 50, 60)).toBe("");
  });

  it("stops the last step of a scenario before sibling scenario keys", () => {
    const text = `scenarios:
  - id: a
    steps:
      - id: s1
        http:
          operation: "http:o:GET /x"
      - id: s2
        http:
          operation: "http:o:GET /y"
    on_fail: continue
    rate: 5
`;
    // Строка 7 — последний шаг; `to` — за концом текста, как у последнего ключа карты.
    expect(sliceYaml(text, 7, 13)).toBe(`      - id: s2\n        http:\n          operation: "http:o:GET /y"`);
  });

  it("stops the last step of the last scenario before a top-level key", () => {
    const text = `scenarios:
  - id: a
    steps:
      - http:
          operation: "http:o:GET /x"
        expect: { status: 200 }

data:
  users: 10
defaults:
  arrival: poisson
`;
    expect(sliceYaml(text, 4, 13)).toBe(`      - http:\n          operation: "http:o:GET /x"\n        expect: { status: 200 }`);
  });

  it("handles CRLF text without carriage returns in the fragment", () => {
    const text = SUITE_TEXT.replace(/\n/g, "\r\n");
    const out = sliceYaml(text, 6, 10);
    expect(out).not.toContain("\r");
    expect(out).toBe(`      - id: create\n        http:\n          operation: "http:orders:POST /v1/orders"\n        expect: { status: 201 }`);
  });

  it("finds the nearest following line of the map", () => {
    const lines = { "scenarios[0]": 4, "scenarios[0].steps[0]": 6, "scenarios[0].steps[1]": 10 };
    expect(nextLine(lines, 6, 14)).toBe(10);
    expect(nextLine(lines, 10, 14)).toBe(14);
    expect(nextLine(lines, 4, 14)).toBe(6);
  });
});

describe("step helpers", () => {
  it("reads the action kind, operation, checks count, default ids and the driven operation", () => {
    const s = suiteView().suite!.scenarios[0];
    expect(stepAction(s.steps[0])).toEqual({ kind: "http", operation: "http:orders:POST /v1/orders" });
    expect(stepAction({ kafka_consume: { operation: "kafka_consume:orders.created", match: { jsonpath: {} }, timeout_ms: 10000 }, expect: { jsonpath: { "$.id": "exists" }, schema: "response" } })).toEqual({
      kind: "kafka_consume",
      operation: "kafka_consume:orders.created",
    });
    expect(stepAction({ kafka_produce: { operation: "kafka_produce:orders.created", encoding: "json" } })).toEqual({ kind: "kafka_produce", operation: "kafka_produce:orders.created" });
    expect(checksCount(s.steps[0])).toBe(1);
    expect(checksCount(s.steps[1])).toBe(2);
    // Шаг без `expect` (serde его не пишет) — ноль проверок; jsonpath считается по ключам.
    expect(checksCount({ http: { operation: "http:orders:GET /v1/x" } })).toBe(0);
    expect(checksCount({ http: { operation: "http:orders:GET /v1/x" }, expect: { jsonpath: { "$.a": "exists", "$.b": { gt: 1 } }, schema: "response" } })).toBe(3);
    expect(stepId(s.steps[0], 0)).toBe("create");
    expect(stepId(s.steps[1], 1)).toBe("step2");
    expect(drivesOf(s)).toBe("http:orders:POST /v1/orders");
    expect(drivesOf({ ...s, drives: "http:orders:GET /v1/orders/{id}" })).toBe("http:orders:GET /v1/orders/{id}");
    expect(drivesOf({ ...s, steps: [] })).toBe("");
  });
});

describe("SuiteTree", () => {
  it("lists scenarios and steps, shows the YAML of the selected step and opens the file", async () => {
    const onOpenFile = vi.fn();
    render(<SuiteTree view={suiteView()} onOpenFile={onOpenFile} />);
    expect(screen.getByText("order-flow")).toBeInTheDocument();
    expect(screen.getByText("ведёт http:orders:POST /v1/orders")).toBeInTheDocument();
    expect(screen.getByText("шагов: 2")).toBeInTheDocument();
    expect(screen.getByText("Выберите шаг, чтобы увидеть его YAML")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /step2 http:orders:GET/ }));
    expect(screen.getByTestId("step-yaml")).toHaveTextContent('operation: "http:orders:GET /v1/orders/{id}"');
    expect(screen.getByTestId("step-yaml")).not.toHaveTextContent("POST");
    expect(screen.getByText("строка 10")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Открыть файл" }));
    expect(onOpenFile).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /step2 http:orders:GET/ }).closest("li")).toHaveAttribute("aria-selected", "true");
  });

  it("shows a step block without the following keys of the suite", async () => {
    const text = `${SUITE_TEXT}    on_fail: continue\ndata:\n  users: 10\n`;
    render(<SuiteTree view={{ ...suiteView(), text }} onOpenFile={() => undefined} />);
    await userEvent.click(screen.getByRole("button", { name: /step2 http:orders:GET/ }));
    expect(screen.getByTestId("step-yaml")).toHaveTextContent("max_ms: 800");
    expect(screen.getByTestId("step-yaml")).not.toHaveTextContent("on_fail");
    expect(screen.getByTestId("step-yaml")).not.toHaveTextContent("data:");
  });

  it("says the line is unknown for a selected step without a line entry", async () => {
    const view = suiteView();
    delete view.lines["scenarios[0].steps[1]"];
    render(<SuiteTree view={view} onOpenFile={() => undefined} />);
    await userEvent.click(screen.getByRole("button", { name: /step2 http:orders:GET/ }));
    expect(screen.getByText("Строка этого шага неизвестна: откройте файл")).toBeInTheDocument();
    expect(screen.queryByText("Выберите шаг, чтобы увидеть его YAML")).toBeNull();
    expect(screen.queryByTestId("step-yaml")).toBeNull();
  });

  it("selects a step from the keyboard", async () => {
    render(<SuiteTree view={suiteView()} onOpenFile={() => undefined} />);
    screen.getByRole("button", { name: /create http:orders:POST/ }).focus();
    await userEvent.keyboard("{Enter}");
    expect(screen.getByTestId("step-yaml")).toHaveTextContent("- id: create");
    expect(screen.getByText("строка 6")).toBeInTheDocument();
  });

  it("explains when the suite did not parse", () => {
    render(<SuiteTree view={{ ...suiteView(), suite: null }} onOpenFile={() => undefined} />);
    expect(screen.getByText("Сьют не разобран: смотрите отчёт проверки")).toBeInTheDocument();
  });
});
