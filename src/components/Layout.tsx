import { useTranslation } from "react-i18next";
import { type Tab, useWorkspace } from "../state/workspace";
import { Sidebar } from "./Sidebar";
import { i18n } from "../i18n";
import { TabContent } from "./TabContent";
import { UpdateNotice } from "./about/UpdateNotice";
import { RunCloseDialog } from "./run/RunCloseDialog";
import { TabActiveContext } from "../lib/tabActive";
import { useHelpShortcut } from "../lib/useHelpShortcut";
import { baseName } from "../lib/paths";

function tabTitle(tab: Tab): string {
  switch (tab.kind) {
    case "settings":
      return i18n.t("tabs.settings");
    case "about":
      return i18n.t("tabs.about");
    case "help":
      return i18n.t("tabs.help");
    case "project":
      return tab.path ? i18n.t("tabs.project", { name: baseName(tab.path) }) : i18n.t("tabs.newProject");
    case "service":
      return i18n.t("tabs.service", { name: baseName(tab.path), service: tab.serviceId });
    case "registry":
      return i18n.t("tabs.registry", { name: baseName(tab.path) });
    case "prod-load":
      return i18n.t("tabs.prodLoad", { name: baseName(tab.path) });
    case "profile":
      return i18n.t("tabs.profile", { name: baseName(tab.path) });
    case "suite":
      return i18n.t("tabs.suite", { name: baseName(tab.path), suite: tab.name });
    case "run":
      return i18n.t("tabs.run", { name: baseName(tab.path) });
  }
}

export function Layout() {
  const { t } = useTranslation(); // заодно перерисовывает заголовки вкладок при смене языка
  const { tabs, activeId, activate, close } = useWorkspace();
  useHelpShortcut();

  return (
    <div className="layout">
      <Sidebar />
      <main className="workspace">
        {/* Диалог закрытия окна во время прогона — вне вкладок: вкладку прогона могут закрыть. */}
        <RunCloseDialog />
        <UpdateNotice />
        <nav className="tabbar" role="tablist">
          {tabs.map((tab) => {
            const isActive = tab.id === activeId;
            return (
              <div key={tab.id} className={isActive ? "tab active" : "tab"}>
                <button type="button" role="tab" aria-selected={isActive} onClick={() => activate(tab.id)}>
                  {tabTitle(tab)}
                </button>
                <button
                  type="button"
                  className="icon"
                  aria-label={t("layout.closeTab")}
                  onClick={(e) => {
                    e.stopPropagation();
                    close(tab.id);
                  }}
                >
                  ×
                </button>
              </div>
            );
          })}
        </nav>
        {tabs.length === 0 && <p className="empty">{t("layout.empty")}</p>}
        {/* Неактивные вкладки скрыты, но не размонтированы — состояние форм и прогресс разбора сохраняются. */}
        {tabs.map((tab) => (
          <section key={tab.id} className="tab-content" hidden={tab.id !== activeId}>
            <TabActiveContext.Provider value={tab.id === activeId}>
              <TabContent tab={tab} />
            </TabActiveContext.Provider>
          </section>
        ))}
      </main>
    </div>
  );
}
