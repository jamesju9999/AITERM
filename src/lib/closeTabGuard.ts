/**
 * 決定一個分頁能不能關閉。
 *
 * 非當前分頁的內容是 `visibility: hidden` + `pointerEvents: none`
 * （見 TerminalApp 的 HIDDEN LAYOUT TRICK），確認框若畫在裡面，使用者
 * 看不見也點不到，await 就永遠不會 resolve。所以有 guard 時先把該分頁
 * 切成當前分頁再問；沒有 guard 的分頁維持原本行為，不做無謂的切換。
 *
 * 這個函式不是純的：`setActiveId` 是決策過程中間的副作用，而且**順序不可
 * 調換**——`setActiveId` 必須在呼叫 `guard()` 之前完成，否則確認框仍然會
 * 畫在還沒變可見的分頁裡，等於這個修正沒做。日後若有人想「把它整理成純
 * 函式」，請連同呼叫端一起改，不要只把這行往後搬。
 *
 * 已知的取捨：guard 回傳 false（使用者取消）時不會把焦點切回原本的分頁，
 * 使用者會停在自己沒主動選擇的那個分頁上。見 closeTabGuard.test.ts 裡
 * 釘住這個行為的測試。
 */
export async function runCloseGuard(
  id: string,
  activeId: string | null,
  guard: (() => Promise<boolean>) | undefined,
  setActiveId: (id: string) => void,
): Promise<boolean> {
  if (!guard) return true;
  if (activeId !== id) setActiveId(id);
  return guard();
}
