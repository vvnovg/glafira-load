import type { TFunction } from "i18next";
import type { Figures, OpKind } from "../../api/types";
import { formatNumber } from "../../lib/format";
import { latencyFieldFor, latencyOf, unitFor } from "../../lib/units";

/** Задержка строки с единицей: p95 в мс или лаг потребителя в секундах; не заполнена — «—». */
export function latencyText(t: TFunction, kind: OpKind, f: Figures): string {
  const v = latencyOf(kind, f);
  if (v === undefined) return "—";
  return t(latencyFieldFor(kind) === "lag_max_s" ? "units.seconds" : "units.ms", { value: formatNumber(v) });
}

/** Цифры одной строкой для диффа импорта: «среднее/пик ед. · ошибки % · p95 или лаг». Вид неизвестен — без единиц. */
export function figuresText(t: TFunction, kind: OpKind | null, f: Figures | null): string {
  if (!f) return "—";
  const unit = kind ? t(`units.${unitFor(kind)}`) : "";
  const parts = [t("prodLoad.figures.pair", { avg: formatNumber(f.avg), peak: formatNumber(f.peak), unit }).trim()];
  if (f.error_pct !== undefined) parts.push(t("units.percent", { value: formatNumber(f.error_pct) }));
  if (f.p95_ms !== undefined) parts.push(t("prodLoad.figures.p95", { value: t("units.ms", { value: formatNumber(f.p95_ms) }) }));
  if (f.lag_max_s !== undefined) parts.push(t("prodLoad.figures.lag", { value: t("units.seconds", { value: formatNumber(f.lag_max_s) }) }));
  return parts.join(" · ");
}
