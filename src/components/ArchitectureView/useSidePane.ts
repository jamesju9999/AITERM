import { useCallback, useState } from "react";

const WIDTH_KEY = "aiterm_arch_side_w";
const OPEN_KEY = "aiterm_arch_side_open";
export const SIDE_MIN = 280;
export const SIDE_MAX = 720;
export const SIDE_DEFAULT = 340;
/** 側欄再寬，畫布也要留這麼多才看得到東西。 */
const CANVAS_MIN = 320;

/** 夾在 [SIDE_MIN, min(SIDE_MAX, 容器寬 − CANVAS_MIN)]；容器放不下最小值時仍回最小值。 */
export function clampSide(w: number, containerWidth: number = Infinity): number {
  const max = Math.max(SIDE_MIN, Math.min(SIDE_MAX, containerWidth - CANVAS_MIN));
  return Math.round(Math.min(max, Math.max(SIDE_MIN, w)));
}

function readWidth(): number {
  try {
    const n = Number(localStorage.getItem(WIDTH_KEY));
    return Number.isFinite(n) && n > 0 ? clampSide(n) : SIDE_DEFAULT;
  } catch { return SIDE_DEFAULT; }
}

function readOpen(): boolean {
  try { return localStorage.getItem(OPEN_KEY) !== "0"; } catch { return true; }
}

/** 側欄的寬度與收合狀態，存在 localStorage（讀寫失敗不影響使用）。 */
export function useSidePane() {
  const [width, setWidthState] = useState(readWidth);
  const [open, setOpenState] = useState(readOpen);
  const setWidth = useCallback((w: number) => {
    const v = clampSide(w);
    setWidthState(v);
    try { localStorage.setItem(WIDTH_KEY, String(v)); } catch { /* 記不住不影響使用 */ }
  }, []);
  const setOpen = useCallback((o: boolean) => {
    setOpenState(o);
    try { localStorage.setItem(OPEN_KEY, o ? "1" : "0"); } catch { /* 記不住不影響使用 */ }
  }, []);
  const toggle = useCallback(() => setOpen(!open), [open, setOpen]);
  const expand = useCallback(() => { if (!open) setOpen(true); }, [open, setOpen]);
  return { width, open, setWidth, toggle, expand };
}
