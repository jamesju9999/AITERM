import { useEffect, useRef, useState } from "react";
import { listProviders, setDefaultProvider, type ProviderInfo } from "../ipc/provider";
import { PROVIDER_TYPE_LABELS } from "../ipc/provider";
import "./ProviderPalette.css";

interface ProviderPaletteProps {
  onClose: () => void;
  onSwitch: (displayName: string) => void;
}

export function ProviderPalette({ onClose, onSwitch }: ProviderPaletteProps) {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [cursor, setCursor] = useState(0);
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const defaultIdx = list.findIndex((p) => p.is_default);
      if (defaultIdx >= 0) setCursor(defaultIdx);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setCursor((c) => Math.min(c + 1, providers.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => Math.max(c - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        selectProvider(providers[cursor]);
      }
    };
    document.addEventListener("keydown", handler, true);
    return () => document.removeEventListener("keydown", handler, true);
  }, [providers, cursor, onClose]);

  // Scroll cursor into view
  useEffect(() => {
    const el = listRef.current?.children[cursor] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  function selectProvider(p: ProviderInfo) {
    if (!p) return;
    setDefaultProvider(p.id)
      .then(() => {
        onSwitch(p.display_name);
        onClose();
      })
      .catch(() => onClose());
  }

  return (
    <div className="aiterm-palette-backdrop" onClick={onClose}>
      <div
        className="aiterm-palette"
        role="dialog"
        aria-label="Switch AI Provider"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="aiterm-palette__title">Switch Provider</div>
        {providers.length === 0 ? (
          <div className="aiterm-palette__empty">No providers configured.</div>
        ) : (
          <ul ref={listRef} className="aiterm-palette__list">
            {providers.map((p, idx) => (
              <li
                key={p.id}
                className={[
                  "aiterm-palette__item",
                  idx === cursor ? "aiterm-palette__item--focused" : "",
                  p.is_default ? "aiterm-palette__item--active" : "",
                ].join(" ")}
                onClick={() => selectProvider(p)}
                onMouseEnter={() => setCursor(idx)}
              >
                <span className="aiterm-palette__check">{p.is_default ? "✓" : " "}</span>
                <span className="aiterm-palette__name">{p.display_name}</span>
                <span className="aiterm-palette__meta">
                  {PROVIDER_TYPE_LABELS[p.provider_type]} · {p.model}
                </span>
              </li>
            ))}
          </ul>
        )}
        <div className="aiterm-palette__hint">↑↓ 選擇 · Enter 確認 · Esc 關閉</div>
      </div>
    </div>
  );
}
