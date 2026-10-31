import { useTranslation } from "react-i18next";
import { type Tab, useWorkspace } from "../state/workspace";
import { Sidebar } from "./Sidebar";
import { i18n } from "../i18n";
import { TabContent } from "./TabContent";
import { UpdateNotice } from "./about/UpdateNotice";
import { TabActiveContext } from "../lib/tabActive";
import { useHelpShortcut } from "../lib/useHelpShortcut";

/** Имя папки проекта: заголовки вкладок до загрузки манифеста (и для тестов). */
export function projectLabel(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
}

function tabTitle(tab: Tab): string {
  switch (tab.kind) {
    case "settings":
      return i18n.t("tabs.settings");
    case "about":
      return i18n.t("tabs.about");
    case "help":
      return i18n.t("tabs.help");
    case "project":
      return tab.path ? i18n.t("tabs.project", { name: projectLabel(tab.path) }) : i18n.t("tabs.newProject");
    case "service":
      return i18n.t("tabs.service", { name: projectLabel(tab.path), service: tab.serviceId });
    case "registry":
      return i18n.t("tabs.registry", { name: projectLabel(tab.path) });
    case "prod-load":
      return i18n.t("tabs.prodLoad", { name: projectLabel(tab.path) });
    case "profile":
      return i18n.t("tabs.profile", { name: projectLabel(tab.path) });
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
