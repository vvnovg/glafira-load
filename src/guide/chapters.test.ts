import { describe, expect, it } from "vitest";
import { GUIDE_CHAPTERS as SCRIPT_CHAPTERS } from "../../scripts/i18n-check.mjs";
import { GUIDE_CHAPTERS, loadGuide } from "./chapters";

describe("loadGuide", () => {
  it("loads every chapter of a language from docs/guide", async () => {
    const ru = await loadGuide("ru");
    expect(Object.keys(ru).sort()).toEqual([...GUIDE_CHAPTERS].sort());
    expect(ru.project).toMatch(/^# Проект и сервисы/);
    expect(ru.suites).toMatch(/^# Сьюты/);
    expect(ru.agents).toMatch(/^# Агенты и стенды/);
    expect(ru.run).toMatch(/^# Прогон/);
  });

  it("falls back to English for a language without chapters", async () => {
    const [pseudo, en] = await Promise.all([loadGuide("x-pseudo"), loadGuide("en")]);
    expect(pseudo).toEqual(en);
    expect(en["getting-started"]).toMatch(/^# /);
    expect(en.suites).toMatch(/^# Suites/);
  });

  it("is the same list as in scripts/i18n-check.mjs", () => {
    expect([...GUIDE_CHAPTERS]).toEqual(SCRIPT_CHAPTERS);
  });
});
