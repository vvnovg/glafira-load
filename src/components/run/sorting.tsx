import { useTranslation } from "react-i18next";

export type SortDir = "asc" | "desc";
export interface Sort<T> { key: keyof T; dir: SortDir }

/** Клик по той же колонке меняет направление, по другой — сортирует по ней по возрастанию. */
export function toggleSort<T>(s: Sort<T>, key: keyof T): Sort<T> {
  return s.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "asc" };
}

/** Копия строк, отсортированная по колонке: числа как числа, строки по локали, null всегда в конце. */
export function sortRows<T>(rows: T[], s: Sort<T>): T[] {
  const sign = s.dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const x = a[s.key];
    const y = b[s.key];
    if (x === y) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    return (typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y))) * sign;
  });
}

/** Заголовок колонки с сортировкой по клику (кнопка — доступна с клавиатуры) и `aria-sort`. */
export function SortTh<T>({ sort, column, label, onChange }: { sort: Sort<T>; column: keyof T; label: string; onChange: (s: Sort<T>) => void }) {
  const { t } = useTranslation();
  const active = sort.key === column;
  return (
    <th aria-sort={active ? (sort.dir === "asc" ? "ascending" : "descending") : undefined}>
      <button type="button" className="link sort" aria-label={t("run.sort", { column: label })} onClick={() => onChange(toggleSort(sort, column))}>
        {label}
        {active ? (sort.dir === "asc" ? " ▲" : " ▼") : ""}
      </button>
    </th>
  );
}
