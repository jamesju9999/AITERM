import { useRef, useState, type ReactNode } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { SIDE_DEFAULT, SIDE_MAX, SIDE_MIN, clampSide } from "./useSidePane";

const KEY_STEP = 24;

interface Props {
  open: boolean;
  width: number;
  onWidth: (w: number) => void;
  onToggle: () => void;
  children: ReactNode;
}

/** 右側可拖曳調寬、可收合的側欄外殼。收合時只留一條窄邊與展開鈕。 */
export function SidePane({ open, width, onWidth, onToggle, children }: Props) {
  const { t } = useLocale();
  const drag = useRef<{ x: number; w: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // 容器上限：側欄所在的 .arch-body 寬度（jsdom 量到 0 時視為不限）
  const bound = (w: number) => {
    const cw = ref.current?.parentElement?.getBoundingClientRect().width ?? 0;
    return clampSide(w, cw > 0 ? cw : Infinity);
  };

  if (!open) {
    return (
      <div className="arch-side arch-side-closed" data-testid="arch-side">
        <button type="button" className="arch-side-toggle" aria-label={t.arch_side_expand} title={t.arch_side_expand} onClick={onToggle}>‹</button>
      </div>
    );
  }

  return (
    <div ref={ref} className={`arch-side ${dragging ? "arch-side-dragging" : ""}`} data-testid="arch-side" style={{ width }}>
      <div
        className="arch-side-resizer" role="separator" aria-orientation="vertical" tabIndex={0}
        aria-label={t.arch_side_resize} title={t.arch_side_resize}
        aria-valuenow={width} aria-valuemin={SIDE_MIN} aria-valuemax={SIDE_MAX}
        onPointerDown={(e) => {
          drag.current = { x: e.clientX, w: width };
          setDragging(true);
          e.currentTarget.setPointerCapture?.(e.pointerId);
        }}
        onPointerMove={(e) => { if (drag.current) onWidth(bound(drag.current.w + drag.current.x - e.clientX)); }}
        onPointerUp={(e) => { drag.current = null; setDragging(false); e.currentTarget.releasePointerCapture?.(e.pointerId); }}
        onPointerCancel={() => { drag.current = null; setDragging(false); }}
        onDoubleClick={() => onWidth(SIDE_DEFAULT)}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") { e.preventDefault(); onWidth(bound(width + KEY_STEP)); }
          else if (e.key === "ArrowRight") { e.preventDefault(); onWidth(bound(width - KEY_STEP)); }
        }}
      />
      <button type="button" className="arch-side-toggle arch-side-toggle-open" aria-label={t.arch_side_collapse} title={t.arch_side_collapse} onClick={onToggle}>›</button>
      {children}
    </div>
  );
}
