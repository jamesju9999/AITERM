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
  /**
   * Called after a message is successfully marked read locally. Marking read
   * only touches the backing store, so without this the global unread badge
   * would keep showing a stale count until the next sync event (up to
   * poll_interval_secs, default 300s).
   */
  onMessageRead?: () => void;
}

// NOTE: unlike TerminalView, this component does not early-return null when
// !isActive — it follows KnowledgeBaseView's convention of staying mounted
// and letting TerminalApp's wrapper div (visibility: hidden / pointerEvents:
// none) handle hiding inactive tabs, so its own state and effects persist
// across tab switches.
export function MailView({ isActive, onMessageRead }: MailViewProps) {
  const { t } = useLocale();
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MailMessage[]>([]);
  // Guards the empty state: without this, `mail_no_accounts` flashes on every
  // mount before the accounts fetch resolves. Mirrors useNotebooks' `loading`
  // + `error` pair — the error half matters just as much, since without it a
  // failed fetch renders as "no accounts configured".
  const [loading, setLoading] = useState(true);
  // Held as a flag rather than a message string so the accounts effect does
  // not have to close over `t` (which would drag the whole translation table
  // into its dep array and refetch on every locale switch); the message is
  // resolved at render time instead, and stays correct if the locale changes.
  const [loadFailed, setLoadFailed] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Keyed on isActive, not [] — the same reactivation idiom KnowledgeBaseView
  // uses. Settings lives in an overlay that never unmounts TerminalApp, so a
  // mount-only fetch would leave this tab showing `mail_no_accounts` forever
  // after the user adds their first account in Settings and switches back,
  // while the TabBar badge (fed by mail_count_unread) already shows a count.
  // The dep array is what keeps this to one fetch per false -> true edge
  // rather than one per render.
  useEffect(() => {
    if (!isActive) return;
    mailListAccounts().then((list) => {
      if (!mountedRef.current) return;
      setAccounts(list);
      // Keep the user's current pick across a refetch; fall back to the first
      // account only when the selection is gone (removed in Settings) or was
      // never made.
      setSelectedAccountId((prev) =>
        prev && list.some((a) => a.id === prev) ? prev : list[0]?.id ?? null
      );
      // Cleared on success, or a first-load failure would stick as a permanent
      // error screen even once a later refetch succeeds.
      setLoadFailed(false);
    }).catch((err) => {
      // Never swallow: a failed fetch would otherwise be indistinguishable
      // from "no accounts configured" and send the user off to re-add an
      // account they already have. Console logging alone is not enough —
      // this app's Tauri window is hard to attach devtools to, so the error
      // has to reach the UI.
      console.error("[mail] failed to list accounts:", err);
      if (mountedRef.current) setLoadFailed(true);
    }).finally(() => {
      if (mountedRef.current) setLoading(false);
    });
  }, [isActive]);

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
    mailMarkRead(message.id).then(() => {
      // Only on success: the failure path below rolls the row back to unread,
      // so refreshing the badge there would just re-report the same count.
      onMessageRead?.();
    }).catch((err) => {
      console.error("[mail] failed to mark message read:", err);
      // Roll back, otherwise the row reads as read while the backend still
      // has it unread — and the next sync refetch would flip it back to bold,
      // looking like a glitch.
      if (!mountedRef.current) return;
      setMessages((prev) =>
        prev.map((m) => m.id === message.id ? { ...m, is_read_locally: false } : m)
      );
    });
  };

  if (loading) return <div className="mail-view" />;

  if (loadFailed) {
    return <div className="mail-view mail-view--empty">{t.mail_load_failed}</div>;
  }

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
        {messages.map((m) => {
          // Only unread rows are interactive — a read row has nothing left to
          // do, so it must not advertise a button role or take a tab stop.
          const unread = !m.is_read_locally;
          return (
            <li key={m.id} className={`mail-view__item ${unread ? "mail-view__item--unread" : ""}`}>
              {/* The role lives on this inner div, not the <li>: overriding the
                  li's implicit listitem role would leave the <ul> with no list
                  items and stop screen readers announcing the list at all. */}
              <div
                role={unread ? "button" : undefined}
                tabIndex={unread ? 0 : undefined}
                onClick={unread ? () => handleMarkRead(m) : undefined}
                onKeyDown={unread ? (e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    handleMarkRead(m);
                  }
                } : undefined}
              >
                <div className="mail-view__item-sender">{m.sender}</div>
                <div className="mail-view__item-subject">{m.subject}</div>
                {m.ai_summary && <div className="mail-view__item-summary">{m.ai_summary}</div>}
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
