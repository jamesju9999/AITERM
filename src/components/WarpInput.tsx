import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { SubmitShortcut } from "../ipc/config";
import { useLocale } from "../contexts/LocaleContext";
import "./WarpInput.css";

export interface WarpInputProps {
  onSubmit: (cmd: string) => void;
  disabled?: boolean;
  shortcut?: SubmitShortcut;
}

const STORAGE_KEY = "aiterm-command-history";

/**
 * A block-based IDE-like input detached from the PTY.
 * Supports syntax highlighting (via future extension), multiline editing, etc.
 */
export function WarpInput({ onSubmit, disabled, shortcut = "enter" }: WarpInputProps) {
  const { t } = useLocale();
  const [value, setValue] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
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

  // Fill input from bookmark picker
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
    window.addEventListener("aiterm:fill-input", handler);
    return () => window.removeEventListener("aiterm:fill-input", handler);
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

  // newest on top
  const reversedHistory = [...history].reverse();

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
    // displayIdx → originalIdx in history array
    const originalIdx = history.length - 1 - displayIdx;
    const newHistory = history.filter((_, i) => i !== originalIdx);
    saveHistory(newHistory);
    if (newHistory.length === 0) {
      closeHistory(true);
    } else {
      const nextDisplay = Math.min(displayIdx, newHistory.length - 1);
      setDisplayIndex(nextDisplay);
      fillInput([...newHistory].reverse()[nextDisplay]);
    }
  };

  const clearAllHistory = (e: React.MouseEvent) => {
    e.stopPropagation();
    saveHistory([]);
    closeHistory(true);
    setTimeout(() => textareaRef.current?.focus(), 0);
  };

  const selectHistoryItem = (displayIdx: number) => {
    fillInput(reversedHistory[displayIdx]);
    setHistoryOpen(false);
    setDisplayIndex(-1);
    setTimeout(() => {
      textareaRef.current?.focus();
      if (textareaRef.current) {
        const len = reversedHistory[displayIdx].length;
        textareaRef.current.selectionStart = len;
        textareaRef.current.selectionEnd = len;
      }
    }, 0);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    let shouldSubmit = false;

    if (e.key === "Enter") {
      if (historyOpen) {
        // Input already has the selected text; just close popover
        e.preventDefault();
        closeHistory(false);
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
        closeHistory(true); // restore draft on Escape
      }
    } else if (e.key === "Delete" && historyOpen && displayIndex >= 0) {
      e.preventDefault();
      deleteHistoryItem(displayIndex, { stopPropagation: () => {} } as React.MouseEvent);
      return;
    } else if (e.key === "ArrowUp") {
      if (history.length === 0) return;
      e.preventDefault();
      if (!historyOpen) {
        // Save draft before starting navigation
        draftValueRef.current = value;
        setHistoryOpen(true);
        setDisplayIndex(0);
        fillInput(reversedHistory[0]);
      } else {
        // Move DOWN in list (toward older items at bottom)
        const next = Math.min(displayIndex + 1, reversedHistory.length - 1);
        if (next !== displayIndex) {
          setDisplayIndex(next);
          fillInput(reversedHistory[next]);
        }
      }
    } else if (e.key === "ArrowDown") {
      if (!historyOpen) return;
      e.preventDefault();
      if (displayIndex > 0) {
        // Move UP in list (toward newer items at top)
        const prev = displayIndex - 1;
        setDisplayIndex(prev);
        fillInput(reversedHistory[prev]);
      } else {
        // Already at top (newest) — close and restore draft
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
      // User started typing — exit history navigation, discard selection
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
            <span className="warp-history-title">歷史記錄</span>
            <button
              className="warp-history-clear-all"
              onClick={clearAllHistory}
              title="清除全部歷史記錄"
            >
              清除全部
            </button>
          </div>
          <div ref={itemsRef} className="warp-history-items">
            {reversedHistory.map((cmd, displayIdx) => (
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
                  title="刪除此記錄"
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
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
      />
    </div>
  );
}
