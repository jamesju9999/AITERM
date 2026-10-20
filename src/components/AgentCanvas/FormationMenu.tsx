import { useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { FormationsState } from "../../ipc/formations";

type Editing = { kind: "new" | "save_as" | "rename"; value: string } | null;

/**
 * 團隊編制選單：一個專案可以存好幾組團隊，依階段切換（盤點 → 轉換 → 實測）。
 * 名稱用選單裡的輸入框（Tauri 的 webview 沒有 window.prompt）。
 * 見 docs/superpowers/specs/2026-10-20-agent-canvas-team-formations-design.md。
 */
export function FormationMenu({
  state,
  onSwitch,
  onCreate,
  onSaveAs,
  onRename,
  onDelete,
  onCancelPending,
}: {
  state: FormationsState;
  onSwitch: (id: string) => void;
  onCreate: (name: string) => void;
  onSaveAs: (name: string) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string, name: string) => void;
  onCancelPending: () => void;
}) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<Editing>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const active = state.formations.find((f) => f.id === state.active);
  const pending = state.formations.find((f) => f.id === state.pending);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) {
        setOpen(false);
        setEditing(null);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const close = () => {
    setOpen(false);
    setEditing(null);
  };

  const submit = () => {
    if (!editing) return;
    const name = editing.value.trim();
    if (!name) return;
    if (editing.kind === "new") onCreate(name);
    else if (editing.kind === "save_as") onSaveAs(name);
    else if (active) onRename(active.id, name);
    close();
  };

  return (
    <>
      <div className="ac-role-menu-wrap" ref={wrapRef}>
        <button
          className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
          data-testid="ac-formation"
          title={t.ac_form_tip}
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => (open ? close() : setOpen(true))}
        >
          {t.ac_form_button(active?.name ?? t.ac_form_none)} ▾
        </button>
        {open && (
          <div className="ac-role-menu ac-form-menu" role="menu" aria-label={t.ac_form_label}>
            <div className="ac-role-menu-label">{t.ac_form_label}</div>
            {state.formations.map((f) => (
              <div key={f.id} className="ac-role-menu-row">
                <button
                  type="button"
                  role="menuitem"
                  className="ac-role-menu-item"
                  onClick={() => {
                    close();
                    if (f.id !== state.active) onSwitch(f.id);
                  }}
                >
                  <span className="ac-form-check" aria-hidden>
                    {f.id === state.active ? "✓" : ""}
                  </span>
                  <span className="ac-form-name">{f.name}</span>
                  <small className="ac-role-menu-item--muted">{t.ac_form_count(f.agent_count)}</small>
                </button>
                {f.id !== state.active && (
                  <button
                    type="button"
                    className="ac-role-menu-item ac-role-menu-item--muted ac-form-del"
                    aria-label={t.ac_form_delete(f.name)}
                    title={t.ac_form_delete(f.name)}
                    onClick={() => {
                      close();
                      onDelete(f.id, f.name);
                    }}
                  >
                    ×
                  </button>
                )}
              </div>
            ))}
            <div className="ac-role-menu-sep" />
            {editing ? (
              <input
                className="task-field-input ac-form-input"
                autoFocus
                value={editing.value}
                placeholder={t.ac_form_name_placeholder}
                aria-label={t.ac_form_name_placeholder}
                onChange={(e) => setEditing({ ...editing, value: e.target.value })}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.nativeEvent.isComposing) submit();
                  if (e.key === "Escape") setEditing(null);
                }}
              />
            ) : (
              <>
                <button type="button" role="menuitem" className="ac-role-menu-item" onClick={() => setEditing({ kind: "new", value: "" })}>
                  {t.ac_form_new}
                </button>
                <button type="button" role="menuitem" className="ac-role-menu-item" onClick={() => setEditing({ kind: "save_as", value: "" })}>
                  {t.ac_form_save_as}
                </button>
                {active && (
                  <button
                    type="button"
                    role="menuitem"
                    className="ac-role-menu-item"
                    onClick={() => setEditing({ kind: "rename", value: active.name })}
                  >
                    {t.ac_form_rename}
                  </button>
                )}
              </>
            )}
          </div>
        )}
      </div>
      {pending && (
        <span className="ac-form-pending" data-testid="ac-formation-pending">
          {t.ac_form_pending(pending.name, state.busy.join("、"))}
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onCancelPending}>
            {t.ac_form_cancel_pending}
          </button>
        </span>
      )}
    </>
  );
}
