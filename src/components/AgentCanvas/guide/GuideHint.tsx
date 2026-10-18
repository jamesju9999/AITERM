import { useLayoutEffect, useState } from "react";

import { useLocale } from "../../../contexts/LocaleContext";
import { SparklesIcon } from "../../Icons";
import type { Hint } from "./guideSteps";
import "./guide.css";

/** 卡片最寬多少（實際寬度跟著文字走，放置時用這個值預留空間）。 */
const W = 340;
const GAP = 14;

function target(hint: Hint): HTMLElement | null {
  return "guide" in hint.anchor
    ? document.querySelector<HTMLElement>(`[data-guide="${hint.anchor.guide}"]`)
    : document.querySelector<HTMLElement>(`[data-guide-agent="${hint.anchor.agent}"]`);
}

interface Placement {
  /** 泡泡在目標的哪一側；箭頭畫在相反那側、指向目標。 */
  side: "right" | "below";
  left: number;
  top: number;
  /** 目標外框（畫呼吸光暈用）。 */
  ring: { left: number; top: number; width: number; height: number };
}

const same = (a: Placement | null, b: Placement) =>
  !!a && a.side === b.side && a.left === b.left && a.top === b.top && a.ring.left === b.ring.left && a.ring.top === b.ring.top &&
  a.ring.width === b.ring.width && a.ring.height === b.ring.height;

/**
 * 情境提示：在目標旁邊冒一張小卡，箭頭指向目標，目標外圍有一圈呼吸光暈。每次重畫
 * （拖曳、縮放都會讓父層重畫）以及視窗縮放、捲動時重新量位置；目標不在畫面上就不畫。
 */
export function GuideHint({ hint, onDismiss }: { hint: Hint; onDismiss: () => void }) {
  const { t } = useLocale();
  const [pos, setPos] = useState<Placement | null>(null);

  useLayoutEffect(() => {
    const measure = () => {
      const r = target(hint)?.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      if (!r || r.width === 0 || r.bottom < 0 || r.top > vh || r.right < 0 || r.left > vw) {
        setPos((p) => (p === null ? p : null));
        return;
      }
      const ring = { left: r.left - 3, top: r.top - 3, width: r.width + 6, height: r.height + 6 };
      // 右邊放得下就放右邊（跟目標上緣對齊，箭頭指著目標的標題列），否則放在目標下方。
      const next: Placement =
        r.right + GAP + W < vw
          ? { side: "right", left: r.right + GAP, top: Math.min(Math.max(8, r.top + 6), vh - 90), ring }
          : { side: "below", left: Math.min(Math.max(8, r.left), vw - W - 8), top: Math.min(r.bottom + GAP, vh - 90), ring };
      setPos((p) => (same(p, next) ? p : next));
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  });

  if (!pos) return null;
  const text = {
    add_module: t.ac_hint_add_module,
    assign_module: t.ac_hint_assign_module,
    add_reviewer: t.ac_hint_add_reviewer,
    add_orchestrator: t.ac_hint_add_orchestrator,
    add_verifier: t.ac_hint_add_verifier,
  }[hint.id];
  return (
    <>
      <div className="ac-hint-ring" style={pos.ring} aria-hidden />
      <div
        className={`ac-hint ac-hint--${pos.side}`}
        data-testid={`ac-hint-${hint.id}`}
        role="note"
        style={{ left: pos.left, top: pos.top, maxWidth: W }}
      >
        <span className="ac-hint-arrow" aria-hidden />
        <span className="ac-hint-icon" aria-hidden>
          <SparklesIcon size={14} />
        </span>
        <span className="ac-hint-body">
          <span className="ac-hint-label">{t.ac_hint_label}</span>
          <span className="ac-hint-text">{text}</span>
        </span>
        <button className="ac-hint-close" aria-label={t.ac_hint_close} title={t.ac_hint_close} onClick={onDismiss}>
          ×
        </button>
      </div>
    </>
  );
}
