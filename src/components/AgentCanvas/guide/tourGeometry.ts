/** 聚光燈框比目標每邊大 PAD，但夾在視窗內。 */
const PAD = 4;
/** 離視窗邊至少留這麼多：macOS 視窗有圓角與邊框，貼太近（2px）看起來還是像被切掉。 */
const EDGE = 8;

export function clampSpot(r: Pick<DOMRect, "left" | "top" | "right" | "bottom">, vw: number, vh: number) {
  const left = Math.max(EDGE, r.left - PAD);
  const top = Math.max(EDGE, r.top - PAD);
  const right = Math.min(vw - EDGE, r.right + PAD);
  const bottom = Math.min(vh - EDGE, r.bottom + PAD);
  return { left, top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
}
