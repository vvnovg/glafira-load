import { describe, expect, it } from "vitest";
import type { TabSpec } from "../../state/workspace";
import { helpChapterFor } from "./helpChapterFor";

const cases: [TabSpec | null, string][] = [
  [{ kind: "project", path: null }, "project"],
  [{ kind: "project", path: "/p" }, "project"],
  [{ kind: "service", path: "/p", serviceId: "orders" }, "project"],
  [{ kind: "registry", path: "/p" }, "registry"],
  [{ kind: "prod-load", path: "/p" }, "prod-load"],
  [{ kind: "profile", path: "/p" }, "profile"],
  [{ kind: "suite", path: "/p", name: "orders" }, "suites"],
  [{ kind: "run", path: "/p" }, "run"],
  [{ kind: "about" }, "updates"],
  [{ kind: "settings" }, "getting-started"],
  [{ kind: "help", chapter: "registry" }, "getting-started"],
  [null, "getting-started"],
];

describe("helpChapterFor", () => {
  it.each(cases)("%j → %s", (tab, chapter) => {
    expect(helpChapterFor(tab)).toBe(chapter);
  });
});
