import { lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import type { Tab } from "../state/workspace";
import { AboutPanel } from "./about/AboutPanel";
import { ChunkBoundary } from "./common/ChunkBoundary";
import { ProjectPanel } from "./project/ProjectPanel";
import { RegistryPanel } from "./registry/RegistryPanel";
import { ServicePanel } from "./services/ServicePanel";
import { SettingsPanel } from "./settings/SettingsPanel";

// Справка (react-markdown и главы) грузится отдельным чанком при первом открытии.
const HelpPanel = lazy(() => import("./help/HelpPanel").then((m) => ({ default: m.HelpPanel })));

function LazyHelp({ tab }: { tab: Extract<Tab, { kind: "help" }> }) {
  const { t } = useTranslation();
  return (
    <ChunkBoundary fallback={<p className="empty">{t("help.loadFailed")}</p>}>
      <Suspense fallback={<p className="empty">{t("common.loading")}</p>}>
        <HelpPanel spec={tab} />
      </Suspense>
    </ChunkBoundary>
  );
}

export function TabContent({ tab }: { tab: Tab }) {
  const { t } = useTranslation();
  switch (tab.kind) {
    case "about":
      return <AboutPanel />;
    case "help":
      return <LazyHelp tab={tab} />;
    case "settings":
      return <SettingsPanel />;
    case "project":
      return <ProjectPanel path={tab.path} />;
    case "service":
      return <ServicePanel path={tab.path} serviceId={tab.serviceId} />;
    case "registry":
      return <RegistryPanel path={tab.path} />;
    default:
      // Остальные экраны проекта появляются в Tasks 18–19.
      return <p className="empty">{t("common.loading")}</p>;
  }
}
