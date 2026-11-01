import { type ReactNode, useEffect, useId, useRef } from "react";
import { useTranslation } from "react-i18next";
import { trapTab } from "../../lib/focusTrap";

export interface DialogAction { label: string; onClick: () => void; primary?: boolean; disabled?: boolean }

/**
 * Модальный вопрос с кнопками действий и «Отмена»; Esc и клик по фону — отмена, Tab ходит по кругу.
 * Фокус при открытии уходит в диалог, при закрытии возвращается к элементу, который был в фокусе.
 */
export function ConfirmDialog({ title, message, actions, onCancel, children }: { title: string; message: string; actions: DialogAction[]; onCancel: () => void; children?: ReactNode }) {
  const { t } = useTranslation();
  const root = useRef<HTMLDivElement>(null);
  const uid = useId();
  useEffect(() => {
    const opener = document.activeElement;
    root.current?.focus();
    return () => {
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  }, []);
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div
        ref={root}
        className="modal"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${uid}-title`}
        aria-describedby={`${uid}-message`}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
          else if (e.key === "Tab") trapTab(root.current, e);
        }}
      >
        <h3 id={`${uid}-title`}>{title}</h3>
        <p id={`${uid}-message`}>{message}</p>
        {children}
        <div className="row">
          {actions.map((a) => (
            <button key={a.label} type="button" className={a.primary ? "primary" : undefined} disabled={a.disabled} onClick={a.onClick}>
              {a.label}
            </button>
          ))}
          <button type="button" onClick={onCancel}>
            {t("common.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}
