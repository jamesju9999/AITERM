import { parseCmdTags } from "../../lib/cmdParser";
import { extractResponseText, unescapeNewlines, MarkdownText } from "../../lib/markdown";
import { CmdTag } from "./CmdTag";

interface MessageBubbleProps {
  role: "user" | "assistant";
  content: string;
  onExecuteCommand: (cmd: string) => void;
  streaming?: boolean;
}

export function MessageBubble({
  role,
  content,
  onExecuteCommand,
  streaming,
}: MessageBubbleProps) {
  if (role === "user") {
    return (
      <div className="aiterm-bubble aiterm-bubble-user">
        <span>{content}</span>
      </div>
    );
  }

  // Extract response field from JSON wrappers (e.g. {"thought":..., "response":...})
  // and convert literal \n escape sequences to real newlines.
  const cleaned = unescapeNewlines(extractResponseText(content));

  // Split by <cmd> tags; render text parts with markdown, cmd parts as CmdTag.
  const parts = parseCmdTags(cleaned);
  return (
    <div
      className="aiterm-bubble aiterm-bubble-assistant"
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
    </div>
  );
}
