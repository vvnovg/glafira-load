import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Project } from "../../api/types";
import { isActive, type ProjectRun, useRunStore } from "../../state/run";
import { ConfirmDialog } from "../common/ConfirmDialog";
import { ErrorBanner } from "../common/ErrorBanner";

/**
 * Событие Tauri: окно закрывают или выходят из приложения (Cmd+Q) при идущем прогоне (`lib::RUN_CLOSE_REQUESTED`),
 * закрытие уже отменено бэкендом.
 */
export const RUN_CLOSE_REQUESTED = "run-close-requested";

/**
 * Прогон идёт на встроенном агенте внутри приложения и прервётся вместе с ним: смок или стенд без агентов.
 * Стенд берётся из манифеста в кэше запросов (его загрузила форма прогона); не известен — считаем, что агенты есть.
 */
function runsInsideApp(qc: QueryClient, path: string, run: ProjectRun): boolean {
  if (!run.params) return false;
  if (run.params.mode === "smoke") return true;
  const env = qc.getQueryData<Project>(["project", path])?.manifest.environments.find((e) => e.id === run.params?.env);
  return env !== undefined && (env.agents?.length ?? 0) === 0;
}

/**
 * Окно закрывают во время прогона (спека L2 §7.1, §6.3): остановить и закрыть, закрыть (агенты доработают сами,
 * папка достроится при следующем открытии проекта) или отменить. Смонтирован в `Layout` всегда: вкладку прогона
 * могут закрыть, а бэкенд не даст закрыть окно, пока прогон идёт, — этот диалог единственный выход.
 * «Остановить и закрыть» — `confirm_close(true)`: бэкенд сам останавливает все прогоны и ждёт (не дольше 30 с),
 * пока контроллеры допишут папки; после этого отменить закрытие уже нельзя.
 * Все идущие прогоны — на встроенном агенте: «закрыть» прерывает прогон, а не оставляет его агентам.
 */
export function RunCloseDialog() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const requested = useRunStore((s) => s.closeRequested);
  const setRequested = useRunStore((s) => s.setCloseRequested);
  const inside = useRunStore((s) => {
    const active = Object.entries(s.byProject).filter(([, r]) => isActive(r.status));
    return active.length > 0 && active.every(([path, r]) => runsInsideApp(qc, path, r));
  });
  const [error, setError] = useState<unknown>(null);
  const [stopping, setStopping] = useState(false);
  // Номер попытки: ошибка «закрыть» после «Отмены» не всплывает в уже закрытом диалоге.
  const attempt = useRef(0);
  useEffect(() => {
    // Вне Tauri (тесты без мока, предпросмотр в браузере) подписаться не на что — закрытию окна там ничто не мешает.
    const unlisten = listen(RUN_CLOSE_REQUESTED, () => useRunStore.getState().setCloseRequested(true)).catch(() => null);
    return () => {
      void unlisten.then((f) => f?.());
    };
  }, []);
  if (!requested) return null;
  const close = async (stop: boolean) => {
    const mine = ++attempt.current;
    setError(null);
    setStopping(stop);
    try {
      // При успехе приложение завершается и ответа не будет.
      await api.confirmClose(stop);
    } catch (e) {
      if (mine === attempt.current) {
        setError(e);
        setStopping(false);
      }
    }
  };
  return (
    <ConfirmDialog
      title={t("run.close.title")}
      message={t(inside ? "run.close.messageInside" : "run.close.message")}
      actions={[
        { label: t("run.close.stop"), onClick: () => void close(true), primary: true, disabled: stopping },
        { label: t(inside ? "run.close.leaveInside" : "run.close.leave"), onClick: () => void close(false), disabled: stopping },
      ]}
      cancelDisabled={stopping}
      onCancel={() => {
        attempt.current++;
        setError(null);
        setRequested(false);
      }}
    >
      {stopping && (
        <p className="muted" role="status">
          {t("run.close.stopping")}
        </p>
      )}
      {error ? <ErrorBanner error={error} /> : null}
    </ConfirmDialog>
  );
}
