import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";

export interface InstallLogLine {
  text: string;
  isError: boolean;
}

interface Props {
  command: string;
  args: string[];
  lines: InstallLogLine[];
  onClose: () => void;
}

export function McpInstallTerminal({ command, args, lines, onClose }: Props) {
  const { t } = useLocale();
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines]);

  return (
    <div
      style={{
        position: "fixed",
        bottom: 0,
        left: 0,
        right: 0,
        height: 160,
        background: "#0a0a0a",
        borderTop: "1px solid #2a2a2a",
        display: "flex",
        flexDirection: "column",
        animation: "slideUp 0.2s ease-out",
        zIndex: 100,
      }}
    >
      <style>{`@keyframes slideUp { from { transform: translateY(100%); } to { transform: translateY(0); } }`}</style>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          padding: "4px 10px",
          borderBottom: "1px solid #1e1e1e",
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: 11, color: "#555" }}>{t("mcp_marketplace_terminal_title")}</span>
        <button
          onClick={onClose}
          style={{
            background: "none",
            border: "none",
            color: "#555",
            cursor: "pointer",
            fontSize: 14,
            lineHeight: 1,
            padding: "0 2px",
          }}
        >
          ×
        </button>
      </div>
      <div
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "6px 10px",
          fontFamily: "monospace",
          fontSize: 11,
        }}
      >
        <div style={{ color: "#555" }}>
          $ {command} {args.join(" ")}
        </div>
        {lines.map((line, i) => (
          <div
            key={i}
            style={{ color: line.isError ? "#f87171" : "#4ade80", wordBreak: "break-all" }}
          >
            {line.text}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
