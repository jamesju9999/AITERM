import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLocale } from "../../contexts/LocaleContext";
import {
  MAIL_SYNC_EVENT,
  mailListAccounts,
  mailListMessages,
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
    }).catch(() => {});
  }, []);

  useEffect(() => {
    if (!selectedAccountId) {
      setMessages([]);
      return;
    }
    mailListMessages(selectedAccountId).then((list) => {
      if (mountedRef.current) setMessages(list);
    }).catch(() => {});
  }, [selectedAccountId]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let active = true;
    listen<MailSyncEvent>(MAIL_SYNC_EVENT, (event) => {
      if (!active) return;
      if (event.payload.account_id !== selectedAccountId) return;
      mailListMessages(event.payload.account_id).then((list) => {
        if (mountedRef.current) setMessages(list);
      }).catch(() => {});
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
          <li key={m.id} className={`mail-view__item ${m.is_read_locally ? "" : "mail-view__item--unread"}`}>
            <div className="mail-view__item-sender">{m.sender}</div>
            <div className="mail-view__item-subject">{m.subject}</div>
            <div className="mail-view__item-summary">{m.ai_summary}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}
