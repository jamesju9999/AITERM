import { useEffect, useRef } from "react";
import { sendNotification } from "@tauri-apps/plugin-notification";
import type { ActivitySnapshot } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { ensureNotificationPermission } from "../../lib/notifyPermission";
import { detectTransitions, findStuck, STUCK_MS } from "./alerts";
import { EMPTY_SNAPSHOT } from "../../ipc/claudeActivity";
import { isClaudeActivityNotifyEnabled } from "./settings";

const STUCK_CHECK_MS = 30_000;

/**
 * 失敗／全部完成／卡住時發桌面通知，只在使用者沒在看這個分頁時才發
 * （分頁不是作用中，或視窗沒有焦點）。掛在 TerminalView：非作用中的分頁也是掛載著的。
 */
export function useClaudeActivityAlerts(snapshot: ActivitySnapshot, isActive: boolean): void {
  const { t } = useLocale();
  const tRef = useRef(t);
  const activeRef = useRef(isActive);
  const snapRef = useRef(snapshot);
  const prevRef = useRef<ActivitySnapshot>(EMPTY_SNAPSHOT);
  const stuckNotified = useRef(new Set<string>());

  useEffect(() => {
    tRef.current = t;
    activeRef.current = isActive;
    snapRef.current = snapshot;
  });

  const notify = (body: string) => {
    if (!isClaudeActivityNotifyEnabled()) return;
    if (activeRef.current && document.hasFocus()) return;
    void ensureNotificationPermission().then((granted) => {
      if (granted) sendNotification({ title: tRef.current.claude_activity_title, body });
    });
  };

  useEffect(() => {
    const prev = prevRef.current;
    prevRef.current = snapshot;
    if (snapshot.nodes.length === 0) stuckNotified.current.clear();
    const label = (d: string | null) => d ?? tRef.current.claude_activity_untitled;
    for (const a of detectTransitions(prev, snapshot, Date.now())) {
      notify(
        a.kind === "failed"
          ? tRef.current.claude_activity_alert_failed(label(a.description))
          : tRef.current.claude_activity_alert_all_done(a.count, a.failed),
      );
    }
  }, [snapshot]);

  const anyRunning = snapshot.nodes.some((n) => n.status === "running");
  useEffect(() => {
    if (!anyRunning) return;
    const id = setInterval(() => {
      for (const node of findStuck(snapRef.current.nodes, Date.now(), stuckNotified.current)) {
        stuckNotified.current.add(node.id);
        notify(
          tRef.current.claude_activity_alert_stuck(
            node.description ?? tRef.current.claude_activity_untitled,
            Math.floor(STUCK_MS / 60_000),
          ),
        );
      }
    }, STUCK_CHECK_MS);
    return () => clearInterval(id);
  }, [anyRunning]);
}
