interface CmdTagProps {
  command: string;
  multiline: boolean;
  onExec: (cmd: string) => void;
}

export function CmdTag({ command, multiline, onExec }: CmdTagProps) {
  const handleClick = () => {
    if (multiline) {
      const ok = window.confirm(
        `確定執行多行命令？\n\n${command}`,
      );
      if (!ok) return;
    }
    onExec(command);
  };
  return (
    <button
      type="button"
      className="aiterm-cmd-tag"
      onClick={handleClick}
      title={multiline ? "多行命令 — 執行前會再確認一次" : "點擊即執行"}
    >
      <code>{command}</code>
      <span className="aiterm-cmd-tag-play">▶</span>
    </button>
  );
}
