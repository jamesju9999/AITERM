import { parseCmdTags } from "../../lib/cmdParser";
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

  // Assistant: split by <cmd> tags.
  const parts = parseCmdTags(content);
  return (
    <div
      className="aiterm-bubble aiterm-bubble-assistant"
      aria-busy={streaming ? "true" : "false"}
    >
      {parts.map((p, i) =>
        p.type === "text" ? (
          <span key={i}>{p.content}</span>
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
