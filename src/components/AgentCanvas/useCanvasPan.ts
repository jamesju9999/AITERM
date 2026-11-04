import { useCallback, useEffect, useRef, type MouseEvent as ReactMouseEvent, type RefObject } from "react";

/** 桌布上哪些地方按下去是「抓住桌布」：桌布本身與裝節點的兩層容器的空白處。
 *  節點、按鈕、資源、連線等都在別的元素上，按到它們不平移（那是拖節點、按按鈕）。 */
const PAN_SURFACE = ["ac-sizer", "ac-world"];

function startsOnSurface(target: EventTarget | null, canvas: HTMLElement): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target === canvas || PAN_SURFACE.some((c) => target.classList.contains(c));
}

/**
 * 在桌布空白處按住左鍵拖曳來平移（像地圖）。回傳要掛在桌布 `onMouseDown` 的處理函式。
 * 直接改捲動位置，所以縮放後的座標、全部顯示的錨點都不受影響。
 * 拖曳期間在桌布上加 `ac-canvas--panning`（游標變抓手）；放開或卸載時一律清掉。
 */
export function useCanvasPan(ref: RefObject<HTMLElement | null>) {
  const stop = useRef<(() => void) | null>(null);

  useEffect(() => () => stop.current?.(), []);

  return useCallback(
    (e: ReactMouseEvent) => {
      const el = ref.current;
      if (e.button !== 0 || !el || !startsOnSurface(e.target, el)) return;
      // 不然拖過去會選到文字。
      e.preventDefault();
      stop.current?.();
      const startX = e.clientX;
      const startY = e.clientY;
      const startLeft = el.scrollLeft;
      const startTop = el.scrollTop;
      const move = (m: MouseEvent) => {
        el.scrollLeft = startLeft - (m.clientX - startX);
        el.scrollTop = startTop - (m.clientY - startY);
      };
      const end = () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", end);
        el.classList.remove("ac-canvas--panning");
        stop.current = null;
      };
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", end);
      el.classList.add("ac-canvas--panning");
      stop.current = end;
    },
    [ref],
  );
}
