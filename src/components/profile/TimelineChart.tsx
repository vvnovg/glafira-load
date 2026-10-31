import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import type { Point } from "../../api/types";

const HEIGHT = 220;

function themeColor(name: string, fallback: string): string {
  if (typeof getComputedStyle !== "function") return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

/** График профиля: ось X — минуты, левая ось — % цели, правая — суммарный RPS. Ширина следует за контейнером. */
export function TimelineChart({ points, total }: { points: Point[]; total: number }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el || points.length === 0) return;
    const muted = themeColor("--muted", "#6b7280");
    const axis = { stroke: muted, grid: { stroke: themeColor("--border", "#e3e5e8") }, ticks: { stroke: muted } };
    const plot = new uPlot(
      {
        width: el.clientWidth || 600,
        height: HEIGHT,
        scales: { x: { time: false }, pct: {}, rps: {} },
        axes: [
          { ...axis, label: t("profile.chartMinutes") },
          { ...axis, scale: "pct", label: "%", values: (_u: uPlot, v: number[]) => v.map((x) => `${x}%`) },
          { ...axis, scale: "rps", side: 1, label: "RPS", values: (_u: uPlot, v: number[]) => v.map((x) => x.toFixed(0)) },
        ],
        series: [
          {},
          { label: "%", scale: "pct", stroke: themeColor("--chart-1", "#3b82f6"), width: 2, value: (_u: uPlot, v: number | null) => (v == null ? "—" : `${v}%`) },
          { label: "RPS", scale: "rps", stroke: themeColor("--warn", "#f59e0b"), width: 1, dash: [4, 4], value: (_u: uPlot, v: number | null) => (v == null ? "—" : v.toFixed(1)) },
        ],
      },
      [points.map((p) => p.minute), points.map((p) => p.pct), points.map((p) => (p.pct / 100) * total)] as uPlot.AlignedData,
      el,
    );
    const resize = new ResizeObserver(() => plot.setSize({ width: el.clientWidth || 600, height: HEIGHT }));
    resize.observe(el);
    return () => {
      resize.disconnect();
      plot.destroy();
    };
  }, [points, total, t]);
  return <div className="usage-chart" ref={ref} data-testid="timeline-chart" />;
}
