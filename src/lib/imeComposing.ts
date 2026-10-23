/**
 * 使用者用中文／日文等輸入法選字時，按 Enter 只是要「確認候選字」，但瀏覽器
 * 一樣會送出一個 keydown，且 e.key === "Enter"。吃純 Enter 送出的文字輸入若
 * 沒有分辨組字階段，就會把確認候選字誤當成送出指令。
 *
 * 所有「純 Enter 觸發送出／選取」的處理器都應在開頭呼叫這個函式擋掉組字中的事件。
 * 需要 Ctrl/Cmd/Shift 修飾鍵才送出的處理器不受影響（IME 確認是純 Enter）。
 *
 * isComposing 是標準屬性；keyCode === 229 是舊版 WebKit（含部分 WKWebView 狀態）
 * 在組字期間的既有行為，一起判斷比較保險。
 *
 * 同時接受 React 合成事件（透過 nativeEvent）與原生 KeyboardEvent（window 層監聽）。
 */
export function isImeComposing(e: {
  isComposing?: boolean;
  keyCode?: number;
  nativeEvent?: { isComposing?: boolean };
}): boolean {
  return Boolean(e.nativeEvent?.isComposing ?? e.isComposing) || e.keyCode === 229;
}

/** compositionend 之後多久內的 Enter 仍視為「確認候選字」的那一下。 */
export const IME_ENTER_GRACE_MS = 100;

/**
 * Safari／WKWebView（Tauri macOS）的事件順序是 compositionend 先於確認候選字的
 * keydown，所以 keydown 當下 isComposing 已是 false、keyCode 也可能是 13，
 * 只靠 isImeComposing 擋不住。這個 guard 自己記錄組字狀態與結束時間，
 * 把 compositionend 後 IME_ENTER_GRACE_MS 內的 Enter 也吞掉。
 *
 * 用法：textarea 掛 onCompositionStart／onCompositionEnd，onKeyDown 內
 * `if (guard.shouldIgnoreEnter(e)) return;`。
 */
export function createImeEnterGuard(now: () => number = () => performance.now()) {
  let composing = false;
  let endedAt = -Infinity;
  return {
    onCompositionStart() {
      composing = true;
    },
    onCompositionEnd() {
      composing = false;
      endedAt = now();
    },
    shouldIgnoreEnter(e: Parameters<typeof isImeComposing>[0]): boolean {
      return isImeComposing(e) || composing || now() - endedAt < IME_ENTER_GRACE_MS;
    },
  };
}
