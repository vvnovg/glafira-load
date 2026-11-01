import type { CheckReport, SuiteView } from "../api/types";

/** Текст сьюта `orders`: строки совпадают с картой `lines` в `suiteView()`. */
export const SUITE_TEXT = `format: 1
name: orders
scenarios:
  - id: order-flow
    steps:
      - id: create
        http:
          operation: "http:orders:POST /v1/orders"
        expect: { status: 201 }
      - http:
          operation: "http:orders:GET /v1/orders/{id}"
        expect: { status: 200, max_ms: 800 }
`;

/**
 * Разобранный сьют `orders`: один сценарий, два HTTP-шага (второй без id).
 * Поля с `skip_serializing_if` в Rust опущены, как их и присылает IPC.
 */
export function suiteView(): SuiteView {
  return {
    name: "orders",
    file: "suites/orders/suite.yaml",
    text: SUITE_TEXT,
    suite: {
      format: 1,
      name: "orders",
      defaults: { http: { timeout_ms: 5000, headers: {} }, kafka: {}, arrival: "poisson", max_in_flight: 1000 },
      scenarios: [
        {
          id: "order-flow",
          on_fail: "stop",
          steps: [
            { id: "create", http: { operation: "http:orders:POST /v1/orders" }, expect: { status: 201 } },
            { http: { operation: "http:orders:GET /v1/orders/{id}" }, expect: { status: 200, max_ms: 800 } },
          ],
        },
      ],
    },
    findings: [],
    lines: { "scenarios[0]": 4, "scenarios[0].steps[0]": 6, "scenarios[0].steps[1]": 10 },
  };
}

/** Отчёт `check`: ошибка со строкой, предупреждение без строки, покрытие с расхождением на второй операции. */
export function checkReport(): CheckReport {
  return {
    findings: [
      { level: "error", file: "suites/orders/suite.yaml", line: 8, message: "operation http:orders:POST /v1/orders is not in the registry" },
      { level: "warning", file: "suites/orders/suite.yaml", line: null, message: "operation http:orders:GET /v1/orders/{id} is removed" },
    ],
    coverage: {
      rows: [
        { id: "http:orders:POST /v1/orders", kind: "http", target: 100, planned: 100, delta: 0 },
        { id: "http:orders:GET /v1/orders/{id}", kind: "http", target: 50, planned: 100, delta: 50 },
      ],
      uncovered: ["kafka_consume:orders.created"],
    },
    profile: "nominal",
  };
}
