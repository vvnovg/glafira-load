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

/** Вертикальные пунктирные линии на минутах `boundaries` поверх области графика. */
function drawBoundaries(boundaries: number[], color: string) {
  return (u: uPlot) => {
    if (boundaries.length === 0) return;
    const { ctx, bbox } = u;
    ctx.save();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 3]);
    for (const b of boundaries) {
      const x = Math.round(u.valToPos(b, "x", true)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, bbox.top);
      ctx.lineTo(x, bbox.top + bbox.height);
      ctx.stroke();
    }
    ctx.restore();
  };
}

/**
 * График профиля: ось X — минуты, левая ось — % цели, правая — суммарный RPS; пунктир — границы фаз.
 * Ширина следует за контейнером.
 */
export function TimelineChart({ points, total, boundaries = [] }: { points: Point[]; total: number; boundaries?: number[] }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  // Массив границ приходит новым на каждый рендер — пересоздаём график только при смене значений.
  const boundaryKey = boundaries.join(",");
  useEffect(() => {
    const el = ref.current;
    if (!el || points.length === 0) return;
    const marks = boundaryKey ? boundaryKey.split(",").map(Number) : [];
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
        hooks: { draw: [drawBoundaries(marks, muted)] },
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
  }, [points, total, boundaryKey, t]);
  return <div className="usage-chart" ref={ref} data-testid="timeline-chart" />;
}
