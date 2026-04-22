import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useLocale } from "../../contexts/LocaleContext";
import { openUrl } from "../../ipc/shell";
import iconUrl from "../../../src-tauri/icons/128x128.png";
import "./AboutPage.css";

const GITHUB_URL = "https://github.com/jamesju9999/AITERM";
const RELEASES_API = "https://api.github.com/repos/jamesju9999/AITERM/releases/latest";
const RELEASES_URL = "https://github.com/jamesju9999/AITERM/releases/latest";

type UpdateStatus = "idle" | "checking" | "up-to-date" | "available" | "error";

export function AboutPage() {
  const { t } = useLocale();
  const [version, setVersion] = useState<string>("…");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [latestVersion, setLatestVersion] = useState<string>("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("—"));
  }, []);

  const handleGitHub = () => {
    openUrl(GITHUB_URL).catch(console.error);
  };

  const handleCheckUpdates = async () => {
    setUpdateStatus("checking");
    try {
      const res = await fetch(RELEASES_API);
      if (!res.ok) throw new Error("network");
      const data = await res.json();
      const latest = (data.tag_name as string).replace(/^v/, "");
      const current = version.replace(/^v/, "");
      setLatestVersion(latest);
      // String equality (not semver): GitHub's /releases/latest excludes pre-releases,
      // so any mismatch reliably means a newer stable release is available.
      setUpdateStatus(latest === current ? "up-to-date" : "available");
    } catch {
      setUpdateStatus("error");
    }
  };

  const statusText = () => {
    switch (updateStatus) {
      case "checking": return <span>{t.about_checking}</span>;
      case "up-to-date": return <span>{t.about_up_to_date}</span>;
      case "available":
        return (
          <span>
            {t.about_update_available} v{latestVersion} —{" "}
            <button
              className="about-link-btn"
              onClick={() => openUrl(RELEASES_URL).catch(console.error)}
            >
              {t.about_update_link}
            </button>
          </span>
        );
      case "error": return <span>{t.about_update_error}</span>;
      default: return null;
    }
  };

  return (
    <div className="about-page">
      <img src={iconUrl} alt="AITerm" className="about-icon" />
      <p className="about-name">AITerm</p>
      <p className="about-version">v{version}</p>

      <div className="about-buttons">
        <button className="about-btn" onClick={handleGitHub}>
          {t.about_github}
        </button>
        <button
          className="about-btn"
          onClick={handleCheckUpdates}
          disabled={updateStatus === "checking" || version === "…"}
        >
          {t.about_check_updates}
        </button>
      </div>

      <div className="about-status">{statusText()}</div>

      <p className="about-copyright">{t.about_copyright}</p>
    </div>
  );
}
