import { useState } from "react";
import { parseCmdTags } from "../../lib/cmdParser";
import { extractResponseText, unescapeNewlines, MarkdownText } from "../../lib/markdown";
import { CmdTag } from "./CmdTag";
import type { ContentPart } from "../../ipc/ai";
import { useLocale } from "../../contexts/LocaleContext";

interface MessageBubbleProps {
  role: "user" | "assistant";
  content: string | ContentPart[];
  onExecuteCommand: (cmd: string) => void;
  streaming?: boolean;
}

export function MessageBubble({
  role,
  content,
  onExecuteCommand,
  streaming,
}: MessageBubbleProps) {
  const { t } = useLocale();
  const [copied, setCopied] = useState(false);
  const [timeStr] = useState(() => {
    const d = new Date();
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
  });

  const handleCopy = (text: string) => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  if (role === "user") {
    const userBubbleContent = typeof content !== "string" ? (
      <div className="aiterm-bubble aiterm-bubble-user aiterm-bubble-multipart" style={{ width: '100%' }}>
        {content.map((part, i) =>
          part.type === "text" ? (
            <span key={i}>{part.text}</span>
          ) : part.type === "image_url" ? (
            <img key={i} src={part.image_url.url} alt="attachment" className="aiterm-bubble-image-thumb" />
          ) : null
        )}
      </div>
    ) : (
      <div className="aiterm-bubble aiterm-bubble-user" style={{ width: '100%' }}>
        <span>{content}</span>
      </div>
    );

    return (
      <div className="aiterm-bubble-wrapper user-wrapper" style={{ display: 'flex', flexDirection: 'column', alignSelf: 'flex-end', alignItems: 'flex-end', maxWidth: '85%', marginBottom: '4px' }}>
        <div className="aiterm-bubble-meta" style={{ display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '3px', fontSize: '11px', color: 'var(--text-muted)' }}>
          <span style={{ fontWeight: 600, color: 'var(--text-secondary)' }}>You</span>
          <span style={{ fontSize: '9px', opacity: 0.8 }}>{timeStr}</span>
          <span className="avatar" style={{ width: '18px', height: '18px', borderRadius: '50%', background: 'var(--accent-dim)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', fontSize: '10px', color: 'var(--accent)' }}>👤</span>
        </div>
        {userBubbleContent}
      </div>
    );
  }

  const contentStr = typeof content === "string" ? content : "";
  const cleaned = unescapeNewlines(extractResponseText(contentStr));
  const parts = parseCmdTags(cleaned);

  return (
    <div className="aiterm-bubble-wrapper assistant-wrapper" style={{ display: 'flex', flexDirection: 'column', alignSelf: 'flex-start', alignItems: 'flex-start', maxWidth: '85%', marginBottom: '4px' }}>
      <div className="aiterm-bubble-meta" style={{ display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '3px', fontSize: '11px', color: 'var(--text-muted)' }}>
        <span className="avatar" style={{ width: '18px', height: '18px', borderRadius: '50%', background: 'var(--accent-dim)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', fontSize: '10px', color: 'var(--accent)', boxShadow: '0 0 4px var(--accent-glow)' }}>✨</span>
        <span style={{ fontWeight: 600, color: 'var(--text-secondary)' }}>AI Assistant</span>
        <span style={{ fontSize: '9px', opacity: 0.8 }}>{timeStr}</span>
      </div>
      <div
        className="aiterm-bubble aiterm-bubble-assistant aiterm-bubble-assistant--copyable"
        aria-busy={streaming ? "true" : "false"}
        style={{ width: '100%' }}
      >
        {parts.map((p, i) =>
          p.type === "text" ? (
            <MarkdownText key={i} text={p.content} />
          ) : (
            <CmdTag
              key={i}
              command={p.content}
              multiline={p.multiline}
              onExec={onExecuteCommand}
            />
          )
        )}
        {!streaming && (
          <button
            type="button"
            className={`aiterm-bubble-copy-btn${copied ? " aiterm-bubble-copy-btn--copied" : ""}`}
            onClick={() => handleCopy(cleaned)}
            title={t.ai_copy_markdown}
          >
            {copied ? "✓" : "⎘"}
          </button>
        )}
      </div>
    </div>
  );
}
