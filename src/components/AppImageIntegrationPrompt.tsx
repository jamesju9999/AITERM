import { useEffect, useState } from "react";
import {
  appimageIntegrationState,
  appimageIntegrate,
  isAppImageIntegrationDeclined,
  setAppImageIntegrationDeclined,
  isOnboardingDone,
} from "../ipc/config";
import { useLocale } from "../contexts/LocaleContext";
// Deliberate reuse: this prompt is visually the same corner card as the update
// toast, and a second copy of those rules would drift from it.
import "./UpdateModal.css";

interface Props {
  /** The update prompt occupies the same corner and takes precedence. */
  hasUpdate: boolean;
}

export function AppImageIntegrationPrompt({ hasUpdate }: Props) {
  const { t } = useLocale();
  const [offer, setOffer] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [state, declined, onboarded] = await Promise.all([
          appimageIntegrationState(),
          isAppImageIntegrationDeclined(),
          isOnboardingDone(),
        ]);
        if (cancelled) return;
        setOffer(state.state === "available" && !declined && onboarded);
      } catch {
        // Best-effort: a failure here must never block the app.
      }
    })();
    return () => { cancelled = true; };
  }, []);

  if (!offer || hasUpdate) return null;

  const accept = async () => {
    try {
      await appimageIntegrate();
      setOffer(false);
    } catch (e) {
      setError(String(e));
    }
  };

  const decline = async () => {
    setOffer(false);
    await setAppImageIntegrationDeclined().catch(() => {});
  };

  return (
    <div className="update-modal-backdrop">
      <div className="update-modal" role="status" aria-label={t.appimage_prompt_title}>
        <p className="update-modal-title">{t.appimage_prompt_title}</p>
        <p className="update-modal-notes">{t.appimage_prompt_body}</p>
        <p className="update-modal-notes">{t.appimage_prompt_paths}</p>
        {error && <p className="update-modal-error">{error}</p>}
        <div className="update-modal-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void decline()}>
            {t.appimage_decline}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" onClick={() => void accept()}>
            {t.appimage_create}
          </button>
        </div>
      </div>
    </div>
  );
}
