import { useLocale } from "../contexts/LocaleContext";
import { useUpdaterContext } from "../contexts/UpdaterContext";
import { openUrl } from "../ipc/shell";
import type { UpdaterState } from "../hooks/useUpdater";
import "./UpdateModal.css";

const RELEASES_URL = "https://github.com/jamesju9999/AITERM/releases/latest";

interface UpdateModalViewProps {
  state: UpdaterState;
  dismissed: boolean;
  onInstall: () => void;
  onDismiss: () => void;
  onRelaunch: () => void;
  onOpenReleases: () => void;
}

export function UpdateModalView({
  state,
  dismissed,
  onInstall,
  onDismiss,
  onRelaunch,
  onOpenReleases,
}: UpdateModalViewProps) {
  const { t } = useLocale();

  const visible =
    !dismissed &&
    (state.status === "available" ||
      state.status === "downloading" ||
      state.status === "ready" ||
      state.status === "unsupported" ||
      state.status === "error");

  if (!visible) return null;

  // total is distinct from 0: a real contentLength of 0 is not "unknown", but
  // dividing by it would produce NaN/Infinity, so both are treated as "no
  // percentage to show" rather than truthy-testing total (which would also be
  // correct here but conflates the two cases less explicitly).
  const percent =
    state.status === "downloading" && state.total !== null && state.total > 0
      ? Math.min(100, Math.round((state.downloaded / state.total) * 100))
      : null;

  return (
    <div className="update-modal-backdrop">
      <div className="update-modal" role="dialog" aria-label={t.update_modal_title}>
        <p className="update-modal-title">
          {state.status === "error"
            ? t.update_failed
            : state.status === "ready"
              ? t.update_ready
              : t.update_modal_title}
        </p>

        {"version" in state && <p className="update-modal-version">v{state.version}</p>}

        {state.status === "available" && state.notes && (
          <p className="update-modal-notes">{state.notes}</p>
        )}

        {state.status === "unsupported" && (
          <p className="update-modal-notes">{t.update_manual_hint}</p>
        )}

        {/*
          Not localized, and deliberately so: this is the raw error from the
          Rust side (e.g. "signature error: invalid signature"). The localized
          title above says what failed; this line says why, and mistranslating
          or hiding it would make update failures undiagnosable.
        */}
        {state.status === "error" && (
          <p className="update-modal-error">{state.message}</p>
        )}

        {state.status === "downloading" && (
          <>
            <div className="update-modal-progress">
              <div
                className={
                  percent === null
                    ? "update-modal-progress-bar update-modal-progress-bar--indeterminate"
                    : "update-modal-progress-bar"
                }
                style={percent === null ? undefined : { width: `${percent}%` }}
              />
            </div>
            <p className="update-modal-notes">
              {t.update_downloading}
              {percent !== null && ` ${percent}%`}
            </p>
          </>
        )}

        {state.status === "ready" && (
          <p className="update-modal-warning">{t.update_restart_warning}</p>
        )}

        <div className="update-modal-actions">
          {state.status !== "downloading" && (
            <button className="aiterm-btn aiterm-btn--secondary" onClick={onDismiss}>
              {t.update_later}
            </button>
          )}

          {state.status === "available" && (
            <button className="aiterm-btn aiterm-btn--primary" onClick={onInstall}>
              {t.update_now}
            </button>
          )}

          {state.status === "ready" && (
            <button className="aiterm-btn aiterm-btn--primary" onClick={onRelaunch}>
              {t.update_restart_now}
            </button>
          )}

          {state.status === "unsupported" && (
            <button className="aiterm-btn aiterm-btn--primary" onClick={onOpenReleases}>
              {t.about_update_link}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

export function UpdateModal() {
  const { state, dismissed, dismiss, install, relaunch } = useUpdaterContext();

  return (
    <UpdateModalView
      state={state}
      dismissed={dismissed}
      onInstall={() => void install()}
      onDismiss={dismiss}
      onRelaunch={() => void relaunch()}
      onOpenReleases={() => openUrl(RELEASES_URL).catch(console.error)}
    />
  );
}
