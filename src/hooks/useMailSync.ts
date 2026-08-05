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
  const seqRef = useRef(0);
  const permissionRef = useRef<Promise<boolean> | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Exposed so MailView can call it after a successful local mark-read.
  // Without that, marking a message read only changes the backing store, and
  // the badge would keep showing the stale count until the next sync event
  // (up to poll_interval_secs, default 300s).
  const refreshUnread = useCallback(() => {
    // The poller emits one event per inserted message, so a single poll cycle
    // can fire dozens of refreshes at once. Their round-trips resolve in
    // arbitrary order, so without this guard the badge would be last-RESOLVED-
    // wins: an older, lower count could land after a newer one and stick until
    // the next poll cycle — the very staleness this refresh exists to avoid.
    const seq = ++seqRef.current;
    mailCountUnread().then((count) => {
      if (mountedRef.current && seq === seqRef.current) setUnreadCount(count);
    }).catch((err) => {
      console.error("[mail] failed to count unread messages:", err);
    });
  }, []);

  // Resolved once per session and cached: otherwise every important message
  // costs another permission round-trip, and a burst of them would stack
  // concurrent requestPermission() prompts — including re-prompting a user who
  // already said no.
  //
  // Desktop caveat (tauri-plugin-notification 2.3.3, desktop.rs:61-66): both
  // permission_state() and request_permission() unconditionally return Granted
  // on desktop — the OS permission is never actually consulted here. This stays
  // because it is the plugin's documented API and is load-bearing on mobile,
  // but on macOS/Windows/Linux it always resolves true.
  const ensureNotificationPermission = useCallback((): Promise<boolean> => {
    permissionRef.current ??= isPermissionGranted()
      .then((granted) => granted || requestPermission().then((p) => p === "granted"))
      .catch((err) => {
        console.error("[mail] notification permission check failed:", err);
        // Don't cache a transient IPC failure as a denial.
        permissionRef.current = null;
        return false;
      });
    return permissionRef.current;
  }, []);

  useEffect(() => {
    refreshUnread();

    let unlisten: UnlistenFn | null = null;
    let active = true;

    listen<MailSyncEvent>(MAIL_SYNC_EVENT, async (event) => {
      if (!active) return;
      refreshUnread();

      if (event.payload.kind === "important") {
        if (await ensureNotificationPermission()) {
          // Matches the backend's own placeholder (parse_raw_message) so a
          // genuinely empty subject doesn't produce a blank notification title.
          const title = event.payload.subject || "(no subject)";
          // Do not expect this to be visible under `tauri dev` on macOS: the
          // plugin posts as "com.apple.Terminal" whenever tauri::is_dev() is
          // true (desktop.rs:207-214), because an unbundled dev binary cannot
          // post under its own bundle id. So in dev it surfaces as a Terminal
          // notification — and silently does nothing if Terminal itself lacks
          // notification permission. The plugin also discards the result
          // (`let _ = notification.show()`, desktop.rs:216-218), so a failure
          // is unobservable from here. Verify this path against a real
          // `tauri build` bundle, where the app's own identifier is used.
          sendNotification({ title, body: event.payload.summary });
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
  }, [refreshUnread, ensureNotificationPermission]);

  return { unreadCount, refreshUnread };
}
