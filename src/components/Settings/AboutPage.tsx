import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useLocale } from "../../contexts/LocaleContext";
import { useUpdaterContext } from "../../contexts/UpdaterContext";
import { downloadPercent } from "../../hooks/useUpdater";
import { openUrl } from "../../ipc/shell";
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL } from "../../lib/repo";
import iconUrl from "../../../src-tauri/icons/128x128.png";
import "./AboutPage.css";

export function AboutPage() {
  const { t } = useLocale();
  const { state, check, install, relaunch } = useUpdaterContext();
  const [version, setVersion] = useState<string>("…");

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => setVersion("—"));
  }, []);

  const handleGitHub = () => {
    openUrl(GITHUB_REPO_URL).catch(console.error);
  };

  const percent = downloadPercent(state);

  const statusText = () => {
    switch (state.status) {
      case "idle":
        return <span>{t.about_update_not_checked}</span>;
      case "checking":
        return <span>{t.about_checking}</span>;
      case "none":
        return <span>{t.about_up_to_date}</span>;
      case "available":
        return (
          <span>
            {t.about_update_available} v{state.version} —{" "}
            <button className="about-link-btn" onClick={() => void install()}>
              {t.update_now}
            </button>
          </span>
        );
      case "downloading":
        return (
          <span>
            {t.update_downloading}
            {percent !== null && ` ${percent}%`}
          </span>
        );
      case "ready":
        return (
          <span>
            {t.update_ready} v{state.version} —{" "}
            <button className="about-link-btn" onClick={() => void relaunch()}>
              {t.update_restart_now}
            </button>
          </span>
        );
      case "unsupported":
        return (
          <span>
            {t.about_update_available} v{state.version} —{" "}
            <button
              className="about-link-btn"
              onClick={() => openUrl(GITHUB_RELEASES_URL).catch(console.error)}
            >
              {t.about_update_link}
            </button>
          </span>
        );
      case "error":
        return <span>{state.phase === "check" ? t.update_check_failed : t.update_failed}</span>;
      default:
        return null;
    }
  };

  return (
    <div className="about-page">
      <img src={iconUrl} alt="AITerm" className="about-icon" />
      <p className="about-name">AITerm</p>
      <p className="about-version">v{version}</p>
      <p className="about-author">by James Chu</p>
      <p className="about-email">
        <a href="mailto:jamesjulive@gmail.com" className="about-link-btn">jamesjulive@gmail.com</a>
      </p>

      <div className="about-buttons">
        <button className="aiterm-btn aiterm-btn--primary" onClick={handleGitHub}>
          {t.about_github}
        </button>
        <button
          className="aiterm-btn aiterm-btn--primary"
          onClick={() => void check()}
          disabled={
            state.status === "checking" ||
            state.status === "downloading" ||
            state.status === "ready"
          }
        >
          {t.about_check_updates}
        </button>
      </div>

      <div className="about-status">{statusText()}</div>

      <p className="about-copyright">{t.about_copyright}</p>
    </div>
  );
}
