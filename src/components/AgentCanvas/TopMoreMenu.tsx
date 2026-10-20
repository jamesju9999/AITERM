import { useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";

/** 頂列的「⋯」：不常用的資訊與說明，不跟每天要按的按鈕搶位置。 */
export function TopMoreMenu({ cap, onHelp }: { cap: number | null; onHelp: () => void }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className="ac-role-menu-wrap" ref={ref}>
      <button
        className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm ac-more-btn"
        data-guide="help"
        aria-label={t.ac_more}
        title={t.ac_more}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        ⋯
      </button>
      {open && (
        <div className="ac-role-menu ac-more-menu" role="menu">
          <button
            type="button"
            role="menuitem"
            className="ac-role-menu-item"
            onClick={() => {
              setOpen(false);
              onHelp();
            }}
          >
            {t.ac_help_button}
          </button>
          {cap !== null && (
            <>
              <div className="ac-role-menu-sep" />
              <div className="ac-role-menu-label" title={t.ac_cap_tip}>
                {t.ac_cap(cap)}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
