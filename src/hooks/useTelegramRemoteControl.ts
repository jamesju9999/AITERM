import { useState, useEffect, useCallback, useRef } from "react";
import { listenTelegramMessage, sendTelegramMessage } from "../ipc/telegram";

export function useTelegramRemoteControl(
  tabId: string,
  // 保留這個參數位置純粹是為了不動到其他呼叫端（CrossDbView / DatabaseView /
  // DesignView）的函式簽章——它們仍傳入自己的「這個分頁是否可見」。這支
  // hook 自己已經不再用它做任何判斷：是否註冊監聽器只看下面的
  // isRemoteEnabled。「哪個分頁該是唯一的 remote 分頁」改由呼叫端決定
  // （見 TerminalView + TerminalApp 的 remoteTabId 互斥機制），決定的結果
  // 會直接反映在 isRemoteEnabled 上，不需要這裡再疊一層以「畫面上看不看得
  // 到」為準的閘門——那正是原本的回歸成因：切到首頁等於分頁不可見，
  // 監聽器就被主動 unlisten，期間收到的 Telegram 訊息永久遺失。
  _isActive: boolean,
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
  }, [isRemoteEnabled, tabId]);

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
