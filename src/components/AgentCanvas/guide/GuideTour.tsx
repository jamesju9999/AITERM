import { useEffect, useLayoutEffect, useState } from "react";

import { useLocale } from "../../../contexts/LocaleContext";
import { clampSpot } from "./tourGeometry";
import "./guide.css";

const STEPS = ["pool", "templates", "library", "canvas", "inbox", "log", "help"] as const;
type TourStep = (typeof STEPS)[number];

const find = (id: string) => document.querySelector<HTMLElement>(`[data-guide="${id}"]`);

const TIP_W = 320;
const TIP_H = 170;
/**
 * 步驟導覽：逐一圈出畫面上的區塊並說明。目標靠 `data-guide` 標記；開始時找不到的
 * 步驟（例如還沒選專案時沒有範本鈕）直接略過。任何方式結束都呼叫 `onDone`。
 */
export function GuideTour({ onDone }: { onDone: () => void }) {
  const { t } = useLocale();
  // 掛上的那一輪，有些目標（例如範本鈕）可能跟導覽同一次 render 才出現、還沒進 DOM，
  // 所以每次量位置時重算一次哪些步驟可用。
  const [steps, setSteps] = useState<TourStep[]>(() => STEPS.filter((id) => find(id)));
  const [i, setI] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const step = steps[i];

  useLayoutEffect(() => {
    if (!step) return;
    const measure = () => {
      const avail = STEPS.filter((id) => find(id));
      setSteps((cur) => (cur.length === avail.length && cur.every((x, k) => x === avail[k]) ? cur : avail));
      setRect(find(step)?.getBoundingClientRect() ?? null);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [step]);

  useEffect(() => {
    if (steps.length === 0) onDone();
  }, [steps.length, onDone]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDone();
      else if (e.key === "ArrowRight") setI((n) => (n + 1 < steps.length ? n + 1 : n));
      else if (e.key === "ArrowLeft") setI((n) => Math.max(0, n - 1));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDone, steps.length]);

  if (!step) return null;
  const text: Record<TourStep, [string, string]> = {
    pool: [t.ac_guide_pool_title, t.ac_guide_pool_body],
    templates: [t.ac_guide_templates_title, t.ac_guide_templates_body],
    library: [t.ac_guide_library_title, t.ac_guide_library_body],
    canvas: [t.ac_guide_canvas_title, t.ac_guide_canvas_body],
    inbox: [t.ac_guide_inbox_title, t.ac_guide_inbox_body],
    log: [t.ac_guide_log_title, t.ac_guide_log_body],
    help: [t.ac_guide_help_title, t.ac_guide_help_body],
  };
  const [title, body] = text[step];
  const last = i === steps.length - 1;

  // 說明框放在目標下方，下面放不下就放上方；左右夾在視窗內。目標比視窗還高（桌布）時放在目標裡面靠上。
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  let top = 16;
  let left = 16;
  if (rect) {
    if (rect.bottom + 12 + TIP_H < vh) top = rect.bottom + 12;
    else if (rect.top - 12 - TIP_H > 0) top = rect.top - 12 - TIP_H;
    else top = Math.max(16, rect.top + 16);
    left = Math.min(Math.max(16, rect.left), Math.max(16, vw - TIP_W - 16));
  }

  // 框比目標每邊大 4px，但夾在視窗內——貼著視窗邊的目標（角色列延伸到右緣）框線才不會被切掉。
  const spot = rect && clampSpot(rect, vw, vh);

  return (
    <div className="ac-tour" role="region" aria-label={t.ac_guide_region} data-testid="ac-tour">
      {/* 導覽期間擋住底下的操作。 */}
      <div className="ac-tour-block" />
      {spot && <div className="ac-tour-spot" style={spot} />}
      <div className="ac-tour-tip" style={{ left, top, width: TIP_W }}>
        <div className="ac-tour-head">
          <b>{title}</b>
          <span className="ac-tour-count">{t.ac_guide_step(i + 1, steps.length)}</span>
        </div>
        <p>{body}</p>
        <div className="ac-tour-actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onDone}>
            {t.ac_guide_skip}
          </button>
          <span className="ac-spacer" />
          {i > 0 && (
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setI(i - 1)}>
              {t.ac_guide_prev}
            </button>
          )}
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            autoFocus
            onClick={() => (last ? onDone() : setI(i + 1))}
          >
            {last ? t.ac_guide_finish : t.ac_guide_next}
          </button>
        </div>
      </div>
    </div>
  );
}
