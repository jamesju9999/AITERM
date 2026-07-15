import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";

interface CmdTagProps {
  command: string;
  multiline: boolean;
  onExec: (cmd: string) => void;
}

export function CmdTag({ command, multiline, onExec }: CmdTagProps) {
  const { t } = useLocale();
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
        title={multiline ? t.cmd_multiline_tip : t.cmd_click_execute}
      >
        <code>{command}</code>
        <span className="aiterm-cmd-tag-play">{confirming ? t.cmd_confirm_execute : "▶"}</span>
      </button>
      {confirming && (
        <button
          type="button"
          style={{ fontSize: 11, padding: "2px 6px", cursor: "pointer" }}
          onClick={() => setConfirming(false)}
        >
          {t.cancel}
        </button>
      )}
    </span>
  );
}
