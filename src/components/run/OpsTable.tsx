import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Tick } from "../../api/types";
import { formatNumber } from "../../lib/format";
import { type Sort, sortRows, SortTh } from "./sorting";

interface OpRow { id: string; n: number; p95: number; errors: number; dropped: number }

const sum = (xs: number[]): number => xs.reduce((a, b) => a + b, 0);

/** Живая таблица операций по последнему тику: запросов за секунду, p95, ошибки, dropped; сортировка по колонкам. */
export function OpsTable({ tick }: { tick: Tick | null }) {
  const { t } = useTranslation();
  const [sort, setSort] = useState<Sort<OpRow>>({ key: "id", dir: "asc" });
  const rows = useMemo(
    () => sortRows((tick?.ops ?? []).map((o) => ({ id: o.id, n: o.n, p95: o.p95, errors: sum(Object.values(o.err)), dropped: o.dropped })), sort),
    [tick, sort],
  );
  return (
    <section>
      <h4>{t("run.ops.title")}</h4>
      <table>
        <thead>
          <tr>
            <SortTh sort={sort} column="id" label={t("run.ops.operation")} onChange={setSort} />
            <SortTh sort={sort} column="n" label={t("run.ops.n")} onChange={setSort} />
            <SortTh sort={sort} column="p95" label={t("run.ops.p95")} onChange={setSort} />
            <SortTh sort={sort} column="errors" label={t("run.ops.errors")} onChange={setSort} />
            <SortTh sort={sort} column="dropped" label={t("run.ops.dropped")} onChange={setSort} />
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.id}>
              <td>
                <code>{r.id}</code>
              </td>
              <td>{formatNumber(r.n, 0)}</td>
              <td>{formatNumber(r.p95, 1)}</td>
              <td className={r.errors > 0 ? "warn" : undefined}>{formatNumber(r.errors, 0)}</td>
              <td className={r.dropped > 0 ? "warn" : undefined}>{formatNumber(r.dropped, 0)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
