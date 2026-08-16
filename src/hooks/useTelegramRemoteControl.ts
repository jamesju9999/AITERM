import { useState, useEffect, useCallback, useRef } from "react";
import { listenTelegramMessage, sendTelegramMessage } from "../ipc/telegram";

export function useTelegramRemoteControl(
  tabId: string,
  /**
   * 這個實例可不可以註冊監聽器。
   *
   * `listen` 是全域的，所以同時有兩個實例註冊，同一則 Telegram 指令就會被
   * 執行兩次——這個參數存在的唯一理由就是保證「同時只有一個」。
   *
   * 各呼叫端傳的東西不同，因為它們保證唯一性的方式不同：
   * - `TerminalView` 傳「我是不是那個 remote 分頁」（`TerminalApp` 的
   *   `remoteTabId` 互斥機制）。**不能**傳「分頁看不看得見」——首頁是啟動
   *   預設畫面，按一下首頁就會 unlisten，期間的 Telegram 訊息永久遺失
   *   （Tauri 的 emit 找不到 listener 就直接丟棄，沒有 buffer）。那就是這次
   *   修掉的回歸。
   * - `CrossDbView` / `DatabaseView` / `DesignView` 仍傳「這個分頁可不可見」。
   *   它們沒有接上 `remoteTabId`，所以還是靠可見性當唯一性的代理，也因此
   *   仍有同一個首頁回歸。修它們是另一個 Task 的事——但這個參數必須留著，
   *   否則它們會跟終端機分頁同時註冊，指令被執行兩次。
   */
  canListen: boolean,
  onMessageReceived: (text: string) => void
) {
  // 刻意不持久化 isRemoteEnabled：Remote 開著時外部訊息可以直接讓終端機
  // 執行指令，這件事不該在使用者不知情的情況下跨重啟自動恢復——這是安全
  // 考量，不是忘了做。
  //
  // （這裡原本有一段持久化邏輯，但它本身是壞的：storage key 用 PTY
  // sessionId，而 sessionId 在這個 useState initializer 執行的當下永遠是
  // 空字串，且每次啟動都是新 UUID，所以冷啟動一律讀回 false，接著持久化
  // effect 又會立刻把 "false" 寫回去蓋掉舊值——拿掉這段不是行為變更。）
  const [isRemoteEnabled, setIsRemoteEnabled] = useState(false);

  const onMessageReceivedRef = useRef(onMessageReceived);
  useEffect(() => {
    onMessageReceivedRef.current = onMessageReceived;
  }, [onMessageReceived]);

  useEffect(() => {
    if (!isRemoteEnabled || !canListen) return;

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
  }, [isRemoteEnabled, canListen, tabId]);

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
