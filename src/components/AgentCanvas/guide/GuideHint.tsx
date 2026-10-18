import { useLayoutEffect, useState } from "react";

import { useLocale } from "../../../contexts/LocaleContext";
import type { Hint } from "./guideSteps";
import "./guide.css";

const W = 240;

function target(hint: Hint): HTMLElement | null {
  return "guide" in hint.anchor
    ? document.querySelector<HTMLElement>(`[data-guide="${hint.anchor.guide}"]`)
    : document.querySelector<HTMLElement>(`[data-guide-agent="${hint.anchor.agent}"]`);
}

/**
 * 情境提示：在目標旁邊冒一個小泡泡。每次重畫（拖曳、縮放都會讓父層重畫）以及
 * 視窗縮放、捲動時重新量位置；目標不在畫面上就不畫。
 */
export function GuideHint({ hint, onDismiss }: { hint: Hint; onDismiss: () => void }) {
  const { t } = useLocale();
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const measure = () => {
      const r = target(hint)?.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      if (!r || r.width === 0 || r.bottom < 0 || r.top > vh || r.right < 0 || r.left > vw) {
        setPos((p) => (p === null ? p : null));
        return;
      }
      // 右邊放得下就放右邊（垂直置中對齊目標），否則放在目標下方。
      const next =
        r.right + 10 + W < vw
          ? { left: r.right + 10, top: Math.max(8, r.top + r.height / 2 - 24) }
          : { left: Math.min(Math.max(8, r.left), vw - W - 8), top: Math.min(r.bottom + 10, vh - 70) };
      setPos((p) => (p && p.left === next.left && p.top === next.top ? p : next));
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
    <div className="ac-hint" data-testid={`ac-hint-${hint.id}`} role="note" style={{ left: pos.left, top: pos.top, maxWidth: W }}>
      <span>{text}</span>
      <button className="ac-iconbtn" aria-label={t.ac_hint_close} title={t.ac_hint_close} onClick={onDismiss}>
        ×
      </button>
    </div>
  );
}
