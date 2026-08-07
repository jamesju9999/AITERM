# About Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "About" tab to Settings that displays the app icon, name, version, GitHub link, and a check-for-updates button.

**Architecture:** New `AboutPage.tsx` component reads app version via Tauri's `getVersion()` API; a new Rust command `open_url` (using the `open` crate) handles opening browser URLs; update check fetches GitHub Releases API directly from the frontend.

**Tech Stack:** React 19, TypeScript, Tauri 2, `@tauri-apps/api/app` (getVersion), `open` Rust crate (v5)

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src-tauri/Cargo.toml` | Modify | Add `open = "5"` dependency |
| `src-tauri/src/commands/shell.rs` | Create | `open_url` Tauri command |
| `src-tauri/src/commands/mod.rs` | Modify | Expose `shell` module |
| `src-tauri/src/lib.rs` | Modify | Import + register `open_url` |
| `src/ipc/shell.ts` | Create | Frontend IPC wrapper for `open_url` |
| `src/lib/i18n.ts` | Modify | Add About i18n strings (both locales) |
| `src/components/Settings/AboutPage.tsx` | Create | About page UI component |
| `src/components/Settings/AboutPage.css` | Create | About page styles |
| `src/components/Settings/SettingsView.tsx` | Modify | Add `about` tab type, sidebar button, content render |

---

### Task 1: Add `open_url` Rust command

**Files:**
- Create: `src-tauri/src/commands/shell.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add `open` crate to Cargo.toml**

In `src-tauri/Cargo.toml`, add after the `reqwest` line:

```toml
open = "5"
```

- [ ] **Step 2: Create `src-tauri/src/commands/shell.rs`**

```rust
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    open::that(url).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Expose the module in `src-tauri/src/commands/mod.rs`**

Add `pub mod shell;` so the file becomes:

```rust
pub mod ai;
pub mod config;
pub mod db;
pub mod provider;
pub mod secret;
pub mod shell;
```

- [ ] **Step 4: Register `open_url` in `src-tauri/src/lib.rs`**

Add the import at the top of the use block (after `commands::secret::{delete_api_key, has_api_key},`):

```rust
commands::shell::open_url,
```

Add `open_url,` inside `tauri::generate_handler![...]` (after `delete_api_key,` for example):

```rust
// Shell
open_url,
```

- [ ] **Step 5: Verify Rust compiles**

```bash
cd src-tauri && cargo check
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/commands/shell.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(shell): add open_url Tauri command"
```

---

### Task 2: Frontend IPC wrapper + i18n strings

**Files:**
- Create: `src/ipc/shell.ts`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Create `src/ipc/shell.ts`**

```typescript
import { invoke } from "@tauri-apps/api/core";

export function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}
```

- [ ] **Step 2: Add About strings to `src/lib/i18n.ts`**

In the `zh-TW` object (after the `ProvidersPage` section, before the closing `}`):

```typescript
// AboutPage
about: "關於",
about_check_updates: "檢查更新",
about_checking: "🔄 檢查中…",
about_up_to_date: "✅ 已是最新版",
about_update_available: "🆕 有新版本",
about_update_link: "點此前往下載",
about_update_error: "檢查失敗，請稍後再試",
about_github: "GitHub",
about_copyright: "© 2025 AITerm",
```

In the `en` object (same position):

```typescript
// AboutPage
about: "About",
about_check_updates: "Check for Updates",
about_checking: "🔄 Checking…",
about_up_to_date: "✅ Up to date",
about_update_available: "🆕 New version available",
about_update_link: "Click to download",
about_update_error: "Check failed, please try again",
about_github: "GitHub",
about_copyright: "© 2025 AITerm",
```

- [ ] **Step 3: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/ipc/shell.ts src/lib/i18n.ts
git commit -m "feat(about): add IPC wrapper and i18n strings"
```

---

### Task 3: AboutPage component + styles

**Files:**
- Create: `src/components/Settings/AboutPage.tsx`
- Create: `src/components/Settings/AboutPage.css`

- [ ] **Step 1: Create `src/components/Settings/AboutPage.css`**

