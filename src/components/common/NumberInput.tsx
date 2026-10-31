import { type InputHTMLAttributes, useEffect, useState } from "react";

/**
 * Разбор текста поля: пустая строка — `undefined`, если поле необязательное, иначе NaN;
 * «-», «1e» и прочие недописанные числа — NaN. Ноль вместо пустого поля не подставляется.
 */
export function parseNumber(text: string, optional: boolean): number | undefined {
  if (text.trim() === "") return optional ? undefined : Number.NaN;
  return Number(text);
}

const show = (v: number | undefined) => (v === undefined || Number.isNaN(v) ? "" : String(v));
const same = (a: number | undefined, b: number | undefined) => a === b || (Number.isNaN(a) && Number.isNaN(b));

type Props = Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange" | "type"> & {
  value: number | undefined;
  onChange: (v: number | undefined) => void;
  /** Пустое поле означает «не задано» (`undefined`), а не ошибку. */
  optional?: boolean;
};

/**
 * Числовое поле с черновиком-строкой: пока человек печатает («», «-», «-1»), текст не заменяется
 * нулём. Недописанное значение уходит в модель как NaN — проверки формы помечают его и не дают сохранить.
 */
export function NumberInput({ value, onChange, optional = false, ...rest }: Props) {
  const [text, setText] = useState(() => show(value));
  // Значение сменилось снаружи (отмена правок, другой профиль) — показываем его.
  useEffect(() => {
    setText((prev) => (same(parseNumber(prev, optional), value) ? prev : show(value)));
  }, [value, optional]);
  return (
    <input
      {...rest}
      type="number"
      step={rest.step ?? "any"}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        onChange(parseNumber(e.target.value, optional));
      }}
    />
  );
}
