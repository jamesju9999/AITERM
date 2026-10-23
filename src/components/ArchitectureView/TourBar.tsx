import { useEffect } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { Tour } from "./qa";
import { autoplayDelayMs, clampStep, stepBounds } from "./tour";

interface Props {
  tour: Tour;
  index: number;
  playing: boolean;
  onIndex: (i: number) => void;
  onPlaying: (p: boolean) => void;
  onClose: () => void;
  onOpenFile: (path: string, line?: number) => void;
}

/** 畫布底部中央的導覽字幕列。自動播放的計時、鍵盤操作都在這裡，只在導覽進行中才掛載。 */
export function TourBar({ tour, index, playing, onIndex, onPlaying, onClose, onOpenFile }: Props) {
  const { t } = useLocale();
  const { steps } = tour;
  const { first, last } = stepBounds(steps, index);
  const step = steps[clampStep(steps, index)];

  // 每步一個 setTimeout：換步、暫停、卸載都會清掉重來；最後一步到時就停住。
  useEffect(() => {
    if (!playing) return;
    const timer = setTimeout(() => {
      if (last) onPlaying(false);
      else onIndex(index + 1);
    }, autoplayDelayMs);
    return () => clearTimeout(timer);
  }, [playing, index, last, onIndex, onPlaying]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { onClose(); return; }
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
      onIndex(clampStep(steps, index + (e.key === "ArrowRight" ? 1 : -1)));
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [steps, index, onIndex, onClose]);

  return (
    <div className="arch-tour" role="region" aria-label={tour.title}>
      <div className="arch-tour-text">
        <span className="arch-tour-step">{t.arch_tour_step.replace("{i}", String(index + 1)).replace("{n}", String(steps.length))}</span>
        <strong className="arch-tour-title">{tour.title}</strong>
        <span className="arch-tour-caption">{step.caption}</span>
        {step.evidence && (
          <button type="button" className="arch-tour-evidence" onClick={() => onOpenFile(step.evidence!.file, step.evidence!.line)}>
            {`${step.evidence.file}:${step.evidence.line}`}
          </button>
        )}
      </div>
      <div className="arch-tour-ctl">
        <button type="button" className="arch-btn arch-btn-small" aria-label={t.arch_tour_prev} title={t.arch_tour_prev} disabled={first} onClick={() => onIndex(index - 1)}>⏮</button>
        <button type="button" className="arch-btn arch-btn-small" aria-label={playing ? t.arch_tour_pause : t.arch_tour_play} title={playing ? t.arch_tour_pause : t.arch_tour_play}
          onClick={() => {
            // 在最後一步按播放＝從頭再來。
            if (!playing && last) onIndex(0);
            onPlaying(!playing);
          }}>{playing ? "⏸" : "▶"}</button>
        <button type="button" className="arch-btn arch-btn-small" aria-label={t.arch_tour_next} title={t.arch_tour_next} disabled={last} onClick={() => onIndex(index + 1)}>⏭</button>
        <button type="button" className="arch-btn arch-btn-small" aria-label={t.arch_tour_close} title={t.arch_tour_close} onClick={onClose}>✕</button>
      </div>
    </div>
  );
}
