import { useEffect, useRef } from "react";
import "./index.css";

interface Props {
  onSelect: (type: "terminal" | "database") => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

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
          <div className="new-tab-picker__label">終端機</div>
          <div className="new-tab-picker__desc">開啟新 Shell Session</div>
        </div>
      </button>
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("database"); onClose(); }}
      >
        <span className="new-tab-picker__icon">🗄️</span>
        <div>
          <div className="new-tab-picker__label">資料庫</div>
          <div className="new-tab-picker__desc">連接資料庫並瀏覽</div>
        </div>
      </button>
    </div>
  );
}
