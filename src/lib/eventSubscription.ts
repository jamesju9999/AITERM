import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * 把 `listen()` 回傳的 promise 包成一個可以安全放進 useEffect cleanup 的
 * 函式。
 *
 * 直接寫 `return () => void un.then((f) => f())` 有兩個問題：
 *
 * 1. **沒有 catch。** `_unlisten` 是先呼叫 webview 端的
 *    `unregisterListener`、成功了才 `invoke('plugin:event|unlisten')` 通知
 *    後端。前者丟出例外時（實測看過 `listeners[eventId]` already
 *    undefined），整串就只變成一個未處理的 rejection——console 只會印出
 *    `undefined is not an object`，看不出是哪一個事件、也看不出後端那邊
 *    其實沒解除成功。
 * 2. **不是冪等的。** StrictMode 的雙重掛載、HMR、以及 cleanup 在 listen
 *    尚未 resolve 時就跑掉，都可能讓同一個 eventId 被解除兩次；第二次必然
 *    失敗，因為第一次已經把它從登記簿移掉了。
 *
 * 所以這裡只解除一次，並且把失敗實際印出來（含事件名）而不是吞掉——
 * 解除失敗代表後端可能還留著那個監聽器，是值得看見的事。
 */
export function unlistenOnCleanup(pending: Promise<UnlistenFn>, eventName: string): () => void {
  let done = false;
  return () => {
    if (done) return;
    done = true;
    void pending
      .then((f) => f())
      .catch((e: unknown) => {
        console.warn(`unlisten "${eventName}" 失敗，後端可能還留著這個監聽器:`, e);
      });
  };
}