```css
.about-page {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
  height: 100%;
  gap: 12px;
  color: #e6e6e6;
  font-family: "Consolas", "Cascadia Code", "Menlo", monospace;
}

.about-icon {
  width: 96px;
  height: 96px;
  border-radius: 20px;
  margin-bottom: 8px;
}

.about-name {
  font-size: 24px;
  font-weight: 700;
  margin: 0;
}

.about-version {
  font-size: 14px;
  color: #888;
  margin: 0;
}

.about-buttons {
  display: flex;
  gap: 12px;
  margin-top: 8px;
}

.about-btn {
  background: #1e2a42;
  border: none;
  border-radius: 6px;
  color: #7aadff;
  cursor: pointer;
  font-family: inherit;
  font-size: 13px;
  padding: 8px 18px;
}

.about-btn:hover {
  background: #263450;
}

.about-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.about-status {
  font-size: 13px;
  color: #aaa;
  min-height: 20px;
  text-align: center;
}

.about-status a {
  color: #7aadff;
  cursor: pointer;
  text-decoration: underline;
}

.about-copyright {
  font-size: 12px;
  color: #555;
  margin-top: 16px;
}
```

- [ ] **Step 2: Create `src/components/Settings/AboutPage.tsx`**

```typescript
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
  const [version, setVersion] = useState<string>("");
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
            <a onClick={() => openUrl(RELEASES_URL).catch(console.error)}>
              {t.about_update_link}
            </a>
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
          disabled={updateStatus === "checking" || version === ""}
        >
          {t.about_check_updates}
        </button>
      </div>

      <div className="about-status">{statusText()}</div>

      <p className="about-copyright">{t.about_copyright}</p>
    </div>
  );
}
```

- [ ] **Step 3: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors. If Vite complains about the PNG import path (`../../../src-tauri/icons/128x128.png`), add a declaration in `src/vite-env.d.ts`:

```typescript
declare module "*.png" {
  const src: string;
  export default src;
}
```

- [ ] **Step 4: Commit**

```bash
git add src/components/Settings/AboutPage.tsx src/components/Settings/AboutPage.css
git commit -m "feat(about): add AboutPage component and styles"
```

---

### Task 4: Wire About tab into SettingsView

**Files:**
- Modify: `src/components/Settings/SettingsView.tsx`

- [ ] **Step 1: Update `SettingsView.tsx`**

Replace the entire file content with:

```typescript
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProvidersPage } from "./ProvidersPage";
import { GeneralPage } from "./GeneralPage";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { AboutPage } from "./AboutPage";
import { useLocale } from "../../contexts/LocaleContext";
import "./SettingsView.css";

type SettingsTab = "general" | "providers" | "databases" | "about";

export function SettingsView() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<SettingsTab>("general");
  const { t } = useLocale();

  return (
    <div className="settings-view">
      {/* Sidebar */}
      <nav className="settings-sidebar">
        <div className="settings-sidebar-title">{t.settings_title}</div>

        <button
          className={`sidebar-item ${tab === "general" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("general")}
        >
          ⚙️ {t.general}
        </button>
        <button
          className={`sidebar-item ${tab === "providers" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("providers")}
        >
          🤖 {t.ai_providers}
        </button>
        <button
          className={`sidebar-item ${tab === "databases" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("databases")}
        >
          🗄️ {t.db_connections}
        </button>
        <button
          className={`sidebar-item ${tab === "about" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("about")}
        >
          ℹ️ {t.about}
        </button>

        <div className="sidebar-spacer" />

        <button className="sidebar-back" onClick={() => navigate("/")}>
          {t.back_to_terminal}
        </button>
      </nav>

      {/* Content */}
      <main className="settings-content">
        {tab === "general" && <GeneralPage />}
        {tab === "providers" && <ProvidersPage />}
        {tab === "databases" && <DatabaseConnectionsPage />}
        {tab === "about" && <AboutPage />}
      </main>
    </div>
  );
}
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 3: Smoke test in dev**

```bash
npm run tauri:dev
```

Open Settings → click "ℹ️ 關於 / About" → verify icon, version, GitHub button, and check-for-updates all render correctly.

- [ ] **Step 4: Commit**

```bash
git add src/components/Settings/SettingsView.tsx
git commit -m "feat(settings): add About tab to settings sidebar"
```
