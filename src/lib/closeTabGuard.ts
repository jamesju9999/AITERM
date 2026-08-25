/**
 * 決定一個分頁能不能關閉。
 *
 * 非當前分頁的內容是 `visibility: hidden` + `pointerEvents: none`
 * （見 TerminalApp 的 HIDDEN LAYOUT TRICK），確認框若畫在裡面，使用者
 * 看不見也點不到，await 就永遠不會 resolve。所以有 guard 時先把該分頁
 * 切成當前分頁再問；沒有 guard 的分頁維持原本行為，不做無謂的切換。
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
