import type { TFunction } from "i18next";
import { type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import type { RunSeries } from "../../state/run";

const HEIGHT = 220;
/** Сколько операций можно вывести на график p95. */
export const MAX_OPS = 5;
const COLORS = ["--chart-1", "--chart-2", "--chart-3", "--chart-4", "--chart-5"];
const FALLBACK = ["#3b82f6", "#16a34a", "#f59e0b", "#dc2626", "#8b5cf6"];

type PlotOptions = Omit<uPlot.Options, "width" | "height">;

function themeColor(name: string, fallback: string): string {
  if (typeof getComputedStyle !== "function") return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function axisStyle() {
  const muted = themeColor("--muted", "#6b7280");
  return { stroke: muted, grid: { stroke: themeColor("--border", "#e3e5e8") }, ticks: { stroke: muted } };
}

const fmt = (_u: uPlot, v: number | null): string => (v == null ? "—" : v.toFixed(1));

function rpsOptions(t: TFunction): PlotOptions {
  const axis = axisStyle();
  return {
    scales: { x: { time: false } },
    axes: [{ ...axis, label: t("run.chart.seconds") }, { ...axis, label: "RPS" }],
    series: [
      {},
      { label: t("run.chart.target"), stroke: themeColor("--warn", "#f59e0b"), width: 1, dash: [4, 4], value: fmt },
      { label: t("run.chart.actual"), stroke: themeColor("--chart-1", "#3b82f6"), width: 2, value: fmt },
    ],
  };
}

function p95Options(t: TFunction, ops: string[]): PlotOptions {
  const axis = axisStyle();
  return {
    scales: { x: { time: false } },
    axes: [{ ...axis, label: t("run.chart.seconds") }, { ...axis, label: "ms" }],
    series: [{}, ...ops.map((id, i) => ({ label: id, stroke: themeColor(COLORS[i], FALLBACK[i]), width: 1, value: fmt }))],
  };
}

/**
 * График создаётся один раз на `key` (набор серий, язык) и следует за шириной контейнера; новые данные уходят через
 * `setData` без пересоздания.
 */
function usePlot(el: RefObject<HTMLDivElement | null>, options: () => PlotOptions, data: uPlot.AlignedData, key: string) {
  const plotRef = useRef<uPlot | null>(null);
  // Опции и данные первого рендера читаются из ref: эффект создания зависит только от `key`.
  const optionsRef = useRef(options);
  const dataRef = useRef(data);
  useEffect(() => {
    optionsRef.current = options;
    dataRef.current = data;
  });
  useEffect(() => {
    const node = el.current;
    if (!node) return;
    const plot = new uPlot({ ...optionsRef.current(), width: node.clientWidth || 600, height: HEIGHT }, dataRef.current, node);
    plotRef.current = plot;
    const resize = new ResizeObserver(() => plot.setSize({ width: node.clientWidth || 600, height: HEIGHT }));
    resize.observe(node);
    return () => {
      resize.disconnect();
      plot.destroy();
      plotRef.current = null;
    };
  }, [el, key]);
  useEffect(() => {
    plotRef.current?.setData(data);
  }, [data]);
}

/** Два графика прогона: суммарный RPS (цель и факт) и p95 по выбранным операциям (до MAX_OPS). */
export function LiveCharts({ series, version, ops }: { series: RunSeries; version: number; ops: string[] }) {
  const { t, i18n } = useTranslation();
  const [picked, setPicked] = useState<string[] | null>(null);
  // Пока человек ничего не выбрал — первые MAX_OPS операций.
  const selected = (picked ?? ops.slice(0, MAX_OPS)).filter((id) => ops.includes(id));
  const selectedKey = selected.join("\u0000");
  const lang = i18n.resolvedLanguage ?? "";
  const rpsRef = useRef<HTMLDivElement>(null);
  const p95Ref = useRef<HTMLDivElement>(null);
  // Ряды в сторе мутируются на месте — новая ссылка на данные появляется по `version`.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const rpsData = useMemo(() => [series.t, series.rpsTarget, series.rps] as uPlot.AlignedData, [series, version]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const p95Data = useMemo(() => [series.t, ...selected.map((id) => series.p95ByOp[id] ?? [])] as uPlot.AlignedData, [series, version, selectedKey]);
  usePlot(rpsRef, () => rpsOptions(t), rpsData, `rps\u0000${lang}`);
  usePlot(p95Ref, () => p95Options(t, selected), p95Data, `${lang}\u0000${selectedKey}`);
  const toggle = (id: string) =>
    setPicked(selected.includes(id) ? selected.filter((x) => x !== id) : selected.length >= MAX_OPS ? selected : [...selected, id]);
  return (
    <div className="charts">
      <h4>{t("run.chart.rps")}</h4>
      <div className="usage-chart" ref={rpsRef} data-testid="rps-chart" />
      <h4>{t("run.chart.p95")}</h4>
      <fieldset className="plain row" aria-label={t("run.chart.pickOps", { max: MAX_OPS })}>
        {ops.map((id) => (
          <label key={id} className="inline">
            <input type="checkbox" checked={selected.includes(id)} disabled={!selected.includes(id) && selected.length >= MAX_OPS} onChange={() => toggle(id)} /> <code>{id}</code>
          </label>
        ))}
      </fieldset>
      <div className="usage-chart" ref={p95Ref} data-testid="p95-chart" />
    </div>
  );
}
