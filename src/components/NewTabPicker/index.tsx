import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  onSelect: (type: "terminal" | "database" | "design" | "cross-db" | "vcs") => void;
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
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("design"); onClose(); }}
      >
        <span className="new-tab-picker__icon">📝</span>
        <div>
          <div className="new-tab-picker__label">設計與規格 (Design)</div>
          <div className="new-tab-picker__desc">進行需求討論與 SDD 規劃</div>
        </div>
      </button>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("cross-db"); onClose(); }}
      >
        <span className="new-tab-picker__icon">🔗</span>
        <div>
          <div className="new-tab-picker__label">{t.cross_db_tab}</div>
          <div className="new-tab-picker__desc">{t.new_cross_db_desc}</div>
        </div>
      </button>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("vcs"); onClose(); }}
      >
        <span className="new-tab-picker__icon">🔀</span>
        <div>
          <div className="new-tab-picker__label">{t.vcs_tab}</div>
          <div className="new-tab-picker__desc">{t.new_vcs_desc}</div>
        </div>
      </button>
    </div>
  );
}
