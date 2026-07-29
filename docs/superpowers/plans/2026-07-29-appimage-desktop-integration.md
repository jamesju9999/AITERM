# AppImage Desktop Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user running the AppImage opt into a system menu entry, so the dock shows AITerm's icon instead of a generic gear and the app appears in the application menu.

**Architecture:** Three Tauri commands report state, install, and remove an entry in `~/.local/share/applications`. The `.desktop` is **copied** from the AppDir and only its `Exec=` and `Icon=` lines are rewritten — everything else, including the load-bearing `StartupWMClass`, is carried through. All rewriting logic lives in pure functions so it is testable on macOS. A startup hook repairs `Exec=` when the AppImage has moved. The frontend adds a first-run prompt and a Settings section that both installs and removes.

**Tech Stack:** Rust (Tauri 2, `#[cfg(target_os = "linux")]`), React 19 + TypeScript, Vitest + React Testing Library.

**Spec:** `docs/superpowers/specs/2026-07-29-appimage-desktop-integration-design.md`

**Why this exists (do not "fix" the wrong thing):** Two plausible causes were ruled out empirically. The window *already* carries a correct `_NET_WM_ICON` (32×32, verified with `xprop`), so setting a window icon in Rust would change nothing. And `StartupWMClass=app` in the bundled `.desktop` *already matches* the window's real `WM_CLASS(STRING) = "app", "App"`. The only problem is that an AppImage never registers its `.desktop` with the system, and GNOME does not fall back to `_NET_WM_ICON` for unmatched windows.

---

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/src/commands/appimage.rs` (new) | Pure rewrite/parse helpers + the three commands + auto-repair |
| `src-tauri/src/commands/mod.rs` | Register the module |
| `src-tauri/src/lib.rs` | Register commands; call auto-repair in `setup` |
| `src-tauri/src/config/types.rs` | `appimage_integration_declined` flag |
| `src-tauri/src/commands/config.rs` | Getter/setter commands for the flag |
| `src/ipc/config.ts` | IPC wrappers |
| `src/lib/i18n.ts` | Strings, both locales |
| `src/components/AppImageIntegrationPrompt.tsx` (new) | First-run prompt |
| `src/components/Settings/GeneralPage.tsx` | Settings section |

---

## Task 1: Pure `.desktop` rewriting

Everything that can be got wrong lives here, and none of it needs a real AppImage. `$APPIMAGE` only exists inside a running AppImage, so if this logic is welded to the filesystem it cannot be tested at all on macOS. Same pattern as `commands/updater.rs::supported_for` and `pty/commands.rs::drives_from_mask`.

**Files:**
- Create: `src-tauri/src/commands/appimage.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/commands/appimage.rs` with stubs and tests:

```rust
/// The `.desktop` filename and icon name we install under. Deliberately not
/// "app": that name is far too generic to put in a shared icon theme, and it
/// would collide with any other project that ships an `app.png`.
#[cfg(any(target_os = "linux", test))]
const ENTRY_NAME: &str = "aiterm";

/// Rewrites the AppDir's `.desktop` for installation into the user's
/// applications directory.
///
/// Only `Exec=` and `Icon=` change. Everything else is carried through — the
/// bundler already generated `Name`, `Comment` and `Categories` from
/// tauri.conf.json, and rewriting them here would create a second source that
/// drifts the moment the config changes.
///
/// `StartupWMClass` in particular MUST survive untouched. It is `app`, derived
/// from the Rust binary name, and it is what GNOME matches the window against.
/// Renaming it to match ENTRY_NAME would leave a menu entry that looks correct
/// while the dock still shows a generic icon — a failure with no visible symptom.
#[cfg(any(target_os = "linux", test))]
fn rewrite_desktop(source: &str, appimage_path: &str) -> String {
    let _ = (source, appimage_path);
    String::new()
}

/// The path in a `.desktop`'s `Exec=` line, unquoted and without trailing
/// field codes. `None` when there is no `Exec=` line.
#[cfg(any(target_os = "linux", test))]
fn exec_path_of(desktop: &str) -> Option<String> {
    let _ = desktop;
    None
}

