import type { ReactNode } from "react";
import "./styles.css";

interface CloseConfirmDialogProps {
  title: string;
  /** 允許多行／`<br />`，沿用 LoopStudio 既有的內文寫法。 */
  body: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 分頁關閉確認框。純呈現：不知道 close guard 存在，也不持有狀態，
 * 由呼叫端決定何時掛載、按鈕按下後要 resolve 成什麼。
 */
export function CloseConfirmDialog({
  title,
  body,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onCancel,
}: CloseConfirmDialogProps) {
  return (
    <div className="aiterm-close-overlay">
      <div className="aiterm-close-dialog">
        <h3 className="aiterm-close-dialog-title">{title}</h3>
        <p className="aiterm-close-dialog-body">{body}</p>
        <div className="aiterm-close-dialog-actions">
          <button
            type="button"
            className="aiterm-btn aiterm-btn--secondary"
            onClick={onCancel}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className="aiterm-btn aiterm-btn--danger-solid"
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
