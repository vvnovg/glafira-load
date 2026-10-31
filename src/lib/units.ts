import type { Figures, OpKind } from "../api/types";

const KINDS: readonly OpKind[] = ["http", "kafka_produce", "kafka_consume", "grpc", "sql"];

/** Единица цифр ПРОМ строки: у чтения из Kafka — сообщения в секунду, у остальных — запросы (как в шаблоне). */
export type RateUnit = "rps" | "msgPerSec";

export function unitFor(kind: OpKind): RateUnit {
  return kind === "kafka_consume" ? "msgPerSec" : "rps";
}

/** Поле задержки строки: у чтения из Kafka — допустимый лаг в секундах, у остальных — p95 в миллисекундах. */
export function latencyFieldFor(kind: OpKind): "p95_ms" | "lag_max_s" {
  return kind === "kafka_consume" ? "lag_max_s" : "p95_ms";
}

/** Вид операции по префиксу её ID (`kind:…`); неизвестный префикс — null. */
export function kindFromId(id: string): OpKind | null {
  const prefix = id.slice(0, id.indexOf(":"));
  return (KINDS as readonly string[]).includes(prefix) ? (prefix as OpKind) : null;
}

/** Значение задержки строки по её виду (undefined — не заполнено). */
export function latencyOf(kind: OpKind, f: Figures): number | undefined {
  return f[latencyFieldFor(kind)];
}
