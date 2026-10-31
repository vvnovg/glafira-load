import { i18n } from "../../i18n";
import { templateLabels } from "./templateLabels";

describe("templateLabels", () => {
  it("builds 12 columns, 12 notes and 5 params in the current language", () => {
    const l = templateLabels(i18n.t.bind(i18n));
    expect(l.columns).toHaveLength(12);
    expect(l.notes).toHaveLength(12);
    expect(l.params).toHaveLength(5);
    expect(l.columns[0]).toBe("ID");
    expect(l.columns[10]).toBe("p95 ПРОМ, мс");
    expect(l.notes[5]).toContain("сообщений/с");
    expect(l.notes[0]).toBe("");
    expect(Array.isArray(l.helpLines) && l.helpLines.length > 0).toBe(true);
  });
});
