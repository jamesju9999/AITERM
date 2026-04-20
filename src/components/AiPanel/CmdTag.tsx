import { useState } from "react";

interface CmdTagProps {
  command: string;
  multiline: boolean;
  onExec: (cmd: string) => void;
}

export function CmdTag({ command, multiline, onExec }: CmdTagProps) {
  const [confirming, setConfirming] = useState(false);

  const handleClick = () => {
    if (multiline && !confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    onExec(command);
  };

  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      <button
        type="button"
        className="aiterm-cmd-tag"
        onClick={handleClick}
        title={multiline ? "多行命令 — 執行前會再確認一次" : "點擊即執行"}
      >
        <code>{command}</code>
        <span className="aiterm-cmd-tag-play">{confirming ? "確認執行？" : "▶"}</span>
      </button>
      {confirming && (
        <button
          type="button"
          style={{ fontSize: 11, padding: "2px 6px", cursor: "pointer" }}
          onClick={() => setConfirming(false)}
        >
          取消
        </button>
      )}
    </span>
  );
}
