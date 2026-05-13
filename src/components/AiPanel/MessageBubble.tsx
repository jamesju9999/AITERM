import { useState } from "react";
import { parseCmdTags } from "../../lib/cmdParser";
import { extractResponseText, unescapeNewlines, MarkdownText } from "../../lib/markdown";
import { CmdTag } from "./CmdTag";
import type { ContentPart } from "../../ipc/ai";

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
  const [copied, setCopied] = useState(false);

  if (role === "user") {
    return (
      <div className="aiterm-bubble aiterm-bubble-user">
        {typeof content === "string" ? (
          <span>{content}</span>
        ) : (
          <div className="aiterm-bubble-multipart">
            {content.map((part, i) =>
              part.type === "text" ? (
                <span key={i}>{part.text}</span>
              ) : (
                <img
                  key={i}
                  src={part.image_url.url}
                  alt="attached image"
                  className="aiterm-bubble-image-thumb"
                />
              )
            )}
          </div>
        )}
      </div>
    );
  }

  // Extract response field from JSON wrappers (e.g. {"thought":..., "response":...})
  // and convert literal \n escape sequences to real newlines.
  const cleaned = unescapeNewlines(extractResponseText(typeof content === "string" ? content : ""));

  const handleCopy = () => {
    void navigator.clipboard.writeText(cleaned).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  // Split by <cmd> tags; render text parts with markdown, cmd parts as CmdTag.
  const parts = parseCmdTags(cleaned);
  return (
    <div
      className="aiterm-bubble aiterm-bubble-assistant aiterm-bubble-assistant--copyable"
      aria-busy={streaming ? "true" : "false"}
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
        ),
      )}
      {!streaming && (
        <button
          type="button"
          className={`aiterm-bubble-copy-btn${copied ? " aiterm-bubble-copy-btn--copied" : ""}`}
          onClick={handleCopy}
          title="複製為 Markdown"
        >
          {copied ? "✓" : "⎘"}
        </button>
      )}
    </div>
  );
}
