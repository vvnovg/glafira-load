import { useTranslation } from "react-i18next";
import { useWorkspace } from "../state/workspace";
import { openSupport } from "../lib/links";

export function Sidebar() {
  const { t } = useTranslation();
  const open = useWorkspace((s) => s.open);

  return (
    <aside className="sidebar">
      <div className="tree" data-testid="project-tree">
        <p className="empty">{t("sidebar.emptyHint")}</p>
      </div>
      <div className="sidebar-footer">
        <button type="button" className="link" onClick={() => open({ kind: "settings" })}>
          {t("sidebar.settings")}
        </button>
        <button type="button" className="link" onClick={openSupport}>
          {t("support.title")}
        </button>
        <button type="button" className="link" onClick={() => open({ kind: "help" })}>
          {t("sidebar.help")}
        </button>
        <button type="button" className="link" onClick={() => open({ kind: "about" })}>
          {t("sidebar.about")}
        </button>
      </div>
    </aside>
  );
}
