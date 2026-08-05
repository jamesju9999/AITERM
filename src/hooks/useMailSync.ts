import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { MAIL_SYNC_EVENT, mailCountUnread, type MailSyncEvent } from "../ipc/mail";

/**
 * Tracks the global unread mail count and raises an OS notification when the
 * backend flags an incoming message as important.
 *
 * Mounted once in TerminalApp (the always-mounted shell), NOT in MailView:
 * important mail has to notify the user even if the Mail tab was never opened
 * this session.
 */
export function useMailSync() {
  const [unreadCount, setUnreadCount] = useState(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Exposed so MailView can call it after a successful local mark-read.
  // Without that, marking a message read only changes the backing store, and
  // the badge would keep showing the stale count until the next sync event
  // (up to poll_interval_secs, default 300s).
  const refreshUnread = useCallback(() => {
    mailCountUnread().then((count) => {
      if (mountedRef.current) setUnreadCount(count);
    }).catch((err) => {
      console.error("[mail] failed to count unread messages:", err);
    });
  }, []);

  useEffect(() => {
    refreshUnread();

    let unlisten: UnlistenFn | null = null;
    let active = true;

    listen<MailSyncEvent>(MAIL_SYNC_EVENT, async (event) => {
      if (!active) return;
      refreshUnread();

      if (event.payload.kind === "important") {
        let granted = await isPermissionGranted();
        if (!granted) {
          const permission = await requestPermission();
          granted = permission === "granted";
        }
        if (granted) {
          sendNotification({ title: event.payload.subject, body: event.payload.summary });
        }
      }
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
        try { Promise.resolve(unlisten()).catch(() => {}); } catch { /* teardown races are not actionable */ }
      }
    };
  }, [refreshUnread]);

  return { unreadCount, refreshUnread };
}
