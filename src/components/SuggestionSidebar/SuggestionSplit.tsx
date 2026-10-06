import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";
import "./SuggestionSplit.css";

export const SIDEBAR_WIDTH_KEY = "aiterm-suggest-sidebar-width";
export const DEFAULT_SIDEBAR_WIDTH = 340;
export const MIN_MAIN_WIDTH = 360;
export const MIN_SIDEBAR_WIDTH = 240;
export const MAX_SIDEBAR_WIDTH = 560;
const DIVIDER_WIDTH = 6;

function loadWidth(): number {
  try {
    const n = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
    if (Number.isFinite(n) && n > 0) return n;
  } catch { /* ignore */ }
  return DEFAULT_SIDEBAR_WIDTH;
}

/** 側欄實際寬度：夾在上下限之間，且不能把終端機欄擠到 MIN_MAIN_WIDTH 以下。 */
function clampWidth(width: number, containerWidth: number): number {
  const room = containerWidth > 0 ? containerWidth - MIN_MAIN_WIDTH - DIVIDER_WIDTH : MAX_SIDEBAR_WIDTH;
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(width, MAX_SIDEBAR_WIDTH, room));
}

interface SuggestionSplitProps {
  /** 側欄內容；null＝不顯示（版面就只有終端機，與沒有這個元件時一樣）。 */
  sidebar: ReactNode | null;
  children: ReactNode;
}

/**
 * 終端機（左）與建議側欄（右）並排，不是疊在上面。
 *
 * **縮放是這裡最大的風險**：側欄寬度一變，終端機欄就跟著變，FitAddon 的
 * ResizeObserver 會觸發真正的 PTY resize；Windows ConPTY 在 resize 時會重送整個
 * 畫面，曾造成舊內容浮現、輸入卡住。所以：
 * - 這裡不自己呼叫 fit／resize，完全交給既有的 ResizeObserver 與縮放閘門；
 * - 拖曳中**完全不改版面**，只畫一條跟著游標的虛線，放開才套用一次寬度——
 *   不然每個 pointermove 都會觸發一次 PTY resize。
 *
 * 拖曳用 setPointerCapture：終端機的 canvas 與其他子元素會在游標移過去時把事件
 * 吃掉，window 上的 mousemove 會頓（同 ArtifactSplit 的理由）。
 */
export function SuggestionSplit({ sidebar, children }: SuggestionSplitProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const [width, setWidth] = useState(loadWidth);
  const [ghostX, setGhostX] = useState<number | null>(null);
  const draggingRef = useRef(false);

  useEffect(() => {
    const el = containerRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (typeof w === "number") setContainerWidth(w);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // 還沒量到寬度（0）時照常顯示；量到了而且放不下兩欄就整個不顯示側欄。
  const tooNarrow = containerWidth > 0 && containerWidth < MIN_MAIN_WIDTH + MIN_SIDEBAR_WIDTH;
  const showSidebar = sidebar != null && !tooNarrow;
  const sidebarWidth = clampWidth(width, containerWidth);

  const xInContainer = (clientX: number) => {
    const rect = containerRef.current?.getBoundingClientRect();
    return rect ? clientX - rect.left : clientX;
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    draggingRef.current = true;
    (e.currentTarget as HTMLDivElement).setPointerCapture?.(e.pointerId);
    setGhostX(xInContainer(e.clientX));
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (draggingRef.current) setGhostX(xInContainer(e.clientX));
  };
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    setGhostX(null);
    const rect = containerRef.current?.getBoundingClientRect();
    const total = rect?.width ?? containerWidth;
    const next = clampWidth(total - xInContainer(e.clientX), total);
    setWidth(next);
    try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(next)); } catch { /* ignore */ }
  };

  return (
    <div
      ref={containerRef}
      className={`aiterm-sugg-split${ghostX != null ? " aiterm-sugg-split--dragging" : ""}`}
    >
      <div className="aiterm-sugg-split__main" style={{ flex: "1 1 0%", minWidth: 0 }}>
        {children}
      </div>
      {showSidebar && (
        <>
          <div
            className="aiterm-sugg-split__divider"
            role="separator"
            aria-orientation="vertical"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
          />
          <div className="aiterm-sugg-split__side" style={{ width: `${sidebarWidth}px`, flex: "0 0 auto" }}>
            {sidebar}
          </div>
        </>
      )}
      {ghostX != null && <div className="aiterm-sugg-split__ghost" style={{ left: `${ghostX}px` }} />}
    </div>
  );
}
