import { describe, expect, it } from "vitest";
import { GUIDE_CHAPTERS, loadGuide } from "./chapters";

describe("loadGuide", () => {
  it("loads every chapter of a language from docs/guide", async () => {
    const ru = await loadGuide("ru");
    expect(Object.keys(ru).sort()).toEqual([...GUIDE_CHAPTERS].sort());
    expect(ru.project).toMatch(/^# Проект и сервисы/);
  });

  it("falls back to English for a language without chapters", async () => {
    const [pseudo, en] = await Promise.all([loadGuide("x-pseudo"), loadGuide("en")]);
    expect(pseudo).toEqual(en);
    expect(en["getting-started"]).toMatch(/^# /);
  });
});
