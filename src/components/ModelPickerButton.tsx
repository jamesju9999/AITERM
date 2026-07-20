import { useEffect, useRef, useState } from "react";
import type { ProviderInfo } from "../ipc/provider";
import { RobotIcon } from "./Icons";
import { useLocale } from "../contexts/LocaleContext";
import "./ModelPickerButton.css";

interface Props {
  providers: ProviderInfo[];
  selectedId: string;
  onChange: (id: string) => void;
  disabled?: boolean;
}

export function ModelPickerButton({ providers, selectedId, onChange, disabled }: Props) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const selected = providers.find((p) => p.id === selectedId) ?? null;

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  // Auto-flip: if dropdown overflows the right edge of the viewport, switch to right-aligned
  useEffect(() => {
    if (!open || !dropdownRef.current) return;
    const rect = dropdownRef.current.getBoundingClientRect();
    if (rect.right > window.innerWidth - 8) {
      dropdownRef.current.style.left = "auto";
      dropdownRef.current.style.right = "0";
    }
  }, [open]);

  return (
    <div ref={wrapRef} className="model-picker-wrap">
      <button
        disabled={disabled}
        className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm model-picker-btn"
        onClick={() => !disabled && setOpen((o) => !o)}
      >
        <RobotIcon size={13} style={{ color: selected ? "var(--accent, #a855f7)" : "#666", flexShrink: 0 }} />
        <span className="model-picker-btn__label">
          {selected ? selected.display_name : t.model_picker_no_provider}
        </span>
      </button>
      {open && (
        <div ref={dropdownRef} className="model-picker-dropdown">
          {providers.map((p) => (
            <button
              key={p.id}
              className={`model-picker-dropdown__item${p.id === selectedId ? " model-picker-dropdown__item--active" : ""}`}
              onClick={() => { onChange(p.id); setOpen(false); }}
            >
              <span className="model-picker-dropdown__check">{p.id === selectedId ? "✓" : ""}</span>
              <span className="model-picker-dropdown__name">{p.display_name}</span>
              <span className="model-picker-dropdown__model">{p.model}</span>
            </button>
          ))}
          {providers.length === 0 && (
            <div className="model-picker-dropdown__empty">{t.model_picker_no_provider}</div>
          )}
        </div>
      )}
    </div>
  );
}
