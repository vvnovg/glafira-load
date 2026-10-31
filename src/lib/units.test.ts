import { describe, expect, it } from "vitest";
import { kindFromId, latencyFieldFor, latencyOf, unitFor } from "./units";

describe("units", () => {
  it("единица и поле задержки по виду операции", () => {
    expect(unitFor("http")).toBe("rps");
    expect(unitFor("kafka_produce")).toBe("rps");
    expect(unitFor("kafka_consume")).toBe("msgPerSec");
    expect(latencyFieldFor("grpc")).toBe("p95_ms");
    expect(latencyFieldFor("kafka_consume")).toBe("lag_max_s");
    expect(latencyOf("kafka_consume", { avg: 1, peak: 2, p95_ms: 5, lag_max_s: 3 })).toBe(3);
  });

  it("вид по ID", () => {
    expect(kindFromId("kafka_consume:orders.paid")).toBe("kafka_consume");
    expect(kindFromId("http:o:POST /a")).toBe("http");
    expect(kindFromId("weird")).toBeNull();
    expect(kindFromId("other:x")).toBeNull();
  });
});
