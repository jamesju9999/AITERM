import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
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

interface DropdownPos {
  top?: number;
  bottom?: number;
  left?: number;
  right?: number;
  minWidth: number;
}

export function ModelPickerButton({ providers, selectedId, onChange, disabled }: Props) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<DropdownPos | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const selected = providers.find((p) => p.id === selectedId) ?? null;

  const handleToggle = () => {
    if (disabled) return;
    if (open) { setOpen(false); return; }

    const rect = buttonRef.current?.getBoundingClientRect();
    if (!rect) return;
    const viewH = window.innerHeight;

    const newPos: DropdownPos = { minWidth: Math.max(rect.width, 260) };
    // Open above if there's more space there, otherwise below
    if (rect.top > viewH - rect.bottom) {
      newPos.bottom = viewH - rect.top + 6;
    } else {
      newPos.top = rect.bottom + 6;
    }
    // Default: left-align with button; overflow check runs after render
    newPos.left = rect.left;

    setPos(newPos);
    setOpen(true);
  };

  // After dropdown renders: flip to right-aligned if it overflows right edge
  useEffect(() => {
    if (!open || !dropdownRef.current || !buttonRef.current) return;
    const dr = dropdownRef.current.getBoundingClientRect();
    if (dr.right > window.innerWidth - 8) {
      const br = buttonRef.current.getBoundingClientRect();
      setPos((p) => p ? { ...p, left: undefined, right: window.innerWidth - br.right } : p);
    }
  }, [open]);

  // Close on outside click or any scroll
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!buttonRef.current?.contains(e.target as Node) &&
          !dropdownRef.current?.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onScroll = () => setOpen(false);
    document.addEventListener("mousedown", close);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("mousedown", close);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  const dropdownStyle: React.CSSProperties = {
    position: "fixed",
    zIndex: 9999,
    minWidth: pos?.minWidth,
    ...(pos?.top    !== undefined ? { top:    pos.top    } : {}),
    ...(pos?.bottom !== undefined ? { bottom: pos.bottom } : {}),
    ...(pos?.left   !== undefined ? { left:   pos.left   } : {}),
    ...(pos?.right  !== undefined ? { right:  pos.right  } : {}),
  };

  return (
    <div className="model-picker-wrap">
      <button
        ref={buttonRef}
        disabled={disabled}
        className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm model-picker-btn"
        onClick={handleToggle}
      >
        <RobotIcon size={13} style={{ color: selected ? "var(--accent, #a855f7)" : "#666", flexShrink: 0 }} />
        <span className="model-picker-btn__label">
          {selected ? selected.display_name : t.model_picker_no_provider}
        </span>
      </button>
      {open && pos && createPortal(
        <div ref={dropdownRef} className="model-picker-dropdown" style={dropdownStyle}>
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
        </div>,
        document.body
      )}
    </div>
  );
}
