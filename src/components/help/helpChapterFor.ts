import type { ChapterId } from "../../guide/chapters";
import type { TabSpec } from "../../state/workspace";

/** Глава руководства для экрана (F1). */
export function helpChapterFor(tab: TabSpec | null): ChapterId {
  switch (tab?.kind) {
    case "project":
    case "service":
      return "project";
    case "registry":
      return "registry";
    case "prod-load":
      return "prod-load";
    case "profile":
      return "profile";
    case "about":
      return "updates";
    default:
      return "getting-started";
  }
}
