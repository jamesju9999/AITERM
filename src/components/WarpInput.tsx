import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { SubmitShortcut } from "../ipc/config";
import { useLocale } from "../contexts/LocaleContext";
import { listDirectory, type DirEntry } from "../ipc/fs";
import "./WarpInput.css";

export interface WarpInputProps {
  onSubmit: (cmd: string) => void;
  disabled?: boolean;
  shortcut?: SubmitShortcut;
  sessionId?: string;
}

const STORAGE_KEY = "aiterm-command-history";

export function WarpInput({ onSubmit, disabled, shortcut = "enter", sessionId }: WarpInputProps) {
  const { t } = useLocale();
  const [value, setValue] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [dirPickerOpen, setDirPickerOpen] = useState(false);
  const [dirEntries, setDirEntries] = useState<DirEntry[] | null>(null);
  // displayIndex: position in reversedHistory (0 = top = newest, length-1 = bottom = oldest)
  const [displayIndex, setDisplayIndex] = useState(-1);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const itemsRef = useRef<HTMLDivElement>(null);
  // Saves the input draft before the user starts navigating history
  const draftValueRef = useRef("");

  useEffect(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) setHistory(JSON.parse(stored));
    } catch {
      // ignore
    }
  }, []);

  useEffect(() => {
    const handler = (e: Event) => {
      const { cmd } = (e as CustomEvent).detail as { cmd: string };
      fillInput(cmd);
      setTimeout(() => {
        textareaRef.current?.focus();
        if (textareaRef.current) {
          textareaRef.current.selectionStart = cmd.length;
          textareaRef.current.selectionEnd = cmd.length;
        }
      }, 0);
    };
    window.addEventListener("warp-fill-command", handler);
    return () => window.removeEventListener("warp-fill-command", handler);
  }, []);

  useEffect(() => {
    if (!disabled && !historyOpen) {
      textareaRef.current?.focus();
    }
  }, [disabled, historyOpen]);

  // Scroll active item into view — displayIndex maps directly to children order
  useEffect(() => {
    if (historyOpen && displayIndex >= 0 && itemsRef.current) {
      const el = itemsRef.current.children[displayIndex] as HTMLElement;
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [displayIndex, historyOpen]);

  // oldest at top, newest at bottom — so ↑ moves lightbar UP (toward older) and ↓ moves DOWN
  const displayHistory = history;

  const fillInput = (text: string) => {
    setValue(text);
    requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.style.height = "auto";
        textareaRef.current.style.height = `${textareaRef.current.scrollHeight}px`;
      }
    });
  };

  const closeHistory = (restoreDraft = false) => {
    setHistoryOpen(false);
    setDisplayIndex(-1);
    if (restoreDraft) fillInput(draftValueRef.current);
  };

  const saveHistory = (newHistory: string[]) => {
    setHistory(newHistory);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(newHistory));
  };

  const commitCommand = (cmd: string) => {
    if (!cmd.trim()) return;
    let newHistory = history.filter((h) => h !== cmd);
    newHistory.push(cmd);
    if (newHistory.length > 100) newHistory = newHistory.slice(newHistory.length - 100);
    saveHistory(newHistory);
    onSubmit(cmd);
    setValue("");
    setHistoryOpen(false);
    setDisplayIndex(-1);
    if (textareaRef.current) textareaRef.current.style.height = "auto";
  };

  const deleteHistoryItem = (displayIdx: number, e: React.MouseEvent) => {
    e.stopPropagation();
    // displayIdx maps directly to history array index
    const newHistory = history.filter((_, i) => i !== displayIdx);
    saveHistory(newHistory);
    if (newHistory.length === 0) {
      closeHistory(true);
    } else {
      const nextDisplay = Math.min(displayIdx, newHistory.length - 1);
      setDisplayIndex(nextDisplay);
      fillInput(newHistory[nextDisplay]);
    }
  };

  const clearAllHistory = (e: React.MouseEvent) => {
    e.stopPropagation();
    saveHistory([]);
    closeHistory(true);
    setTimeout(() => textareaRef.current?.focus(), 0);
  };

  const toggleDirPicker = () => {
    setHistoryOpen(false);
    setDirPickerOpen((open) => {
      const next = !open;
      if (next) {
        setDirEntries(null);
        if (sessionId) {
          listDirectory(sessionId, "")
            .then((entries) => setDirEntries(entries.filter((e) => e.is_dir)))
            .catch(() => setDirEntries([]));
        } else {
          setDirEntries([]);
        }
      }
      return next;
    });
  };

  const selectDirEntry = (entry: DirEntry) => {
    setDirPickerOpen(false);
    commitCommand(`cd "${entry.name}"`);
  };

  const selectParentDir = () => {
    setDirPickerOpen(false);
    commitCommand("cd ..");
  };

  const selectHistoryItem = (displayIdx: number) => {
    fillInput(displayHistory[displayIdx]);
    closeHistory(false);
    setTimeout(() => {
      textareaRef.current?.focus();
      if (textareaRef.current) {
        const len = displayHistory[displayIdx].length;
        textareaRef.current.selectionStart = len;
        textareaRef.current.selectionEnd = len;
      }
    }, 0);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    let shouldSubmit = false;

    if (e.key === "Enter") {
      if (historyOpen && displayIndex >= 0) {
        e.preventDefault();
        selectHistoryItem(displayIndex);
        return;
      }

      if (shortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        shouldSubmit = true;
      } else if (shortcut === "shift-enter" && e.shiftKey && !e.ctrlKey && !e.metaKey) {
        shouldSubmit = true;
      } else if (shortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey)) {
        shouldSubmit = true;
      }
    } else if (e.key === "Escape") {
      if (historyOpen) {
        e.preventDefault();
        closeHistory(true);
      }
      if (dirPickerOpen) {
        e.preventDefault();
        setDirPickerOpen(false);
      }
    } else if (e.key === "Delete" && historyOpen && displayIndex >= 0) {
      e.preventDefault();
      deleteHistoryItem(displayIndex, { stopPropagation: () => {} } as React.MouseEvent);
      return;
    } else if (e.key === "ArrowUp") {
      if (history.length === 0) return;
      e.preventDefault();
      setDirPickerOpen(false);
      if (!historyOpen) {
        // Save draft, open popover at bottom (newest item)
        draftValueRef.current = value;
        const startIdx = displayHistory.length - 1;
        setHistoryOpen(true);
        setDisplayIndex(startIdx);
        fillInput(displayHistory[startIdx]);
      } else if (displayIndex > 0) {
        // Move UP toward older items
        const prev = displayIndex - 1;
        setDisplayIndex(prev);
        fillInput(displayHistory[prev]);
      }
      // Already at top (oldest) — stay
    } else if (e.key === "ArrowDown") {
      if (!historyOpen) return;
      e.preventDefault();
      if (displayIndex < displayHistory.length - 1) {
        // Move DOWN toward newer items
        const next = displayIndex + 1;
        setDisplayIndex(next);
        fillInput(displayHistory[next]);
      } else {
        // Already at bottom (newest) — close and restore draft
        closeHistory(true);
      }
    }

    if (shouldSubmit) {
      e.preventDefault();
      commitCommand(value);
    }
  };

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setValue(e.target.value);
    if (historyOpen) {
      // User started typing — exit history navigation
      setHistoryOpen(false);
      setDisplayIndex(-1);
    }
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = `${textareaRef.current.scrollHeight}px`;
    }
  };

  return (
    <div className="warp-input-container">
      {historyOpen && history.length > 0 && (
        <div className="warp-history-popover">
          <div className="warp-history-header">
            <span className="warp-history-title">{t.warp_history_title}</span>
            <button
              className="warp-history-clear-all"
              onClick={clearAllHistory}
              title={t.warp_clear_all_title}
            >
              {t.warp_clear_all}
            </button>
          </div>
          <div ref={itemsRef} className="warp-history-items">
            {displayHistory.map((cmd, displayIdx) => (
              <div
                key={displayIdx}
                className={`warp-history-item ${displayIdx === displayIndex ? "selected" : ""}`}
                onClick={() => selectHistoryItem(displayIdx)}
              >
                <span className="warp-history-prefix">&gt;_</span>
                <span className="warp-history-text">{cmd}</span>
                <button
                  className="warp-history-delete"
                  onClick={(e) => deleteHistoryItem(displayIdx, e)}
                  title={t.warp_delete_item_title}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
      {dirPickerOpen && (
        <div className="warp-history-popover warp-dir-popover">
          <div className="warp-history-header">
            <span className="warp-history-title">{t.warp_dir_picker_title}</span>
          </div>
          <div className="warp-history-items">
            <div className="warp-history-item" onClick={selectParentDir}>
              <span className="warp-history-prefix">⬆️</span>
              <span className="warp-history-text">.. ({t.file_go_up})</span>
            </div>
            {dirEntries === null && (
              <div className="warp-dir-status">{t.warp_dir_picker_loading}</div>
            )}
            {dirEntries !== null && dirEntries.length === 0 && (
              <div className="warp-dir-status">{t.warp_dir_picker_empty}</div>
            )}
            {dirEntries?.map((entry) => (
              <div
                key={entry.path}
                className="warp-history-item"
                onClick={() => selectDirEntry(entry)}
              >
                <span className="warp-history-prefix">📁</span>
                <span className="warp-history-text">{entry.name}</span>
              </div>
            ))}
          </div>
        </div>
      )}
      <button
        type="button"
        className="warp-dir-picker-btn"
        onClick={toggleDirPicker}
        title={t.warp_dir_picker_tooltip}
        disabled={!sessionId}
      >
        📁
      </button>
      <div className="warp-input-prompt">▶</div>
      <textarea
        ref={textareaRef}
        className="warp-input-textarea"
        value={value}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        placeholder={t.input_placeholder(shortcut === "enter" ? "Enter" : shortcut === "shift-enter" ? "Shift+Enter" : "Ctrl+Enter")}
        rows={1}
        disabled={disabled}
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
      />
      <button
        type="button"
        className="warp-send-btn"
        onClick={() => commitCommand(value)}
        disabled={disabled || !value.trim()}
        title={t.warp_send_title}
        aria-label={t.warp_send_title}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
          <path d="M12 20V5M12 5l-6 6M12 5l6 6" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
    </div>
  );
}
