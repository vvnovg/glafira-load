import { useMutation, useQuery } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import { joinProjectPath } from "../../lib/paths";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { CheckReport } from "./CheckReport";
import { CoverageTable } from "./CoverageTable";
import { CreateSuiteDialog } from "./CreateSuiteDialog";
import { SuiteTree } from "./SuiteTree";

/** Вкладка «Сьют»: дерево шагов, отчёт `check`, покрытие профиля; файл правят в редакторе — вкладка его перечитывает. */
export function SuitePanel({ path, name }: { path: string; name: string }) {
  const { t } = useTranslation();
  const [creating, setCreating] = useState(false);
  // Файл меняют снаружи: при возврате фокуса в окно и по кнопке «Проверить» оба запроса перечитывают диск
  // (глобально во `App` перезапрос по фокусу выключен — здесь он нужен). `refetchOnWindowFocus` TanStack Query
  // срабатывает только на `visibilitychange`, а окно Tauri его не шлёт при переходе в редактор и обратно
  // (лишь при сворачивании) — поэтому ещё и подписка на фокус окна Tauri ниже.
  const view = useQuery({ queryKey: ["suite", path, name], queryFn: () => api.suiteGet(path, name), refetchOnWindowFocus: true });
  const check = useQuery({ queryKey: ["suiteCheck", path, name], queryFn: () => api.suiteCheck(path, name), refetchOnWindowFocus: true });
  const { refetch: refetchView } = view;
  const { refetch: refetchCheck } = check;
  useEffect(() => {
    let unlisten: Promise<(() => void) | null>;
    try {
      unlisten = getCurrentWindow()
        .onFocusChanged(({ payload: focused }) => {
          if (focused) {
            void refetchView();
            void refetchCheck();
          }
        })
        .catch(() => null);
    } catch {
      // Вне Tauri (предпросмотр в браузере, тесты без мока) окна нет — остаётся перезапрос по `visibilitychange`.
      unlisten = Promise.resolve(null);
    }
    return () => {
      void unlisten.then((f) => f?.());
    };
  }, [refetchView, refetchCheck]);
  const openFile = useMutation({ mutationFn: (file: string) => api.openPath(path, joinProjectPath(path, file)) });
  const recheck = () => {
    void view.refetch();
    void check.refetch();
  };
  if (view.error) return <ErrorBanner error={view.error} onRetry={recheck} />;
  const v = view.data;
  if (!v) return <p className="empty">{t("common.loading")}</p>;
  // Пока отчёт не пришёл (или упал), показываем находки разбора из `suite_get`.
  const findings = check.data?.findings ?? v.findings;
  return (
    <div className="page">
      <div className="toolbar">
        <h2>
          {t("suite.title", { name: v.name })} <Hint k="hints.suite.title" label={t("suite.title", { name: v.name })} />
        </h2>
        <code className="muted">{v.file}</code>
        <button type="button" onClick={() => openFile.mutate(v.file)}>
          {t("suite.openFile")}
        </button>
        <button type="button" disabled={view.isFetching || check.isFetching} onClick={recheck}>
          {t("suite.check")}
        </button>
        <button type="button" onClick={() => setCreating(true)}>
          {t("suite.create")}
        </button>
      </div>
      {openFile.error && <ErrorBanner error={openFile.error} />}
      {creating && <CreateSuiteDialog path={path} onClose={() => setCreating(false)} />}
      <SuiteTree view={v} onOpenFile={() => openFile.mutate(v.file)} />
      <section>
        <h3>{t("suite.findings")}</h3>
        {check.error && <ErrorBanner error={check.error} onRetry={() => check.refetch()} />}
        <CheckReport findings={findings} onOpen={(file) => openFile.mutate(file)} />
      </section>
      {check.data?.coverage ? <CoverageTable coverage={check.data.coverage} profile={check.data.profile} /> : check.data && <p className="muted">{t("suite.noCoverage")}</p>}
    </div>
  );
}
