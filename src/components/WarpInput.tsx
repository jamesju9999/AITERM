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
  const [historyIndex, setHistoryIndex] = useState(-1);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        setHistory(JSON.parse(stored));
      }
    } catch {
      // ignore
    }
  }, []);

  useEffect(() => {
    // Keep focus unless the user is explicitly interacting with something else
    if (!disabled && !historyOpen) {
      textareaRef.current?.focus();
    }
  }, [disabled, historyOpen]);

  // Scroll active item into view
  useEffect(() => {
    if (historyOpen && historyIndex >= 0 && popoverRef.current) {
      const activeEl = popoverRef.current.children[historyIndex] as HTMLElement;
      if (activeEl) {
        activeEl.scrollIntoView({ block: "nearest" });
      }
    }
  }, [historyIndex, historyOpen, history.length]);

  const commitCommand = (cmd: string) => {
    if (!cmd.trim()) return;
    
    // add to history
    let newHistory = history.filter((h) => h !== cmd);
    newHistory.push(cmd);
    if (newHistory.length > 100) newHistory = newHistory.slice(newHistory.length - 100);
    setHistory(newHistory);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(newHistory));

    onSubmit(cmd);
    setValue("");
    setHistoryOpen(false);
    setHistoryIndex(-1);
    
    // Reset height
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const selectHistoryItem = (idx: number) => {
    if (idx >= 0 && idx < history.length) {
      setValue(history[idx]);
      setHistoryOpen(false);
      setHistoryIndex(-1);
      setTimeout(() => {
        if (textareaRef.current) {
          textareaRef.current.style.height = "auto";
          textareaRef.current.style.height = `${textareaRef.current.scrollHeight}px`;
          textareaRef.current.focus();
          // Move cursor to end
          textareaRef.current.selectionStart = textareaRef.current.value.length;
          textareaRef.current.selectionEnd = textareaRef.current.value.length;
        }
      }, 0);
    }
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    let shouldSubmit = false;

    if (e.key === "Enter") {
      if (historyOpen && historyIndex >= 0) {
        e.preventDefault();
        selectHistoryItem(historyIndex);
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
        setHistoryOpen(false);
        setHistoryIndex(-1);
      }
    } else if (e.key === "ArrowUp") {
      // Check if cursor is at the very line 0, or if history is open
      // Simplified: if history has items, ArrowUp opens/navigates
      if (history.length > 0) {
        e.preventDefault();
        if (!historyOpen) {
          setHistoryOpen(true);
          setHistoryIndex(history.length - 1);
        } else {
          setHistoryIndex((prev) => Math.max(0, prev - 1));
        }
      }
    } else if (e.key === "ArrowDown") {
      if (historyOpen) {
        e.preventDefault();
        if (historyIndex < history.length - 1) {
          setHistoryIndex((prev) => prev + 1);
        } else {
          setHistoryOpen(false);
          setHistoryIndex(-1);
        }
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
      setHistoryOpen(false);
    }
    // Auto-resize vertically
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = `${textareaRef.current.scrollHeight}px`;
    }
  };

  return (
    <div className="warp-input-container">
      {historyOpen && history.length > 0 && (
        <div className="warp-history-popover" ref={popoverRef}>
          {history.map((cmd, idx) => (
            <div
              key={idx}
              className={`warp-history-item ${idx === historyIndex ? 'selected' : ''}`}
              onClick={() => selectHistoryItem(idx)}
            >
              <span className="warp-history-prefix">&gt;_</span>
              <span className="warp-history-text">{cmd}</span>
            </div>
          ))}
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
