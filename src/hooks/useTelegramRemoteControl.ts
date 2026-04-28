import { useState, useEffect, useCallback, useRef } from "react";
import { listenTelegramMessage, sendTelegramMessage } from "../ipc/telegram";

export function useTelegramRemoteControl(
  tabId: string,
  isActive: boolean,
  onMessageReceived: (text: string) => void
) {
  // Persist isRemoteEnabled per tabId so it survives HMR / hot reloads.
  // Only persist when tabId is stable (non-empty, non-UUID-like).
  const storageKey = tabId ? `aiterm-remote:${tabId}` : null;

  const [isRemoteEnabled, setIsRemoteEnabled] = useState(() => {
    if (!storageKey) return false;
    try {
      return localStorage.getItem(storageKey) === "true";
    } catch {
      return false;
    }
  });

  // Persist state changes to localStorage.
  useEffect(() => {
    if (!storageKey) return;
    try {
      localStorage.setItem(storageKey, String(isRemoteEnabled));
    } catch { /* ignore */ }
  }, [storageKey, isRemoteEnabled]);

  const onMessageReceivedRef = useRef(onMessageReceived);
  useEffect(() => {
    onMessageReceivedRef.current = onMessageReceived;
  }, [onMessageReceived]);

  useEffect(() => {
    if (!isRemoteEnabled || !isActive) return;

    // Use the same active-flag pattern as useAiChat to handle the
    // async listen() race condition with React StrictMode / HMR cleanup.
    let unlisten: (() => void) | null = null;
    let active = true;

    listenTelegramMessage((payload) => {
      onMessageReceivedRef.current(payload.text);
    }).then((fn) => {
      if (!active) {
        // Cleanup already ran before the Promise resolved — unlisten immediately.
        // Wrap in Promise.resolve().catch to swallow Tauri internal rejections
        // when the listener wasn't fully wired up.
        Promise.resolve(fn()).catch(() => { /* ignore */ });
      } else {
        unlisten = fn;
      }
    }).catch((err) => {
      console.error("[telegram] listener registration failed:", err);
    });

    return () => {
      active = false;
      if (unlisten) {
        try {
          Promise.resolve(unlisten()).catch(() => { /* ignore */ });
        } catch { /* ignore */ }
      }
    };
  }, [isRemoteEnabled, isActive, tabId]);

  const sendRemoteResponse = useCallback(
    async (text: string) => {
      if (isRemoteEnabled) {
        try {
          await sendTelegramMessage(text);
        } catch (e) {
          console.error("Failed to send telegram message:", e);
        }
      }
    },
    [isRemoteEnabled]
  );

  return {
    isRemoteEnabled,
    setIsRemoteEnabled,
    sendRemoteResponse,
  };
}
