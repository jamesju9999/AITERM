# About Page — Design Spec

**Date:** 2026-04-22
**Feature:** Settings → About tab

---

## Overview

Add a dedicated "About" tab to the Settings sidebar. The page displays the app icon, name, version number, GitHub link, a check-for-updates button, and copyright.

---

## UI Layout

```
┌──────────────────────────────────┐
│                                  │
│        [ icon 128×128 ]          │
│            AITerm                │
│           v0.1.29                │
│                                  │
│   [ GitHub ]   [ 檢查更新 ]      │
│                                  │
│     （狀態訊息區）                │
│                                  │
│        © 2025 AITerm             │
└──────────────────────────────────┘
```

- Icon: `src-tauri/icons/128x128.png`, imported via Vite asset handling
- App name: hardcoded "AITerm"
- Version: fetched at mount via `getVersion()` from `@tauri-apps/api/app`
- Buttons: GitHub link + Check for Updates, side by side
- Status area: shows update check result (hidden until triggered)
- Copyright: static string, year 2025

---

## Settings Sidebar Change

`SettingsView.tsx` — add a 4th tab:

```
type SettingsTab = "general" | "providers" | "databases" | "about"
```

Sidebar button: `ℹ️ 關於 / About`, inserted after "資料庫連線", before the spacer.

---

## Components

### `AboutPage.tsx`

- Calls `getVersion()` on mount, stores in local state
- Renders the layout above
- Handles GitHub button click → calls `open_url` Tauri command
- Handles check-for-updates button click → fetches GitHub API

### `AboutPage.css`

Minimal styles: centered column layout, icon sizing, button styles consistent with existing settings pages.

---

## Version Source

`getVersion()` from `@tauri-apps/api/app` reads from `tauri.conf.json`. The CI workflow (`release.yml`) already syncs `tauri.conf.json` version from the git tag before building, so the displayed version always matches the release tag.

---

## Open URL — Rust Command

Add to `src-tauri/`:

**`Cargo.toml`** — add dependency:
```toml
open = "5"
```

**`commands.rs`** — add command:
```rust
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    open::that(url).map_err(|e| e.to_string())
}
```

Register in `lib.rs` alongside existing commands.

---

## Check for Updates

Frontend fetches:
```
GET https://api.github.com/repos/jamesju9999/AITERM/releases/latest
```

Response field used: `tag_name` (e.g. `"v0.1.30"`).

Comparison logic:
1. Strip leading `v` from both current version and `tag_name`
2. Compare as strings (semver ordering not needed — tags are always newer if different)

Three UI states in the status area:

| State | Display |
|-------|---------|
| Checking | 🔄 檢查中… / Checking… |
| Up to date | ✅ 已是最新版 / Up to date |
| Update available | 🆕 有新版本 vX.X.X — 點此前往下載 (clickable, opens GitHub releases page) |

Error state (network failure): show a short error string, no crash.

---

## i18n

Add to both `zh-TW` and `en` in `src/lib/i18n.ts`:

| Key | zh-TW | en |
|-----|-------|----|
| `about` | 關於 | About |
| `about_version` | 版本 | Version |
| `about_check_updates` | 檢查更新 | Check for Updates |
| `about_checking` | 🔄 檢查中… | 🔄 Checking… |
| `about_up_to_date` | ✅ 已是最新版 | ✅ Up to date |
| `about_update_available` | 🆕 有新版本 | 🆕 New version available |
| `about_update_link` | 點此前往下載 | Click to download |
| `about_update_error` | 檢查失敗，請稍後再試 | Check failed, please try again |
| `about_github` | GitHub | GitHub |
| `about_copyright` | © 2025 AITerm | © 2025 AITerm |

---

## Files Changed

| File | Change |
|------|--------|
| `src/components/Settings/SettingsView.tsx` | Add `about` tab type + sidebar button + content render |
| `src/components/Settings/AboutPage.tsx` | New component |
| `src/components/Settings/AboutPage.css` | New styles |
| `src/lib/i18n.ts` | Add About strings to both locales |
| `src-tauri/Cargo.toml` | Add `open = "5"` |
| `src-tauri/src/commands.rs` | Add `open_url` command |
| `src-tauri/src/lib.rs` | Register `open_url` in invoke handler |

---

## Out of Scope

- Auto-update / download: only links to the GitHub releases page
- Build number or git SHA display
- Licenses page
