import { useState, useEffect, useRef } from "react";
import { useLocale } from "../contexts/LocaleContext";
import "./CommandBookmarks.css";

export interface CommandBookmark {
  id: string;
  name: string;
  command: string;
  tags?: string;
}

const STORAGE_KEY = "aiterm-command-bookmarks";

export function loadBookmarks(): CommandBookmark[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored ? JSON.parse(stored) : [];
  } catch {
    return [];
  }
}

export function saveBookmarks(bms: CommandBookmark[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(bms));
}

export function addBookmark(command: string, name?: string): CommandBookmark {
  const bms = loadBookmarks();
  const bm: CommandBookmark = {
    id: crypto.randomUUID(),
    name: name || command.slice(0, 40),
    command,
  };
  const updated = [bm, ...bms].slice(0, 200);
  saveBookmarks(updated);
  return bm;
}

export function removeBookmark(id: string) {
  saveBookmarks(loadBookmarks().filter((b) => b.id !== id));
}

interface CommandBookmarksProps {
  onSelect: (command: string) => void;
  onClose: () => void;
}

export function CommandBookmarksPicker({ onSelect, onClose }: CommandBookmarksProps) {
  const { t } = useLocale();
  const [bookmarks, setBookmarks] = useState<CommandBookmark[]>([]);
  const [query, setQuery] = useState("");
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setBookmarks(loadBookmarks());
    setTimeout(() => inputRef.current?.focus(), 50);
  }, []);

  const filtered = bookmarks.filter((b) => {
    const q = query.toLowerCase();
    return (
      !q ||
      b.name.toLowerCase().includes(q) ||
      b.command.toLowerCase().includes(q) ||
      (b.tags || "").toLowerCase().includes(q)
    );
  });

  useEffect(() => {
    setActiveIdx(0);
  }, [query]);

  useEffect(() => {
    if (listRef.current && activeIdx >= 0) {
      const el = listRef.current.children[activeIdx] as HTMLElement;
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [activeIdx]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") { e.preventDefault(); onClose(); return; }
    if (e.key === "ArrowDown") { e.preventDefault(); setActiveIdx((i) => Math.min(i + 1, filtered.length - 1)); return; }
    if (e.key === "ArrowUp") { e.preventDefault(); setActiveIdx((i) => Math.max(0, i - 1)); return; }
    if (e.key === "Enter") {
      e.preventDefault();
      if (filtered[activeIdx]) onSelect(filtered[activeIdx].command);
      return;
    }
  };

  const deleteItem = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    removeBookmark(id);
    setBookmarks(loadBookmarks());
  };

  return (
    <div className="bookmarks-overlay" onClick={onClose}>
      <div className="bookmarks-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="bookmarks-header">
          <span className="bookmarks-title">{t.bookmarks_title}</span>
          <button className="bookmarks-close" onClick={onClose}>✕</button>
        </div>
        <input
          ref={inputRef}
          className="bookmarks-search"
          type="text"
          placeholder={t.bookmarks_search_placeholder}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
        />
        <div ref={listRef} className="bookmarks-list">
          {filtered.length === 0 ? (
            <div className="bookmarks-empty">
              {bookmarks.length === 0
                ? t.bookmarks_empty
                : t.bookmarks_no_match}
            </div>
          ) : (
            filtered.map((b, i) => (
              <div
                key={b.id}
                className={`bookmarks-item ${i === activeIdx ? "bookmarks-item--active" : ""}`}
                onClick={() => onSelect(b.command)}
                onMouseEnter={() => setActiveIdx(i)}
              >
                <div className="bookmarks-item-name">{b.name}</div>
                <div className="bookmarks-item-cmd">{b.command}</div>
                <button
                  className="bookmarks-item-delete"
                  onClick={(e) => deleteItem(b.id, e)}
                  title={t.bookmarks_delete_tip}
                >
                  ×
                </button>
              </div>
            ))
          )}
        </div>
        <div className="bookmarks-footer">
          <span>{t.bookmarks_hint}</span>
        </div>
      </div>
    </div>
  );
}
