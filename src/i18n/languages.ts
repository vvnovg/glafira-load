/** Языки, для которых есть словарь `src/locales/<код>.json`. Остальные 10 языков семейства добавляет этап I7. */
export type LanguageCode = "en" | "ru";

export const LANGUAGES: { code: LanguageCode; nativeName: string }[] = [
  { code: "en", nativeName: "English" },
  { code: "ru", nativeName: "Русский" },
];
