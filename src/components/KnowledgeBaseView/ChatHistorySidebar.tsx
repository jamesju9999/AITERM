import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "../../ipc/fs";
import { kbLoadChatSession, type ChatSessionSummary } from "../../ipc/knowledgeBase";
import { reconstructKbMessages, type KbMessage } from "../../hooks/useKnowledgeBaseChat";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  width: number;
  notebookName: string;
  sessions: ChatSessionSummary[];
  activeSessionId: string | null;
  onNew: () => void;
  onSelect: (sessionId: string) => void;
  onDelete: (sessionId: string) => void;
}

function formatSqliteTimestamp(ts: string): string {
  const iso = `${ts.replace(" ", "T")}Z`;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
}

function buildExportMarkdown(notebookName: string, title: string, messages: KbMessage[]): string {
  const lines: string[] = [`# ${title}\n`, `**筆記本：** \`${notebookName}\`\n`];
  for (const msg of messages) {
    if (msg.role === "user") {
      lines.push(`\n---\n\n**問：** ${msg.content}\n`);
    } else {
      if ((msg.toolCalls ?? []).length > 0) {
        lines.push("\n**工具調用：**\n");
        for (const tc of msg.toolCalls!) {
          lines.push(`- \`${tc.tool}\`（${JSON.stringify(tc.args)}）`);
        }
        lines.push("");
      }
      if (msg.content) {
        lines.push(`\n**答：**\n\n${msg.content}\n`);
      }
    }
  }
  return lines.join("\n");
}

export function ChatHistorySidebar({
  width, notebookName, sessions, activeSessionId, onNew, onSelect, onDelete,
}: Props) {
  const { t } = useLocale();

  const handleExport = async (session: ChatSessionSummary) => {
    const rows = await kbLoadChatSession(session.id);
    const messages = reconstructKbMessages(rows);
    const path = await save({
      defaultPath: `${session.title}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    await writeTextFile(path, buildExportMarkdown(notebookName, session.title, messages));
  };

  return (
    <div className="kb-chat-history" style={{ width }}>
      <div className="kb-chat-history__header">
        <span className="kb-chat-history__title">{t.kb_chat_history_title}</span>
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onNew}>
          {t.kb_new_conversation}
        </button>
      </div>
      <div className="kb-chat-history__list">
        {sessions.length === 0 && (
          <div className="kb-chat-history__empty">{t.kb_no_conversations}</div>
        )}
        {sessions.map((s) => (
          <div
            key={s.id}
            className={`kb-chat-history__item ${s.id === activeSessionId ? "kb-chat-history__item--active" : ""}`}
          >
            <button className="kb-chat-history__item-main" onClick={() => onSelect(s.id)}>
              <div className="kb-chat-history__item-title" title={s.title}>{s.title}</div>
              <div className="kb-chat-history__item-time">{formatSqliteTimestamp(s.updated_at)}</div>
            </button>
            <div className="kb-chat-history__item-actions">
              <button
                className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                title={t.kb_export_conversation}
                onClick={(e) => { e.stopPropagation(); void handleExport(s); }}
              >
                ↓
              </button>
              <button
                className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                onClick={(e) => {
                  e.stopPropagation();
                  if (window.confirm(t.kb_delete_conversation_confirm(s.title))) onDelete(s.id);
                }}
              >
                ✕
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
