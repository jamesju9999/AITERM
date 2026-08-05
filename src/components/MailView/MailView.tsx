import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLocale } from "../../contexts/LocaleContext";
import {
  MAIL_SYNC_EVENT,
  mailListAccounts,
  mailListMessages,
  mailMarkRead,
  type MailAccount,
  type MailMessage,
  type MailSyncEvent,
} from "../../ipc/mail";
import "./MailView.css";

interface MailViewProps {
  isActive: boolean;
}

// NOTE: unlike TerminalView, this component does not early-return null when
// !isActive — it follows KnowledgeBaseView's convention of staying mounted
// and letting TerminalApp's wrapper div (visibility: hidden / pointerEvents:
// none) handle hiding inactive tabs, so its own state and effects persist
// across tab switches.
export function MailView({ isActive: _isActive }: MailViewProps) {
  const { t } = useLocale();
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MailMessage[]>([]);
  // Guards the empty state: without this, `mail_no_accounts` flashes on every
  // mount before the accounts fetch resolves. Mirrors useNotebooks' `loading`.
  const [loading, setLoading] = useState(true);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  useEffect(() => {
    mailListAccounts().then((list) => {
      if (!mountedRef.current) return;
      setAccounts(list);
      setSelectedAccountId((prev) => prev ?? list[0]?.id ?? null);
    }).catch((err) => {
      // Never swallow: a failed fetch would otherwise be indistinguishable
      // from "no accounts configured" and send the user off to re-add an
      // account they already have.
      console.error("[mail] failed to list accounts:", err);
    }).finally(() => {
      if (mountedRef.current) setLoading(false);
    });
  }, []);

  useEffect(() => {
    if (!selectedAccountId) {
      setMessages([]);
      return;
    }
    mailListMessages(selectedAccountId).then((list) => {
      if (mountedRef.current) setMessages(list);
    }).catch((err) => {
      console.error("[mail] failed to list messages:", err);
    });
  }, [selectedAccountId]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let active = true;
    listen<MailSyncEvent>(MAIL_SYNC_EVENT, (event) => {
      if (!active) return;
      if (event.payload.account_id !== selectedAccountId) return;
      mailListMessages(event.payload.account_id).then((list) => {
        if (mountedRef.current) setMessages(list);
      }).catch((err) => {
        console.error("[mail] failed to refresh messages after sync event:", err);
      });
    }).then((fn) => {
      if (!active) {
        Promise.resolve(fn()).catch(() => {});
      } else {
        unlisten = fn;
      }
    }).catch((err) => {
      console.error("[mail-sync-event] listener registration failed:", err);
    });
    return () => {
      active = false;
      if (unlisten) {
        try { Promise.resolve(unlisten()).catch(() => {}); } catch {}
      }
    };
  }, [selectedAccountId]);

  // Optimistically clear the unread styling so the row responds immediately,
  // without waiting for a full refetch.
  const handleMarkRead = (message: MailMessage) => {
    if (message.is_read_locally) return;
    setMessages((prev) =>
      prev.map((m) => m.id === message.id ? { ...m, is_read_locally: true } : m)
    );
    mailMarkRead(message.id).catch((err) => {
      console.error("[mail] failed to mark message read:", err);
    });
  };

  if (loading) return <div className="mail-view" />;

  if (accounts.length === 0) {
    return <div className="mail-view mail-view--empty">{t.mail_no_accounts}</div>;
  }

  return (
    <div className="mail-view">
      <select
        className="mail-view__account-select"
        value={selectedAccountId ?? ""}
        onChange={(e) => setSelectedAccountId(e.target.value)}
        aria-label={t.mail_select_account}
      >
        {accounts.map((a) => (
          <option key={a.id} value={a.id}>{a.email}</option>
        ))}
      </select>
      <ul className="mail-view__list">
        {messages.map((m) => (
          <li
            key={m.id}
            className={`mail-view__item ${m.is_read_locally ? "" : "mail-view__item--unread"}`}
            role="button"
            tabIndex={0}
            onClick={() => handleMarkRead(m)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                handleMarkRead(m);
              }
            }}
          >
            <div className="mail-view__item-sender">{m.sender}</div>
            <div className="mail-view__item-subject">{m.subject}</div>
            {m.ai_summary && <div className="mail-view__item-summary">{m.ai_summary}</div>}
          </li>
        ))}
      </ul>
    </div>
  );
}
