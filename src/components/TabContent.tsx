import { lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import type { Tab } from "../state/workspace";
import { AboutPanel } from "./about/AboutPanel";
import { ChunkBoundary } from "./common/ChunkBoundary";
import { ProdLoadPanel } from "./load/ProdLoadPanel";
import { ProjectPanel } from "./project/ProjectPanel";
import { RegistryPanel } from "./registry/RegistryPanel";
import { ServicePanel } from "./services/ServicePanel";
import { SettingsPanel } from "./settings/SettingsPanel";

// Справка (react-markdown и главы) грузится отдельным чанком при первом открытии.
const HelpPanel = lazy(() => import("./help/HelpPanel").then((m) => ({ default: m.HelpPanel })));

// Профиль нагрузки тянет uPlot — тоже отдельный чанк, грузится при первом открытии вкладки.
const ProfilePanel = lazy(() => import("./profile/ProfilePanel").then((m) => ({ default: m.ProfilePanel })));

function LazyProfile({ path }: { path: string }) {
  const { t } = useTranslation();
  return (
    <ChunkBoundary fallback={<p className="empty">{t("common.chunkFailed")}</p>}>
      <Suspense fallback={<p className="empty">{t("common.loading")}</p>}>
        <ProfilePanel path={path} />
      </Suspense>
    </ChunkBoundary>
  );
}

// Вкладка сьюта — отдельный чанк, как профиль.
const SuitePanel = lazy(() => import("./suites/SuitePanel").then((m) => ({ default: m.SuitePanel })));

function LazySuite({ path, name }: { path: string; name: string }) {
  const { t } = useTranslation();
  return (
    <ChunkBoundary fallback={<p className="empty">{t("common.chunkFailed")}</p>}>
      <Suspense fallback={<p className="empty">{t("common.loading")}</p>}>
        <SuitePanel path={path} name={name} />
      </Suspense>
    </ChunkBoundary>
  );
}

// Вкладка прогона тянет uPlot — отдельный чанк.
const RunPanel = lazy(() => import("./run/RunPanel").then((m) => ({ default: m.RunPanel })));

function LazyRun({ path }: { path: string }) {
  const { t } = useTranslation();
  return (
    <ChunkBoundary fallback={<p className="empty">{t("common.chunkFailed")}</p>}>
      <Suspense fallback={<p className="empty">{t("common.loading")}</p>}>
        <RunPanel path={path} />
      </Suspense>
    </ChunkBoundary>
  );
}

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
    case "prod-load":
      return <ProdLoadPanel path={tab.path} />;
    case "profile":
      return <LazyProfile path={tab.path} />;
    case "suite":
      return <LazySuite path={tab.path} name={tab.name} />;
    case "run":
      return <LazyRun path={tab.path} />;
  }
}