/// Updated contents when `Exec=` no longer points at `appimage_path`, or `None`
/// when it already matches and nothing needs writing.
#[cfg(any(target_os = "linux", test))]
fn repair_exec(desktop: &str, appimage_path: &str) -> Option<String> {
    let _ = (desktop, appimage_path);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the bundler actually produces inside the AppDir, verified by
    /// `cat squashfs-root/*.desktop` on a real build.
    const APPDIR_DESKTOP: &str = "\
[Desktop Entry]
Categories=Development;
Comment=AI TERM Studio
Exec=app
StartupWMClass=app
Icon=app
Name=AITerm
Terminal=false
Type=Application
";

    #[test]
    fn rewrite_points_exec_at_the_appimage() {
        let out = rewrite_desktop(APPDIR_DESKTOP, "/home/u/AITerm.AppImage");
        assert!(out.contains("Exec=\"/home/u/AITerm.AppImage\" %U"), "got:\n{out}");
    }

    #[test]
    fn rewrite_quotes_paths_containing_spaces() {
        // AppImages routinely sit in paths like ~/我的 下載/
        let out = rewrite_desktop(APPDIR_DESKTOP, "/home/u/my downloads/AITerm.AppImage");
        assert!(out.contains("Exec=\"/home/u/my downloads/AITerm.AppImage\" %U"), "got:\n{out}");
    }

    #[test]
    fn rewrite_points_icon_at_our_entry_name() {
        let out = rewrite_desktop(APPDIR_DESKTOP, "/x/A.AppImage");
        assert!(out.contains("Icon=aiterm"), "got:\n{out}");
        assert!(!out.contains("Icon=app\n"), "old icon line survived:\n{out}");
    }

    #[test]
    fn rewrite_preserves_startup_wm_class() {
        // The whole feature hinges on this staying "app" — see the doc comment.
        let out = rewrite_desktop(APPDIR_DESKTOP, "/x/A.AppImage");
        assert!(out.contains("StartupWMClass=app"), "got:\n{out}");
        assert!(!out.contains("StartupWMClass=aiterm"), "got:\n{out}");
    }

    #[test]
    fn rewrite_preserves_bundler_generated_metadata() {
        let out = rewrite_desktop(APPDIR_DESKTOP, "/x/A.AppImage");
        for line in ["Name=AITerm", "Comment=AI TERM Studio", "Categories=Development;"] {
            assert!(out.contains(line), "{line} missing from:\n{out}");
        }
    }

    #[test]
    fn exec_path_reads_a_quoted_path() {
        let d = "[Desktop Entry]\nExec=\"/home/u/my app/A.AppImage\" %U\n";
        assert_eq!(exec_path_of(d).as_deref(), Some("/home/u/my app/A.AppImage"));
    }

    #[test]
    fn exec_path_reads_an_unquoted_path() {
        // A user may have hand-edited the file.
        let d = "[Desktop Entry]\nExec=/home/u/A.AppImage %U\n";
        assert_eq!(exec_path_of(d).as_deref(), Some("/home/u/A.AppImage"));
    }

    #[test]
    fn exec_path_is_none_without_an_exec_line() {
        assert_eq!(exec_path_of("[Desktop Entry]\nName=X\n"), None);
    }

    #[test]
    fn repair_is_a_no_op_when_the_path_already_matches() {
        let d = "[Desktop Entry]\nExec=\"/x/A.AppImage\" %U\nIcon=aiterm\n";
        assert_eq!(repair_exec(d, "/x/A.AppImage"), None);
    }

    #[test]
    fn repair_rewrites_only_the_exec_line() {
        let d = "[Desktop Entry]\nExec=\"/old/A.AppImage\" %U\nStartupWMClass=app\nName=AITerm\n";
        let out = repair_exec(d, "/new/A.AppImage").expect("should need repair");
        assert!(out.contains("Exec=\"/new/A.AppImage\" %U"), "got:\n{out}");
        assert!(out.contains("StartupWMClass=app"), "got:\n{out}");
        assert!(out.contains("Name=AITerm"), "got:\n{out}");
        assert!(!out.contains("/old/"), "old path survived:\n{out}");
    }

    #[test]
    fn repair_is_none_when_there_is_nothing_to_repair() {
        // No Exec= line at all: the user mangled it, leave it alone.
        assert_eq!(repair_exec("[Desktop Entry]\nName=X\n", "/x/A.AppImage"), None);
    }
}
```

Register the module — add to `src-tauri/src/commands/mod.rs`, keeping the list alphabetical (before `pub mod api_docs;`):

```rust
pub mod appimage;
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
cd src-tauri && cargo test commands::appimage
```

Expected: `exec_path_is_none_without_an_exec_line`, `repair_is_a_no_op_when_the_path_already_matches` and `repair_is_none_when_there_is_nothing_to_repair` pass by accident (the stubs return `None`); the other eight FAIL.

- [ ] **Step 3: Implement**

Replace the three stub bodies:

```rust
#[cfg(any(target_os = "linux", test))]
fn rewrite_desktop(source: &str, appimage_path: &str) -> String {
    source
        .lines()
        .map(|line| {
            if line.starts_with("Exec=") {
                format!("Exec=\"{appimage_path}\" %U")
            } else if line.starts_with("Icon=") {
                format!("Icon={ENTRY_NAME}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[cfg(any(target_os = "linux", test))]
fn exec_path_of(desktop: &str) -> Option<String> {
    let value = desktop.lines().find_map(|l| l.strip_prefix("Exec="))?.trim();
    Some(if let Some(rest) = value.strip_prefix('"') {
        rest.split('"').next().unwrap_or("").to_string()
    } else {
        // Unquoted: the path runs until the first space, which is also where
        // any field code (%U, %F) would start.
        value.split(' ').next().unwrap_or("").to_string()
    })
}

#[cfg(any(target_os = "linux", test))]
fn repair_exec(desktop: &str, appimage_path: &str) -> Option<String> {
    let current = exec_path_of(desktop)?;
    if current == appimage_path {
        return None;
    }
    Some(
        desktop
            .lines()
            .map(|line| {
                if line.starts_with("Exec=") {
                    format!("Exec=\"{appimage_path}\" %U")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
}
```

- [ ] **Step 4: Run the tests, confirm all pass**

```bash
cd src-tauri && cargo test commands::appimage
```

Expected: `test result: ok. 11 passed`.

- [ ] **Step 5: Mutation-test — this is the acceptance criterion**

A green suite is not evidence. One at a time, edit the file, run the tests, confirm a FAILURE, then `git checkout -- src-tauri/src/commands/appimage.rs`:

| # | Mutation | Must be caught by |
|---|---|---|
| M1 | make `rewrite_desktop` also rewrite `StartupWMClass=` to `ENTRY_NAME` | `rewrite_preserves_startup_wm_class` |
| M2 | drop the quotes: `format!("Exec={appimage_path} %U")` | `rewrite_quotes_paths_containing_spaces` |
| M3 | `repair_exec` returns `Some(...)` even when the path matches | `repair_is_a_no_op_when_the_path_already_matches` |
| M4 | `rewrite_desktop` passes `Icon=` through unchanged | `rewrite_points_icon_at_our_entry_name` |
| M5 | `exec_path_of` keeps the surrounding quotes | `exec_path_reads_a_quoted_path` |

**M1 is the one that matters.** Renaming `StartupWMClass` produces a menu entry that looks completely correct while the dock keeps showing a gear — a failure with no visible symptom, which is exactly the kind this project has shipped before.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/appimage.rs src-tauri/src/commands/mod.rs
git commit -m "feat(appimage): add pure .desktop rewrite helpers"
```

---

## Task 2: Commands, auto-repair and registration

**Files:**
- Modify: `src-tauri/src/commands/appimage.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the state type and the three commands**

Append to `src-tauri/src/commands/appimage.rs`:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum IntegrationState {
    /// Not running as an AppImage — includes every non-Linux platform.
    NotAppimage,
    /// Running as an AppImage with no menu entry installed yet.
    Available,
    /// A menu entry exists; `exec_path` is what it currently points at.
    Integrated { exec_path: String },
}

#[cfg(target_os = "linux")]
mod paths {
    use std::path::PathBuf;

    pub fn desktop_file() -> Option<PathBuf> {
        Some(
            dirs::data_dir()?
                .join("applications")
                .join(format!("{}.desktop", super::ENTRY_NAME)),
        )
    }

    pub fn icon_dir() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("icons").join("hicolor"))
    }
}

#[tauri::command]
pub fn appimage_integration_state() -> IntegrationState {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("APPIMAGE").is_err() {
            return IntegrationState::NotAppimage;
        }
        if let Some(path) = paths::desktop_file() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Some(exec_path) = exec_path_of(&contents) {
                    return IntegrationState::Integrated { exec_path };
                }
            }
        }
        IntegrationState::Available
    }
    #[cfg(not(target_os = "linux"))]
    {
        IntegrationState::NotAppimage
    }
}
```

`appimage_integrate` copies the AppDir's `.desktop` through `rewrite_desktop`, then copies every icon size it finds:

```rust
#[tauri::command]
pub fn appimage_integrate() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let appimage = std::env::var("APPIMAGE").map_err(|_| "not running as an AppImage".to_string())?;
        let appdir = std::env::var("APPDIR").map_err(|_| "APPDIR is not set".to_string())?;

        // The bundler already generated this with the right Name, Comment,
        // Categories and StartupWMClass — copying keeps tauri.conf.json as the
        // single source rather than duplicating that metadata here.
        let src_dir = std::path::Path::new(&appdir).join("usr/share/applications");
        let source = std::fs::read_dir(&src_dir)
            .map_err(|e| format!("cannot read {}: {e}", src_dir.display()))?
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "desktop"))
            .ok_or_else(|| format!("no .desktop found in {}", src_dir.display()))?;
        let contents = std::fs::read_to_string(source.path()).map_err(|e| e.to_string())?;

        let target = paths::desktop_file().ok_or("cannot resolve the data directory")?;
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&target, rewrite_desktop(&contents, &appimage)).map_err(|e| e.to_string())?;

        // A missing icon is not fatal: the menu entry still works, it just
        // falls back to a generic image. Failing here would be worse.
        let _ = copy_icons(&appdir);
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("only supported on Linux".to_string())
    }
}

/// Copies every hicolor size the AppDir ships, renamed to ENTRY_NAME.
#[cfg(target_os = "linux")]
fn copy_icons(appdir: &str) -> std::io::Result<()> {
    let src_root = std::path::Path::new(appdir).join("usr/share/icons/hicolor");
    let dst_root = paths::icon_dir().ok_or(std::io::ErrorKind::NotFound)?;
    for size in std::fs::read_dir(&src_root)?.filter_map(|e| e.ok()) {
        let src = size.path().join("apps");
        let Ok(entries) = std::fs::read_dir(&src) else { continue };
        for icon in entries.filter_map(|e| e.ok()) {
            let ext = icon.path().extension().map(|e| e.to_owned());
            let Some(ext) = ext else { continue };
            let dst_dir = dst_root.join(size.file_name()).join("apps");
            std::fs::create_dir_all(&dst_dir)?;
            let dst = dst_dir.join(format!("{}.{}", ENTRY_NAME, ext.to_string_lossy()));
            std::fs::copy(icon.path(), dst)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn appimage_remove_integration() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(path) = paths::desktop_file() {
            // Idempotent: already absent is success, not an error.
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        if let Some(root) = paths::icon_dir() {
            if let Ok(sizes) = std::fs::read_dir(&root) {
                for size in sizes.filter_map(|e| e.ok()) {
                    for ext in ["png", "svg"] {
                        let p = size.path().join("apps").join(format!("{ENTRY_NAME}.{ext}"));
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("only supported on Linux".to_string())
    }
}

/// Called from `setup`: keeps an installed entry pointing at the AppImage's
/// current location after the user moves it or swaps in a new version.
///
/// Runs in the backend rather than the UI so it self-heals even for users who
/// dismissed the prompt and never open Settings.
pub fn repair_integration_on_startup() {
    #[cfg(target_os = "linux")]
    {
        let Ok(appimage) = std::env::var("APPIMAGE") else { return };
        let Some(path) = paths::desktop_file() else { return };
        let Ok(contents) = std::fs::read_to_string(&path) else { return };
        if let Some(updated) = repair_exec(&contents, &appimage) {
            let _ = std::fs::write(&path, updated);
        }
    }
}
```

- [ ] **Step 2: Register**

In `src-tauri/src/lib.rs`, add to the `use commands::{...}` block (alphabetically, before `api_docs`):

```rust
    appimage::{appimage_integrate, appimage_integration_state, appimage_remove_integration},
```

Call the repair in `setup` (currently lines 237-241):

```rust
        .setup(|app| {
            telegram::init(app.handle());
            enterprise::agent::init(app.handle());
            commands::appimage::repair_integration_on_startup();
            Ok(())
        })
```

and add the three commands to `generate_handler!`, after `updater_supported,`:

```rust
            updater_supported,
            appimage_integration_state,
            appimage_integrate,
            appimage_remove_integration,
```

- [ ] **Step 3: Verify**

```bash
cd src-tauri && cargo build 2>&1 | tail -3
cd src-tauri && cargo test commands::appimage
```

Expected: `Finished` with no new warnings beyond the 2 pre-existing ones, and 11 tests still passing.

On macOS the `#[cfg(target_os = "linux")]` blocks are not compiled, so `paths` and `copy_icons` produce no dead-code warnings (they are inside the cfg). If any new warning appears, report it rather than suppressing it.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/appimage.rs src-tauri/src/lib.rs
git commit -m "feat(appimage): add integration commands and startup path repair"
```

---

## Task 3: Config flag and IPC wrappers

**Files:**
- Modify: `src-tauri/src/config/types.rs`
- Modify: `src-tauri/src/commands/config.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/ipc/config.ts`

- [ ] **Step 1: Add the flag**

In `src-tauri/src/config/types.rs`, directly after the `onboarding_done` field:

```rust
    /// Set when the user declines the AppImage menu-entry prompt, so it is
    /// asked once rather than on every launch. Settings still offers it.
    #[serde(default)]
    pub appimage_integration_declined: bool,
```

`#[serde(default)]` matters: existing config files on disk have no such key and must keep loading.

- [ ] **Step 2: Add the commands**

In `src-tauri/src/commands/config.rs`, after `set_onboarding_done`:

```rust
#[tauri::command]
pub fn is_appimage_integration_declined(config: State<Arc<ConfigStore>>) -> bool {
    config.get().appimage_integration_declined
}

#[tauri::command]
pub fn set_appimage_integration_declined(config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config
        .update(|cfg| { cfg.appimage_integration_declined = true; })
        .map_err(|e| e.to_string())
}
```

Register both in `lib.rs` — add to the `config::{...}` import and to `generate_handler!` beside the existing `is_onboarding_done` / `set_onboarding_done` entries.

- [ ] **Step 3: Add the IPC wrappers**

In `src/ipc/config.ts`, after the onboarding wrappers (around line 64-68):

```ts
export type AppImageIntegrationState =
  | { state: "not_appimage" }
  | { state: "available" }
  | { state: "integrated"; exec_path: string };

export const appimageIntegrationState = (): Promise<AppImageIntegrationState> =>
  invoke<AppImageIntegrationState>("appimage_integration_state");

export const appimageIntegrate = (): Promise<void> =>
  invoke<void>("appimage_integrate");

export const appimageRemoveIntegration = (): Promise<void> =>
  invoke<void>("appimage_remove_integration");

export const isAppImageIntegrationDeclined = (): Promise<boolean> =>
  invoke<boolean>("is_appimage_integration_declined");

export const setAppImageIntegrationDeclined = (): Promise<void> =>
  invoke<void>("set_appimage_integration_declined");
```

The `state` discriminant must match the Rust `#[serde(tag = "state")]`.

- [ ] **Step 4: Verify**

```bash
cd src-tauri && cargo test config
npx tsc --noEmit
```

Expected: existing config tests still pass (proving `#[serde(default)]` keeps old files loading), and no type errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs src-tauri/src/commands/config.rs \
        src-tauri/src/lib.rs src/ipc/config.ts
git commit -m "feat(appimage): persist the declined flag and add IPC wrappers"
```

---

## Task 4: i18n strings

`Translations` derives from the zh-TW object, so a key in only one locale is a type error. That check is the test.

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add to both locales**

Find `file_switch_drive` (it exists exactly twice). Insert after it in the **zh-TW** object:

```ts
    appimage_prompt_title: "建立應用程式選單項目？",
    appimage_prompt_body: "目前以 AppImage 執行，系統選單中沒有 AITerm，dock 也會顯示通用圖示。",
    appimage_prompt_paths: "會建立 ~/.local/share 底下的選單項目與圖示。",
    appimage_create: "建立",
    appimage_decline: "不用了",
    appimage_section_title: "應用程式選單項目",
    appimage_section_desc: "以 AppImage 執行時，建立選單項目後 dock 才會顯示正確圖示。",
    appimage_remove: "移除選單項目",
    appimage_current_path: "目前指向：",
    appimage_failed: "操作失敗",
```

and after it in the **en** object:

```ts
    appimage_prompt_title: "Create an application menu entry?",
    appimage_prompt_body: "You are running the AppImage. AITerm is not in the system menu, and the dock shows a generic icon.",
    appimage_prompt_paths: "This creates a menu entry and icons under ~/.local/share.",
    appimage_create: "Create",
    appimage_decline: "No thanks",
    appimage_section_title: "Application menu entry",
    appimage_section_desc: "When running as an AppImage, a menu entry is what makes the dock show the right icon.",
    appimage_remove: "Remove menu entry",
    appimage_current_path: "Currently points at:",
    appimage_failed: "Operation failed",
```

- [ ] **Step 2: Verify**

```bash
npx tsc --noEmit
grep -c "appimage_prompt_title" src/lib/i18n.ts
```

Expected: no type errors; grep returns `2`.

- [ ] **Step 3: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(appimage): add desktop integration strings for zh-TW and en"
```

---

## Task 5: First-run prompt

**Files:**
- Create: `src/components/AppImageIntegrationPrompt.tsx`
- Create: `src/components/AppImageIntegrationPrompt.test.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: Write the failing tests**

Create `src/components/AppImageIntegrationPrompt.test.tsx`:

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { AppImageIntegrationPrompt } from "./AppImageIntegrationPrompt";

const DEFAULTS: Record<string, unknown> = {
  appimage_integration_state: { state: "available" },
  is_appimage_integration_declined: false,
  is_onboarding_done: true,
};

function mockCommands(overrides: Record<string, unknown> = {}) {
  const table = { ...DEFAULTS, ...overrides };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in table ? table[cmd] : null),
  );
}

beforeEach(() => { invokeMock.mockReset(); });

describe("AppImageIntegrationPrompt", () => {
  it("offers the entry when running an un-integrated AppImage", async () => {
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate={false} />);

    expect(await screen.findByText("建立應用程式選單項目？")).toBeInTheDocument();
  });

  it("stays hidden on a non-AppImage install", async () => {
    mockCommands({ appimage_integration_state: { state: "not_appimage" } });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("stays hidden once already integrated", async () => {
    mockCommands({
      appimage_integration_state: { state: "integrated", exec_path: "/x/A.AppImage" },
    });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("stays hidden after the user declined once", async () => {
    mockCommands({ is_appimage_integration_declined: true });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("stays hidden until onboarding is done", async () => {
    // A brand-new user already has the onboarding wizard on screen.
    mockCommands({ is_onboarding_done: false });
    render(<AppImageIntegrationPrompt hasUpdate={false} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("yields to the update prompt", async () => {
    // Both live in the bottom-right corner; the update matters more.
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument();
  });

  it("installs the entry and closes when accepted", async () => {
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "建立" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("appimage_integrate"));
    await waitFor(() =>
      expect(screen.queryByText("建立應用程式選單項目？")).not.toBeInTheDocument(),
    );
  });

  it("records the refusal so it is not asked again", async () => {
    mockCommands();
    render(<AppImageIntegrationPrompt hasUpdate={false} />);

    await userEvent.click(await screen.findByRole("button", { name: "不用了" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_appimage_integration_declined"),
    );
  });
});
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
npx vitest run src/components/AppImageIntegrationPrompt.test.tsx
```

Expected: FAIL — `Failed to resolve import "./AppImageIntegrationPrompt"`.

- [ ] **Step 3: Implement**

Create `src/components/AppImageIntegrationPrompt.tsx`:

```tsx
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
```

- [ ] **Step 4: Mount it**

In `src/App.tsx`, import it and render beside `<UpdateModal />`:

```tsx
      <UpdateModal />
      <AppImageIntegrationPrompt hasUpdate={hasUpdate} />
    </div>
```

`hasUpdate` already comes from `useUpdaterContext()` in that component.

- [ ] **Step 5: Run the tests**

```bash
npx vitest run src/components/AppImageIntegrationPrompt.test.tsx
```

Expected: `Tests  8 passed`.

- [ ] **Step 6: Mutation-test**

Revert between each. All must FAIL:

| # | Mutation |
|---|---|
| P1 | drop `&& !declined` from the `setOffer` condition |
| P2 | drop `&& onboarded` |
| P3 | `if (!offer || hasUpdate)` → `if (!offer)` |
| P4 | `state.state === "available"` → `state.state !== "not_appimage"` |
| P5 | `decline` does not call `setAppImageIntegrationDeclined` |

- [ ] **Step 7: Verify and commit**

```bash
npx tsc --noEmit && npm run test
npx eslint src/components/AppImageIntegrationPrompt.tsx src/components/AppImageIntegrationPrompt.test.tsx src/App.tsx
```

Do NOT run bare `npm run lint` — this repo carries ~181 pre-existing problems.

```bash
git add src/components/AppImageIntegrationPrompt.tsx \
        src/components/AppImageIntegrationPrompt.test.tsx src/App.tsx
git commit -m "feat(appimage): prompt once to create a menu entry"
```

---

## Task 6: Settings section

**Files:**
- Modify: `src/components/Settings/GeneralPage.tsx`
- Create: `src/components/Settings/GeneralPage.appimage.test.tsx`

- [ ] **Step 1: Write the failing tests**

Create `src/components/Settings/GeneralPage.appimage.test.tsx` — a focused file rather than extending the existing suite, so the drive-switcher lesson about order-coupled mocks is not repeated:

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { GeneralPage } from "./GeneralPage";

/**
 * GeneralPage calls getConfig() on mount and reads cfg.execution_mode straight
 * off the result, so an unmocked get_config throws inside that promise and the
 * failure looks like it came from the section under test. Every case gets a
 * valid config by default.
 */
const BASE_CONFIG = {
  execution_mode: "graded",
  submit_shortcut: "enter",
  max_agent_steps: 5,
  default_tab: "terminal",
};

function mockCommands(table: Record<string, unknown>) {
  const full = { get_config: BASE_CONFIG, ...table };
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd in full ? full[cmd] : null),
  );
}

beforeEach(() => { invokeMock.mockReset(); });

describe("GeneralPage — AppImage integration", () => {
  it("hides the section entirely off AppImage", async () => {
    mockCommands({ appimage_integration_state: { state: "not_appimage" } });
    render(<GeneralPage />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByText("應用程式選單項目")).not.toBeInTheDocument();
  });

  it("offers to create when not yet integrated", async () => {
    mockCommands({ appimage_integration_state: { state: "available" } });
    render(<GeneralPage />);

    expect(await screen.findByText("應用程式選單項目")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "建立" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("appimage_integrate"));
  });

  it("shows the current path and offers removal when integrated", async () => {
    mockCommands({
      appimage_integration_state: { state: "integrated", exec_path: "/home/u/AITerm.AppImage" },
    });
    render(<GeneralPage />);

    expect(await screen.findByText(/\/home\/u\/AITerm\.AppImage/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "移除選單項目" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("appimage_remove_integration"),
    );
  });
});
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
npx vitest run src/components/Settings/GeneralPage.appimage.test.tsx
```

Expected: the two positive tests FAIL (`Unable to find an element with the text: 應用程式選單項目`); the "hides" test passes vacuously.

- [ ] **Step 3: Implement**

In `src/components/Settings/GeneralPage.tsx` — **first extend the React import on line 1**, which currently reads `import { useState, useEffect } from "react";` and lacks `useCallback`:

```tsx
import { useState, useEffect, useCallback } from "react";
```

Then add state and a loader:

```tsx
  const [appimage, setAppimage] = useState<AppImageIntegrationState>({ state: "not_appimage" });

  const loadAppimage = useCallback(() => {
    appimageIntegrationState().then(setAppimage).catch(() => {});
  }, []);
  useEffect(loadAppimage, [loadAppimage]);
```

and a section at the end of the returned JSX, before the closing tag:

```tsx
      {appimage.state !== "not_appimage" && (
        <section className="settings-section">
          <h3>{t.appimage_section_title}</h3>
          <p className="section-desc">{t.appimage_section_desc}</p>
          {appimage.state === "integrated" && (
            <p className="section-desc">
              {t.appimage_current_path} <code>{appimage.exec_path}</code>
            </p>
          )}
          <button
            className="aiterm-btn aiterm-btn--secondary"
            onClick={() => {
              const action =
                appimage.state === "integrated" ? appimageRemoveIntegration : appimageIntegrate;
              action().then(loadAppimage).catch(() => {});
            }}
          >
            {appimage.state === "integrated" ? t.appimage_remove : t.appimage_create}
          </button>
        </section>
      )}
```

Import the three IPC functions and the type from `../../ipc/config`.

- [ ] **Step 4: Run the tests**

```bash
npx vitest run src/components/Settings/GeneralPage.appimage.test.tsx
```

Expected: `Tests  3 passed`.

- [ ] **Step 5: Mutation-test**

Revert between each. All must FAIL:

| # | Mutation |
|---|---|
| S1 | render the section unconditionally (drop the `!== "not_appimage"` guard) |
| S2 | always call `appimageIntegrate`, never `appimageRemoveIntegration` |
| S3 | always label the button `t.appimage_create` |

- [ ] **Step 6: Verify and commit**

```bash
npx tsc --noEmit && npm run test
npx eslint src/components/Settings/GeneralPage.tsx src/components/Settings/GeneralPage.appimage.test.tsx
```

```bash
git add src/components/Settings/GeneralPage.tsx \
        src/components/Settings/GeneralPage.appimage.test.tsx
git commit -m "feat(appimage): add a Settings section to create or remove the entry"
```

---

## Task 7: Linux verification

`$APPIMAGE` and `$APPDIR` only exist inside a running AppImage, so nothing above proves the entry actually appears or that the dock icon changes. This task is the only thing that does.

**Files:** none (verification only)

- [ ] **Step 1: Full local verification**

```bash
npx tsc --noEmit && npm run test && (cd src-tauri && cargo test)
npx eslint $(git diff --name-only master...HEAD -- '*.ts' '*.tsx')
```

Expected: all clean. Do not proceed until this passes.

- [ ] **Step 2: Confirm the UI is absent on macOS**

```bash
npm run tauri:dev
```

Settings → 一般 must **not** show the menu-entry section, and no prompt appears. This proves the `not_appimage` branch works rather than erroring. Stop the dev server afterwards.

- [ ] **Step 3: Ask before tagging**

A Linux AppImage needs a release. **Do not push a tag without explicit user confirmation.** Ask and wait.

- [ ] **Step 4: Verify on Linux**

On the Ubuntu machine, with the new AppImage:

1. The prompt appears on launch (and **not** at the same time as an update prompt).
2. Pressing 建立 makes the dock icon change to AITerm's, and AITerm appears in the application menu.
3. `cat ~/.local/share/applications/aiterm.desktop` shows `StartupWMClass=app`, a quoted `Exec=` pointing at the AppImage, and `Icon=aiterm` — with `Name`, `Comment` and `Categories` carried over from the bundle.
4. Move the AppImage to another directory, launch it from there, then check the file again: `Exec=` now points at the new location and the menu entry still works.
5. Settings → 一般 → 移除選單項目 removes both the entry and `~/.local/share/icons/hicolor/*/apps/aiterm.png`.
6. Pressing 不用了 means the prompt does not return on the next launch, while Settings still offers to create it.
7. **Removing from Settings must also stop the prompt returning** — the flag means "don't ask again", and removal is an explicit no.
8. **Coexistence with the `.deb`.** The verification machine has both installed. Both menu entries carry `StartupWMClass=app`, so GNOME sees two candidates for the same window class. Check for a duplicated dock icon, and check which binary "New Window" launches when the `.deb` copy is running. This is an inherent limit of WM_CLASS matching rather than something to fix in code, but it must be observed once rather than assumed benign.
9. `ls $APPDIR/usr/share/icons/hicolor` inside the running AppImage — the `.desktop` path was verified against a real build, the icon path never was. If that tree is absent, the entry installs with `Icon=aiterm` pointing at nothing and the dock keeps its gear.

- [ ] **Step 5: Record the result**

Append a 驗證結果 section to `docs/superpowers/specs/2026-07-29-appimage-desktop-integration-design.md` covering which of the six checks passed, on what distro, and anything unexpected. State explicitly whatever was not verified.

```bash
git add -f docs/superpowers/specs/2026-07-29-appimage-desktop-integration-design.md
git commit -m "docs: record AppImage desktop integration verification results"
```

---

## Notes for the implementer

- **Green tests are not evidence.** M1 in Task 1 is the one that matters: renaming `StartupWMClass` yields a menu entry that looks perfect while the dock keeps its generic icon. This repo has already shipped one CI change that was a silent no-op because the chosen verification could not detect it.
- **Do not "fix" the window icon.** `_NET_WM_ICON` is already correct and GNOME ignores it for unmatched windows; that was verified with `xprop` before this plan was written.
- **Do not run bare `npm run lint`** as a gate — ~181 pre-existing problems.
- **`docs/` is gitignored** (`.gitignore:47`) but specs and plans are tracked. Use `git add -f`.
- **Never push a tag without asking.**
