/** 事件在影像上的位置，換成 0～1 的正規化座標（影像之外夾回邊界）。 */
export function normalized(clientX: number, clientY: number, rect: { left: number; top: number; width: number; height: number }) {
  const clamp = (v: number) => (Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0);
  return { x: clamp((clientX - rect.left) / (rect.width || 1)), y: clamp((clientY - rect.top) / (rect.height || 1)) };
}

/** 修飾鍵位元（CDP 的定義）：Alt=1、Ctrl=2、Meta=4、Shift=8。 */
export const modifiers = (e: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
