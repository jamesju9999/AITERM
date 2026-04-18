import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  onSelect: (type: "terminal" | "database") => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const { t } = useLocale();

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [onClose]);

  return (
    <div className="new-tab-picker" ref={ref}>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("terminal"); onClose(); }}
      >
        <span className="new-tab-picker__icon">⬛</span>
        <div>
          <div className="new-tab-picker__label">{t.terminal_tab}</div>
          <div className="new-tab-picker__desc">{t.new_terminal_desc}</div>
        </div>
      </button>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("database"); onClose(); }}
      >
        <span className="new-tab-picker__icon">🗄️</span>
        <div>
          <div className="new-tab-picker__label">{t.database_tab}</div>
          <div className="new-tab-picker__desc">{t.new_database_desc}</div>
        </div>
      </button>
    </div>
  );
}
