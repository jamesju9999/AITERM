import { useEffect, useRef, useState } from "react";
import "./ModelCombo.css";

interface Props {
  value: string;
  options: string[];
  onChange: (value: string) => void;
  placeholder?: string;
}

const LIST_MAX_HEIGHT = 240;

/**
 * 模型下拉。原生 <select> 的彈出清單是作業系統畫的：在 Linux（WebKitGTK）貼近螢幕底部時會被切掉，
 * 下面的選項點不到。這個清單畫在頁面裡（可捲動，空間不夠就往上開），而且輸入框本身可以直接打模型名稱。
 */
export function ModelCombo({ value, options, onChange, placeholder }: Props) {
  const [open, setOpen] = useState(false);
  const [up, setUp] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrap.current && !wrap.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const toggle = () => {
    if (!open && wrap.current) {
      const r = wrap.current.getBoundingClientRect();
      setUp(window.innerHeight - r.bottom < LIST_MAX_HEIGHT && r.top > window.innerHeight - r.bottom);
    }
    setOpen(o => !o);
  };

  return (
    <div className="model-combo" ref={wrap}>
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={e => onChange(e.target.value)}
        onKeyDown={e => e.key === "Escape" && setOpen(false)}
      />
      <button type="button" className="model-combo__toggle" aria-label="models" aria-expanded={open} onClick={toggle} />
      {open && (
        <ul className={`model-combo__list${up ? " model-combo__list--up" : ""}`} role="listbox" style={{ maxHeight: LIST_MAX_HEIGHT }}>
          {options.map(m => (
            <li
              key={m}
              role="option"
              aria-selected={m === value}
              className={m === value ? "is-selected" : undefined}
              onClick={() => {
                onChange(m);
                setOpen(false);
              }}
            >
              {m}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
