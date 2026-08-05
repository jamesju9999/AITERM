import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLocale } from "../../contexts/LocaleContext";
import {
  MAIL_SYNC_EVENT,
  mailDeleteMessage,
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
  // Which row is currently asking "delete?". At most one, so the confirm
  // control never needs a per-row accessible name to stay unambiguous.
  const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);
  // Disables the confirm control for the duration of the round trip. This is a
  // write to the mail server, and it is not instant — the account's connection
  // spends its life parked in IDLE and has to be woken for it — so without this
  // an impatient second click would queue a second server-side move.
  const [deletingId, setDeletingId] = useState<string | null>(null);
  // Held as a resolved string, not a flag: unlike the load failure this one
  // carries the server's own reason ("no Trash folder", "server supports
  // neither MOVE nor UIDPLUS"), which is the whole point of showing it.
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // Which accounts the poller currently can't reach, and why. Keyed by account
  // rather than held as a single string because the poller runs one task per
  // account: an account that broke while another one was on screen still has to
  // explain itself when the user switches to it.
  //
  // Fed only by the backend's healthy <-> failing transition events, so this
  // never churns per reconnect attempt.
  const [connectionErrors, setConnectionErrors] = useState<Record<string, string>>({});
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
      // Handled ahead of the selected-account filter below, and for every
      // account: a connection failure is the one event whose whole purpose is
      // to still be on screen later, so dropping it because the user happened
      // to be looking at a different account would defeat it.
      if (event.payload.kind === "connection_failed") {
        const { account_id, message } = event.payload;
        setConnectionErrors((prev) => ({ ...prev, [account_id]: message }));
        return;
      }
      if (event.payload.kind === "connection_restored") {
        const { account_id } = event.payload;
        setConnectionErrors((prev) => {
          if (!(account_id in prev)) return prev;
          const next = { ...prev };
          delete next[account_id];
          return next;
        });
        return;
      }
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

  // Moves the message to the server's Trash folder — the one place this app
  // writes to IMAP at all.
  //
  // Nothing is removed optimistically, unlike `handleMarkRead` above. A
  // mark-read that fails costs a rolled-back font weight; a delete that fails
  // and was optimistically applied would tell the user their mail is gone when
  // it is still sitting in their inbox, which is the single worst thing this
  // feature could do. The row goes only after the backend confirms the server
  // accepted the move.
  const handleDelete = (message: MailMessage) => {
    setDeleteError(null);
    setDeletingId(message.id);
    mailDeleteMessage(message.id).then(() => {
      if (!mountedRef.current) return;
      setConfirmingDeleteId(null);
      // The backend also emits a `removed` sync event, which the listener above
      // turns into a refetch (and which refreshes the unread badge). Dropping
      // the row here too just makes the list respond immediately instead of one
      // IPC round trip later; the two agree, so the refetch is a no-op.
      setMessages((prev) => prev.filter((m) => m.id !== message.id));
    }).catch((err) => {
      // Console logging alone is not enough: this app's Tauri window is hard to
      // attach devtools to, and a silent failure here means the user believes
      // mail was deleted when it wasn't.
      console.error("[mail] failed to delete message:", err);
      if (!mountedRef.current) return;
      setConfirmingDeleteId(null);
      setDeleteError(`${t.mail_delete_failed}${String(err)}`);
    }).finally(() => {
      if (mountedRef.current) setDeletingId(null);
    });
  };

  if (loading) return <div className="mail-view" />;

  if (loadFailed) {
    return <div className="mail-view mail-view--empty">{t.mail_load_failed}</div>;
  }

  if (accounts.length === 0) {
    return <div className="mail-view mail-view--empty">{t.mail_no_accounts}</div>;
  }

  const selectedAccount = accounts.find((a) => a.id === selectedAccountId);
  const connectionError = selectedAccountId ? connectionErrors[selectedAccountId] : undefined;

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
      {/* A standing condition, not a transient one: it stays until the poller
          reconnects, and it explains why the list below is not growing. Given
          its own modifier class so it cannot be mistaken for the delete failure
          underneath it — that one is about a single row and goes away by
          itself; this one means nothing in this tab is up to date. */}
      {connectionError && selectedAccount && (
        <div className="mail-view__error mail-view__error--connection" role="alert">
          {t.mail_connection_failed(selectedAccount.email)}{connectionError}
        </div>
      )}
      {/* Inline rather than replacing the list the way the load-failure states
          do: the messages are all still there, and the user needs to see which
          row they just failed to delete. */}
      {deleteError && <div className="mail-view__error" role="alert">{deleteError}</div>}
      <ul className="mail-view__list">
        {messages.map((m) => {
          // Only unread rows are interactive — a read row has nothing left to
          // do, so it must not advertise a button role or take a tab stop.
          const unread = !m.is_read_locally;
          const confirming = confirmingDeleteId === m.id;
          return (
            <li
              key={m.id}
              className={`mail-view__item ${unread ? "mail-view__item--unread" : ""}`}
              // Leaving the row abandons a half-started delete, so a confirm
              // state can't be left armed under the pointer for a later stray
              // click. Only attached while this row is the one confirming, so
              // ordinary mouse traffic over the list costs nothing.
              onMouseLeave={confirming ? () => setConfirmingDeleteId(null) : undefined}
            >
              {/* The role lives on this inner div, not the <li>: overriding the
                  li's implicit listitem role would leave the <ul> with no list
                  items and stop screen readers announcing the list at all.
                  The delete controls are a *sibling* of it rather than a child,
                  because a role="button" element may not contain real buttons —
                  nesting them would hide them from screen readers, and would
                  also fold "Delete" into this row's accessible name. */}
              <div
                className="mail-view__item-main"
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
              {/* Two-step confirm, same shape as removing an account in
                  Settings (aiterm-btn--danger -> --danger-solid): a single
                  click can never reach the server. Every handler stops
                  propagation as well, so that moving these controls inside the
                  clickable row later cannot silently turn a delete into a
                  mark-read. */}
              <div className="mail-view__item-actions">
                {confirming ? (
                  <>
                    <button
                      type="button"
                      className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                      aria-label={t.mail_delete_cancel}
                      onClick={(e) => { e.stopPropagation(); setConfirmingDeleteId(null); }}
                    >
                      {t.cancel}
                    </button>
                    <button
                      type="button"
                      className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--sm"
                      disabled={deletingId === m.id}
                      onClick={(e) => { e.stopPropagation(); handleDelete(m); }}
                    >
                      {t.mail_delete_confirm}
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--danger aiterm-btn--sm"
                    // Per-row, and naming the subject: "Delete" repeated once
                    // per message is useless in a screen reader's element list.
                    aria-label={t.mail_delete_aria(m.subject)}
                    onClick={(e) => { e.stopPropagation(); setDeleteError(null); setConfirmingDeleteId(m.id); }}
                  >
                    {t.mail_delete}
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
