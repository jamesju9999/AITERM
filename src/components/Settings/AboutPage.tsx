import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useLocale } from "../../contexts/LocaleContext";
import { openUrl } from "../../ipc/shell";
import iconUrl from "../../../src-tauri/icons/128x128.png";
import "./AboutPage.css";

const GITHUB_URL = "https://github.com/jamesju9999/AITERM";
// Use tags API instead of /releases/latest so draft releases are included in version tracking.
const TAGS_API = "https://api.github.com/repos/jamesju9999/AITERM/tags";
const RELEASES_URL = "https://github.com/jamesju9999/AITERM/releases";

type UpdateStatus = "idle" | "checking" | "up-to-date" | "available" | "error";

export function AboutPage() {
  const { t } = useLocale();
  const [version, setVersion] = useState<string>("…");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [latestVersion, setLatestVersion] = useState<string>("");

  const checkUpdates = async (currentVersion: string) => {
    setUpdateStatus("checking");
    try {
      const res = await fetch(TAGS_API);
      if (!res.ok) throw new Error("network");
      const tags = await res.json() as { name: string }[];
      if (tags.length === 0) {
        setUpdateStatus("up-to-date");
        return;
      }
      const latest = tags[0].name.replace(/^v/, "");
      const current = currentVersion.replace(/^v/, "");
      setLatestVersion(latest);
      // String equality (not semver): tags are sorted newest-first by GitHub,
      // so any mismatch reliably means a newer version is available.
      setUpdateStatus(latest === current ? "up-to-date" : "available");
    } catch {
      setUpdateStatus("error");
    }
  };

  useEffect(() => {
    getVersion()
      .then((v) => { setVersion(v); checkUpdates(v); })
      .catch(() => setVersion("—"));
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleGitHub = () => {
    openUrl(GITHUB_URL).catch(console.error);
  };

  const handleCheckUpdates = () => checkUpdates(version);

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
      <p className="about-author">by James Chu</p>
      <p className="about-email">
        <a href="mailto:jamesjulive@gmail.com" className="about-link-btn">jamesjulive@gmail.com</a>
      </p>

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
