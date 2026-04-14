import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { SubmitShortcut } from "../ipc/config";
import "./WarpInput.css";

export interface WarpInputProps {
  onSubmit: (cmd: string) => void;
  disabled?: boolean;
  shortcut?: SubmitShortcut;
}

/**
 * A block-based IDE-like input detached from the PTY.
 * Supports syntax highlighting (via future extension), multiline editing, etc.
 */
export function WarpInput({ onSubmit, disabled, shortcut = "enter" }: WarpInputProps) {
  const [value, setValue] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    // Keep focus unless the user is explicitly interacting with something else
    if (!disabled) {
      textareaRef.current?.focus();
    }
  }, [disabled]);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    let shouldSubmit = false;

    if (e.key === "Enter") {
      if (shortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        shouldSubmit = true;
      } else if (shortcut === "shift-enter" && e.shiftKey && !e.ctrlKey && !e.metaKey) {
        shouldSubmit = true;
      } else if (shortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey)) {
        shouldSubmit = true;
      }
    }

    if (shouldSubmit) {
      e.preventDefault();
      const cmd = value.trim();
      if (cmd) {
        onSubmit(cmd);
        setValue("");
        // Reset height
        if (textareaRef.current) {
          textareaRef.current.style.height = "auto";
        }
      }
    }
  };

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setValue(e.target.value);
    // Auto-resize vertically
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = `${textareaRef.current.scrollHeight}px`;
    }
  };



  return (
    <div className="warp-input-container">
      <div className="warp-input-prompt">▶</div>
      <textarea
        ref={textareaRef}
        className="warp-input-textarea"
        value={value}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        placeholder={`輸入指令... (${shortcut === "enter" ? "Enter" : shortcut === "shift-enter" ? "Shift+Enter" : "Ctrl+Enter"} 送出)`}
        rows={1}
        disabled={disabled}
      />
    </div>
  );
}
