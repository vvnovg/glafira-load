import "@testing-library/jest-dom/vitest";
import { i18n } from "../i18n";

await i18n.changeLanguage("ru");

// jsdom не реализует matchMedia, а uPlot вызывает его при загрузке модуля (TabContent тянет его через вкладку профиля).
if (typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string) =>
    ({ matches: false, media: query, onchange: null, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false }) as MediaQueryList;
}
