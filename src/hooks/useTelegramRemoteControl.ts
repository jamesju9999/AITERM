import { useCallback, useEffect, useRef } from "react";
import { listenTelegramMessage, sendTelegramMessage } from "../ipc/telegram";

export function useTelegramRemoteControl(
  /** 這個實例的身分。同一個 app 裡必須唯一（例如分頁的 `tab.id`）。 */
  ownerKey: string,
  /**
   * 目前誰擁有 Remote。null = 沒有人。
   *
   * `listen` 是全域的，所以同時有兩個實例註冊，同一則 Telegram 指令就會被
   * 執行兩次。以前是靠各呼叫端自己傳一個「可不可以註冊」的旗標（有的傳分頁
   * 可見性、有的傳互斥 id），四個呼叫端傳的東西不一致，導致終端機分頁跟
   * 資料庫分頁能同時判定自己可以監聽——這就是這次修的 bug。
   *
   * 現在唯一性從擁有權推導：`isRemoteEnabled = ownerKey === remoteOwner`，
   * 而 `remoteOwner` 由所有呼叫端共用同一個上層 state（`TerminalApp` 的
   * `remoteTabId`），天然互斥，「兩個地方同時亮著」在結構上不可能發生。
   */
  remoteOwner: string | null,
  /** 要求變更擁有者。傳 null 表示關閉。 */
  onRemoteOwnerChange: (owner: string | null) => void,
  onMessageReceived: (text: string) => void
) {
  // 刻意不持久化：isRemoteEnabled 完全由 remoteOwner（上層 state，同樣刻意
  // 不持久化）推導而來，沒有自己的 state 可以持久化。Remote 開著時外部訊息
  // 可以直接讓終端機執行指令，這件事不該在使用者不知情的情況下跨重啟自動
  // 恢復——這是安全考量，不是忘了做。
  const isRemoteEnabled = ownerKey === remoteOwner;

  const onMessageReceivedRef = useRef(onMessageReceived);
  useEffect(() => {
    onMessageReceivedRef.current = onMessageReceived;
  }, [onMessageReceived]);

  useEffect(() => {
    if (!isRemoteEnabled) return;

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
  }, [isRemoteEnabled]);

  const toggleRemote = useCallback(() => {
    onRemoteOwnerChange(isRemoteEnabled ? null : ownerKey);
  }, [isRemoteEnabled, ownerKey, onRemoteOwnerChange]);

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
    toggleRemote,
    sendRemoteResponse,
  };
}
