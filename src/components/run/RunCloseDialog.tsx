import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import { isActive, useRunStore } from "../../state/run";
import { ConfirmDialog } from "../common/ConfirmDialog";
import { ErrorBanner } from "../common/ErrorBanner";

/** Событие Tauri: окно закрывают при идущем прогоне (`lib::RUN_CLOSE_REQUESTED`), закрытие уже отменено бэкендом. */
export const RUN_CLOSE_REQUESTED = "run-close-requested";
/** Сколько ждём, пока остановленные прогоны допишут итог, прежде чем закрыть окно всё равно. */
export const STOP_WAIT_MS = 30_000;

/** Ждёт, пока прогоны `paths` перестанут быть активными в сторе, но не дольше `ms`. */
function waitStopped(paths: string[], ms: number): Promise<void> {
  return new Promise((resolve) => {
    const idle = () => paths.every((p) => !isActive(useRunStore.getState().byProject[p]?.status ?? "idle"));
    if (idle()) return resolve();
    const finish = () => {
      clearTimeout(timer);
      unsubscribe();
      resolve();
    };
    const timer = setTimeout(finish, ms);
    const unsubscribe = useRunStore.subscribe(() => {
      if (idle()) finish();
    });
  });
}

/**
 * Окно закрывают во время прогона (спека L2 §7.1, §6.3): остановить и закрыть, закрыть (агенты доработают сами,
 * папка достроится при следующем открытии проекта) или отменить. Смонтирован в `Layout` всегда: вкладку прогона
 * могут закрыть, а бэкенд не даст закрыть окно, пока прогон идёт, — этот диалог единственный выход.
 */
export function RunCloseDialog() {
  const { t } = useTranslation();
  const requested = useRunStore((s) => s.closeRequested);
  const setRequested = useRunStore((s) => s.setCloseRequested);
  const stop = useRunStore((s) => s.stop);
  const [error, setError] = useState<unknown>(null);
  const [stopping, setStopping] = useState(false);
  // Номер попытки: «Отмена» во время остановки не даёт закрыть окно, когда прогон всё же остановится.
  const attempt = useRef(0);
  useEffect(() => {
    // Вне Tauri (тесты без мока, предпросмотр в браузере) подписаться не на что — закрытию окна там ничто не мешает.
    const unlisten = listen(RUN_CLOSE_REQUESTED, () => useRunStore.getState().setCloseRequested(true)).catch(() => null);
    return () => {
      void unlisten.then((f) => f?.());
    };
  }, []);
  if (!requested) return null;
  const close = async (stopFirst: boolean) => {
    const mine = ++attempt.current;
    setError(null);
    try {
      if (stopFirst) {
        const running = Object.entries(useRunStore.getState().byProject)
          .filter(([, r]) => isActive(r.status))
          .map(([path]) => path);
        setStopping(true);
        await Promise.all(running.map((path) => stop(path)));
        // `run_stop` только отменяет прогон: ждём, пока контроллер остановит агентов и допишет папку.
        await waitStopped(running, STOP_WAIT_MS);
      }
      if (mine === attempt.current) await api.confirmClose();
    } catch (e) {
      if (mine === attempt.current) setError(e);
    } finally {
      if (mine === attempt.current) setStopping(false);
    }
  };
  return (
    <ConfirmDialog
      title={t("run.close.title")}
      message={t("run.close.message")}
      actions={[
        { label: t("run.close.stop"), onClick: () => void close(true), primary: true, disabled: stopping },
        { label: t("run.close.leave"), onClick: () => void close(false) },
      ]}
      onCancel={() => {
        attempt.current++;
        setStopping(false);
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
