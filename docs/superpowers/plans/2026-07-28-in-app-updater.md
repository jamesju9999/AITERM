# In-App Updater Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users update AITerm from inside the app — one button downloads the signed new version, verifies it, replaces the install in place, and (on explicit user confirmation) restarts.

**Architecture:** Tauri's official `tauri-plugin-updater` handles download/verify/replace; `tauri-plugin-process` handles relaunch. A single `useUpdater` hook owns all update state and replaces the two duplicated GitHub-tags checks that currently live in `App.tsx` and `AboutPage.tsx`; it is shared through `UpdaterContext` (mirroring the existing `LocaleContext`). A Rust command `updater_supported` gates the one-click path off for `.deb` installs, which `latest.json` cannot serve. On the release side, a new `finalize` job composes `latest.json` after all six build jobs finish, then publishes the draft release.

**Tech Stack:** Rust (Tauri 2, `tauri-plugin-updater` 2, `tauri-plugin-process` 2), React 19 + TypeScript, Vitest + React Testing Library, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-07-28-in-app-updater-design.md`

**Prerequisites already done (2026-07-28):** Signing key pair generated at `~/.tauri/aiterm_updater.key{,.pub}` (minisign key ID `B5F4C8732C15A4A`); GitHub Secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` configured.

---

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/src/commands/updater.rs` (new) | Pure `supported_for()` predicate + `updater_supported` command |
| `src-tauri/src/commands/mod.rs` | Register the new module |
| `src-tauri/src/lib.rs` | Register both plugins + the new command |
| `src-tauri/Cargo.toml` | Add `tauri-plugin-updater`, `tauri-plugin-process` |
| `src-tauri/tauri.conf.json` | `plugins.updater` (pubkey + endpoint), `bundle.createUpdaterArtifacts` |
| `src-tauri/capabilities/default.json` | `updater:default`, `process:allow-restart` |
| `src/hooks/useUpdater.ts` (new) | All update state; the single source of truth |
| `src/hooks/useUpdater.test.ts` (new) | Hook unit tests |
| `src/contexts/UpdaterContext.tsx` (new) | Share one hook instance app-wide |
| `src/components/UpdateModal.tsx` (new) | Presentational `UpdateModalView` + context-reading `UpdateModal` |
| `src/components/UpdateModal.test.tsx` (new) | View render tests |
| `src/components/UpdateModal.css` (new) | Modal styling |
| `src/main.tsx` | Mount `UpdaterProvider` |
| `src/App.tsx` | Drop duplicated tags check; consume context; render modal |
| `src/components/Settings/AboutPage.tsx` | Drop duplicated tags check; consume context; add update button |
| `src/lib/i18n.ts` | New strings (zh-TW + en) |
| `.github/workflows/release.yml` | Draft releases, signing env, `finalize` job |

---

## Task 1: `updater_supported` Rust command

`latest.json`'s `linux-x86_64` entry points at an **AppImage**. A `.deb` install that calls `check()` would be told "update available" and handed an AppImage URL. So the one-click path must be gated *before* the button is offered.

The check is "did Tauri's AppImage launcher export `APPIMAGE`?". Testing that by mutating process env is racy under `cargo test`'s parallel threads, so the logic lives in a pure function and the command supplies the real env.

**Files:**
- Create: `src-tauri/src/commands/updater.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/commands/updater.rs` containing *only* the test module and a stub, so the test compiles but fails:

```rust
/// Decides whether the in-app updater can service this install.
///
/// AppImage is the only self-updatable Linux bundle we ship — `.deb` installs
/// are served by `latest.json`'s AppImage URL, which would be the wrong artifact.
/// Tauri's AppImage launcher exports `APPIMAGE`, so its presence identifies the bundle.
///
/// Kept env-free so it is testable without mutating process state, which races
/// under cargo's parallel test threads.
fn supported_for(is_linux: bool, appimage_env: Option<&str>) -> bool {
    let _ = (is_linux, appimage_env);
    false
}

#[cfg(test)]
mod tests {
    use super::supported_for;

    #[test]
    fn non_linux_is_always_supported() {
        assert!(supported_for(false, None));
        assert!(supported_for(false, Some("/tmp/AITerm.AppImage")));
    }

    #[test]
    fn linux_appimage_is_supported() {
        assert!(supported_for(true, Some("/tmp/AITerm.AppImage")));
    }

    #[test]
    fn linux_without_appimage_env_is_not_supported() {
        assert!(!supported_for(true, None));
    }
}
```

Register the module — add this line to `src-tauri/src/commands/mod.rs`, keeping the list alphabetical (it goes after `pub mod shell;`):

```rust
pub mod updater;
```

- [ ] **Step 2: Run the tests, confirm two fail**

```bash
cd src-tauri && cargo test commands::updater
```

Expected: `non_linux_is_always_supported` and `linux_appimage_is_supported` FAIL (`assertion failed`); `linux_without_appimage_env_is_not_supported` passes by accident.

- [ ] **Step 3: Implement**

Replace the stub body and add the command. Full file contents:

```rust
/// Decides whether the in-app updater can service this install.
///
/// AppImage is the only self-updatable Linux bundle we ship — `.deb` installs
/// are served by `latest.json`'s AppImage URL, which would be the wrong artifact.
/// Tauri's AppImage launcher exports `APPIMAGE`, so its presence identifies the bundle.
///
/// Kept env-free so it is testable without mutating process state, which races
/// under cargo's parallel test threads.
fn supported_for(is_linux: bool, appimage_env: Option<&str>) -> bool {
    !is_linux || appimage_env.is_some()
}

#[tauri::command]
pub fn updater_supported() -> bool {
    let appimage = std::env::var("APPIMAGE").ok();
    supported_for(cfg!(target_os = "linux"), appimage.as_deref())
}

#[cfg(test)]
mod tests {
    use super::supported_for;

    #[test]
    fn non_linux_is_always_supported() {
        assert!(supported_for(false, None));
        assert!(supported_for(false, Some("/tmp/AITerm.AppImage")));
    }

    #[test]
    fn linux_appimage_is_supported() {
        assert!(supported_for(true, Some("/tmp/AITerm.AppImage")));
    }

    #[test]
    fn linux_without_appimage_env_is_not_supported() {
        assert!(!supported_for(true, None));
    }
}
```

- [ ] **Step 4: Run the tests, confirm all pass**

```bash
cd src-tauri && cargo test commands::updater
```

Expected: `test result: ok. 3 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/updater.rs src-tauri/src/commands/mod.rs
git commit -m "feat(updater): add updater_supported command to gate .deb installs"
```

---

## Task 2: Install plugins, configure updater, register command

`tauri-plugin-updater` reads `plugins.updater.pubkey` at build time — the config and the plugin registration must land together or the app will not boot.

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src-tauri/src/lib.rs:60-78` (use block), `src-tauri/src/lib.rs:212-214` (builder), `src-tauri/src/lib.rs:238+` (handler)
- Modify: `package.json`

- [ ] **Step 1: Add the Rust dependencies**

In `src-tauri/Cargo.toml`, directly after the `tauri-plugin-dialog = "2"` line (line 26):

```toml
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

- [ ] **Step 2: Add the JS dependencies**

```bash
npm install @tauri-apps/plugin-updater@^2.10.1 @tauri-apps/plugin-process@^2.3.1
```

- [ ] **Step 3: Configure the updater**

In `src-tauri/tauri.conf.json`, add `"createUpdaterArtifacts": true` as the first key inside `"bundle"`, and add a top-level `"plugins"` object as a sibling of `"app"` and `"bundle"`. The result:

```json
{
  "$schema": "../node_modules/@tauri-apps/cli/config.schema.json",
  "productName": "AITerm",
  "version": "1.1.0",
  "identifier": "com.aiterm.app",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build"
  },
  "app": {
    "windows": [
      {
        "title": "AITerm",
        "width": 800,
        "height": 600,
        "resizable": true,
        "fullscreen": false,
        "decorations": false
      }
    ],
    "security": {
      "csp": null
    }
  },
  "plugins": {
    "updater": {
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI1RjRDODczMkMxNUE0QQpSV1JLV3NFeWgweGZDMktVNHhiOUpxUnE5WXdRaHRqV3JnRnFMOFg0V1ByMWRVQ2Yxc1JVcGR6SQo=",
      "endpoints": [
        "https://github.com/jamesju9999/AITERM/releases/latest/download/latest.json"
      ]
    }
  },
  "bundle": {
    "createUpdaterArtifacts": true,
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "externalBin": [
      "binaries/db2-sidecar"
    ]
  }
}
```

- [ ] **Step 4: Grant the permissions**

In `src-tauri/capabilities/default.json`, add two entries to the end of the `"permissions"` array (after `"dialog:allow-save"`):

```json
    "dialog:allow-save",
    "updater:default",
    "process:allow-restart"
  ]
```

- [ ] **Step 5: Register the plugins and the command**

In `src-tauri/src/lib.rs`, add to the `use` block — insert directly after the `shell::open_url,` line (line 71):

```rust
    shell::open_url,
    updater::updater_supported,
```

Register both plugins in the builder — replace lines 212-213:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
```

Add the command to `generate_handler!` — insert directly after the `open_url,` line (line 312):

```rust
            open_url,
            updater_supported,
```

- [ ] **Step 6: Verify it builds**

```bash
cd src-tauri && cargo build 2>&1 | tail -5
```

Expected: `Finished ...` with no errors. (First build after adding plugins is slow — several minutes.)

```bash
npx tsc --noEmit
```

Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json \
        src-tauri/capabilities/default.json src-tauri/src/lib.rs package.json package-lock.json
git commit -m "feat(updater): wire tauri-plugin-updater and plugin-process"
```

---

## Task 3: `useUpdater` hook

The single source of truth for update state. Replaces the duplicated tags checks in `App.tsx:34-52` and `AboutPage.tsx:20-39`.

Two behaviours worth calling out because they are easy to get wrong:

- **`dismissed` is separate from `state`.** Closing the modal must not clear the TabBar's red dot — the update is still available, the user just doesn't want the dialog right now.
- **The mount check is silent.** A failed background check falls back to `idle`, matching the current best-effort behaviour. A *manual* check surfaces the error.

**Files:**
- Create: `src/hooks/useUpdater.ts`
- Create: `src/hooks/useUpdater.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `src/hooks/useUpdater.test.ts`:

```ts
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const checkMock = vi.fn();
const relaunchMock = vi.fn();
const invokeMock = vi.fn();

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => checkMock(...args),
}));
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: (...args: unknown[]) => relaunchMock(...args),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useUpdater } from "./useUpdater";

/** Minimal stand-in for the plugin's Update object. */
function fakeUpdate(overrides: Record<string, unknown> = {}) {
  return {
    version: "1.2.0",
    body: "Bug fixes",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

beforeEach(() => {
  checkMock.mockReset();
  relaunchMock.mockReset();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(true); // updater_supported
});

describe("useUpdater", () => {
  it("reports 'none' when the endpoint has no newer version", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("none"));
    expect(result.current.hasUpdate).toBe(false);
  });

  it("reports 'available' with version and notes on a supported install", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("available"));
    expect(result.current.state).toEqual({
      status: "available",
      version: "1.2.0",
      notes: "Bug fixes",
    });
    expect(result.current.hasUpdate).toBe(true);
  });

  it("reports 'unsupported' when updater_supported is false (.deb install)", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    invokeMock.mockResolvedValue(false);
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("unsupported"));
    expect(result.current.state).toEqual({ status: "unsupported", version: "1.2.0" });
    // Still counts as an update so the TabBar dot and the manual link show up.
    expect(result.current.hasUpdate).toBe(true);
  });

  it("falls back to idle when the automatic mount check fails", async () => {
    checkMock.mockRejectedValue(new Error("offline"));
    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.state.status).toBe("idle"));
  });

  it("surfaces the error when a manual check fails", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("none"));

    checkMock.mockRejectedValue(new Error("offline"));
    await act(async () => { await result.current.check(); });

    expect(result.current.state.status).toBe("error");
  });

  it("tracks download progress and ends in 'ready'", async () => {
    const update = fakeUpdate({
      downloadAndInstall: vi.fn(async (onEvent: (e: unknown) => void) => {
        onEvent({ event: "Started", data: { contentLength: 1000 } });
        onEvent({ event: "Progress", data: { chunkLength: 400 } });
        onEvent({ event: "Progress", data: { chunkLength: 600 } });
        onEvent({ event: "Finished" });
      }),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });

    expect(result.current.state).toEqual({ status: "ready", version: "1.2.0" });
  });

  it("treats a missing contentLength as an unknown total", async () => {
    let seenTotal: number | null | undefined;
    const update = fakeUpdate({
      downloadAndInstall: vi.fn(async (onEvent: (e: unknown) => void) => {
        onEvent({ event: "Started", data: {} });
        onEvent({ event: "Progress", data: { chunkLength: 10 } });
      }),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => {
      const p = result.current.install();
      if (result.current.state.status === "downloading") seenTotal = result.current.state.total;
      await p;
    });

    expect(seenTotal ?? null).toBeNull();
  });

  it("surfaces install failures as an error", async () => {
    const update = fakeUpdate({
      downloadAndInstall: vi.fn().mockRejectedValue(new Error("signature mismatch")),
    });
    checkMock.mockResolvedValue(update);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    await act(async () => { await result.current.install(); });

    expect(result.current.state.status).toBe("error");
    if (result.current.state.status === "error") {
      expect(result.current.state.message).toContain("signature mismatch");
    }
  });

  it("dismiss() hides the modal but keeps hasUpdate true", async () => {
    checkMock.mockResolvedValue(fakeUpdate());
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("available"));

    act(() => { result.current.dismiss(); });

    expect(result.current.dismissed).toBe(true);
    expect(result.current.hasUpdate).toBe(true);
  });

  it("relaunch() delegates to the process plugin", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.state.status).toBe("none"));

    await act(async () => { await result.current.relaunch(); });

    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
npx vitest run src/hooks/useUpdater.test.ts
```

Expected: FAIL — `Failed to resolve import "./useUpdater"`.

- [ ] **Step 3: Implement the hook**

Create `src/hooks/useUpdater.ts`:

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch as processRelaunch } from "@tauri-apps/plugin-process";

export type UpdaterState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "none" }
  | { status: "available"; version: string; notes: string }
  | { status: "downloading"; version: string; downloaded: number; total: number | null }
  | { status: "ready"; version: string }
  | { status: "unsupported"; version: string }
  | { status: "error"; message: string };

export interface UpdaterApi {
  state: UpdaterState;
  /** True while an update exists, regardless of whether the modal was dismissed. */
  hasUpdate: boolean;
  dismissed: boolean;
  check: () => Promise<void>;
  install: () => Promise<void>;
  relaunch: () => Promise<void>;
  dismiss: () => void;
}

/** The plugin's Update object, narrowed to what we use. */
interface PendingUpdate {
  version: string;
  body?: string;
  downloadAndInstall: (onEvent: (event: DownloadEvent) => void) => Promise<void>;
}

type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export function useUpdater(): UpdaterApi {
  const [state, setState] = useState<UpdaterState>({ status: "idle" });
  const [dismissed, setDismissed] = useState(false);
  const pendingRef = useRef<PendingUpdate | null>(null);

  // Tauri's async plugin calls can resolve after unmount; guard setState the
  // same way useAiChat does to avoid the React warning.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const set = useCallback((next: UpdaterState) => {
    if (mountedRef.current) setState(next);
  }, []);

  const runCheck = useCallback(async (silent: boolean) => {
    set({ status: "checking" });
    try {
      const update = (await check()) as PendingUpdate | null;
      if (!update) {
        set({ status: "none" });
        return;
      }
      pendingRef.current = update;
      setDismissed(false);
      // Gate before offering the one-click path — see commands/updater.rs.
      const supported = await invoke<boolean>("updater_supported");
      if (!supported) {
        set({ status: "unsupported", version: update.version });
        return;
      }
      set({ status: "available", version: update.version, notes: update.body ?? "" });
    } catch (e) {
      // The mount check is best-effort: an offline user should not see an error.
      set(silent ? { status: "idle" } : { status: "error", message: String(e) });
    }
  }, [set]);

  const check_ = useCallback(() => runCheck(false), [runCheck]);

  const install = useCallback(async () => {
    const update = pendingRef.current;
    if (!update) return;

    let total: number | null = null;
    let downloaded = 0;
    set({ status: "downloading", version: update.version, downloaded: 0, total: null });

    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            set({ status: "downloading", version: update.version, downloaded: 0, total });
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            set({ status: "downloading", version: update.version, downloaded, total });
            break;
          case "Finished":
            set({ status: "ready", version: update.version });
            break;
        }
      });
      set({ status: "ready", version: update.version });
    } catch (e) {
      set({ status: "error", message: String(e) });
    }
  }, [set]);

  const relaunch = useCallback(async () => {
    await processRelaunch();
  }, []);

  const dismiss = useCallback(() => setDismissed(true), []);

  // Auto-check once on mount. The ref keeps StrictMode's double-invoke in dev
  // from firing two network requests.
  const autoCheckedRef = useRef(false);
  useEffect(() => {
    if (autoCheckedRef.current) return;
    autoCheckedRef.current = true;
    void runCheck(true);
  }, [runCheck]);

  const hasUpdate =
    state.status === "available" ||
    state.status === "downloading" ||
    state.status === "ready" ||
    state.status === "unsupported";

  return { state, hasUpdate, dismissed, check: check_, install, relaunch, dismiss };
}
```

- [ ] **Step 4: Run the tests, confirm they pass**

```bash
npx vitest run src/hooks/useUpdater.test.ts
```

Expected: `Tests  10 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useUpdater.ts src/hooks/useUpdater.test.ts
git commit -m "feat(updater): add useUpdater hook as single source of update state"
```

---

## Task 4: `UpdaterContext`

`AboutPage` sits several levels below `App.tsx` inside the `SettingsView` route, so prop drilling is not viable. This mirrors `src/contexts/LocaleContext.tsx`, including its forgiving no-provider fallback — that fallback is what keeps existing component tests from needing a new wrapper.

**Files:**
- Create: `src/contexts/UpdaterContext.tsx`
- Modify: `src/main.tsx`

- [ ] **Step 1: Create the context**

Create `src/contexts/UpdaterContext.tsx`:

```tsx
import { createContext, useContext, type ReactNode } from "react";
import { useUpdater, type UpdaterApi } from "../hooks/useUpdater";

const UpdaterContext = createContext<UpdaterApi | null>(null);

export function UpdaterProvider({ children }: { children: ReactNode }) {
  const updater = useUpdater();
  return <UpdaterContext.Provider value={updater}>{children}</UpdaterContext.Provider>;
}

/**
 * Returns an inert updater when no provider is mounted, matching useLocale's
 * fallback so components stay renderable in isolation (tests, storybook-style use).
 */
export function useUpdaterContext(): UpdaterApi {
  const ctx = useContext(UpdaterContext);
  if (!ctx) {
    return {
      state: { status: "idle" },
      hasUpdate: false,
      dismissed: false,
      check: async () => {},
      install: async () => {},
      relaunch: async () => {},
      dismiss: () => {},
    };
  }
  return ctx;
}
```

- [ ] **Step 2: Mount the provider**

Replace the render call in `src/main.tsx`:

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LocaleProvider } from "./contexts/LocaleContext";
import { UpdaterProvider } from "./contexts/UpdaterContext";
import { getActiveTheme, applyTheme } from "./lib/themes";

// Apply saved theme before first render to avoid flash
applyTheme(getActiveTheme());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <LocaleProvider>
      <UpdaterProvider>
        <App />
      </UpdaterProvider>
    </LocaleProvider>
  </React.StrictMode>,
);
```

- [ ] **Step 3: Verify types**

```bash
npx tsc --noEmit
```

Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add src/contexts/UpdaterContext.tsx src/main.tsx
git commit -m "feat(updater): share updater state through UpdaterContext"
```

---

## Task 5: i18n strings

`Translations` is derived from the `zhTW` object, so both locales must gain the same keys or `tsc` fails. That type check is the test here.

`about_update_error` ("檢查失敗，請稍後再試") is superseded by `update_failed`: once the hook exists, a single `error` state covers both a failed check and a failed download, and "檢查失敗" is wrong for the latter. It is **deleted in Task 8**, alongside its only consumer (`AboutPage.tsx:70`) — deleting it here would leave a commit that does not compile.

**Files:**
- Modify: `src/lib/i18n.ts:355` (zh-TW), `src/lib/i18n.ts:1351` (en)

- [ ] **Step 1: Add the zh-TW strings**

In `src/lib/i18n.ts`, insert directly after the `about_update_error: "檢查失敗，請稍後再試",` line (line 355):

```ts
    about_update_error: "檢查失敗，請稍後再試",
    update_modal_title: "有新版本可用",
    update_now: "立即更新",
    update_later: "稍後",
    update_downloading: "下載中…",
    update_ready: "更新已下載完成",
    update_restart_now: "重新啟動以完成更新",
    update_restart_warning: "重新啟動將結束所有終端機分頁與執行中的指令。",
    update_failed: "更新失敗",
    update_manual_hint: "此安裝方式不支援自動更新，請至 GitHub 下載新版。",
```

- [ ] **Step 2: Add the matching en strings**

Insert directly after the `about_update_error: "Check failed, please try again",` line (line 1351):

```ts
    about_update_error: "Check failed, please try again",
    update_modal_title: "Update available",
    update_now: "Update now",
    update_later: "Later",
    update_downloading: "Downloading…",
    update_ready: "Update downloaded",
    update_restart_now: "Restart to finish updating",
    update_restart_warning: "Restarting will end all terminal tabs and running commands.",
    update_failed: "Update failed",
    update_manual_hint: "This install type does not support auto-update. Please download the new version from GitHub.",
```

- [ ] **Step 3: Verify both locales stayed in sync**

```bash
npx tsc --noEmit
```

Expected: no output. (A key added to one locale but not the other surfaces here as a `Translations` mismatch.)

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(updater): add update modal strings for zh-TW and en"
```

---

## Task 6: `UpdateModal`

Split into a presentational `UpdateModalView` (all props, no context) and a thin `UpdateModal` that reads context. The view is what the tests drive — no provider wrapper needed, and every state is reachable directly.

The app runs with `decorations: false`, so this is an in-app React overlay, not a native dialog.

**Files:**
- Create: `src/components/UpdateModal.tsx`
- Create: `src/components/UpdateModal.test.tsx`
- Create: `src/components/UpdateModal.css`

- [ ] **Step 1: Write the failing tests**

Create `src/components/UpdateModal.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { UpdateModalView } from "./UpdateModal";
import type { UpdaterState } from "../hooks/useUpdater";

function renderView(state: UpdaterState, overrides: Record<string, unknown> = {}) {
  const props = {
    state,
    dismissed: false,
    onInstall: vi.fn(),
    onDismiss: vi.fn(),
    onRelaunch: vi.fn(),
    onOpenReleases: vi.fn(),
    ...overrides,
  };
  render(<UpdateModalView {...props} />);
  return props;
}

describe("UpdateModalView", () => {
  it("renders nothing while idle", () => {
    const { container } = render(
      <UpdateModalView
        state={{ status: "idle" }}
        dismissed={false}
        onInstall={vi.fn()}
        onDismiss={vi.fn()}
        onRelaunch={vi.fn()}
        onOpenReleases={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing once dismissed", () => {
    const { container } = render(
      <UpdateModalView
        state={{ status: "available", version: "1.2.0", notes: "" }}
        dismissed
        onInstall={vi.fn()}
        onDismiss={vi.fn()}
        onRelaunch={vi.fn()}
        onOpenReleases={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the version, notes and an update button when available", async () => {
    const props = renderView({ status: "available", version: "1.2.0", notes: "Bug fixes" });

    expect(screen.getByText(/1\.2\.0/)).toBeTruthy();
    expect(screen.getByText("Bug fixes")).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "立即更新" }));
    expect(props.onInstall).toHaveBeenCalledTimes(1);
  });

  it("shows a percentage when the total size is known", () => {
    renderView({ status: "downloading", version: "1.2.0", downloaded: 500, total: 1000 });
    // Regex, not an exact string: the label and the percentage are separate text
    // nodes inside one <p>, so its textContent is "下載中… 50%".
    expect(screen.getByText(/50%/)).toBeTruthy();
  });

  it("omits the percentage when the total size is unknown", () => {
    renderView({ status: "downloading", version: "1.2.0", downloaded: 500, total: null });
    expect(screen.queryByText(/%/)).toBeNull();
  });

  it("warns about losing terminal sessions before restarting", async () => {
    const props = renderView({ status: "ready", version: "1.2.0" });

    expect(screen.getByText(/重新啟動將結束所有終端機分頁與執行中的指令。/)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "重新啟動以完成更新" }));
    expect(props.onRelaunch).toHaveBeenCalledTimes(1);
  });

  it("offers a manual download link on unsupported installs", async () => {
    const props = renderView({ status: "unsupported", version: "1.2.0" });

    expect(screen.getByText(/此安裝方式不支援自動更新/)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "點此前往下載" }));
    expect(props.onOpenReleases).toHaveBeenCalledTimes(1);
    expect(props.onInstall).not.toHaveBeenCalled();
  });

  it("shows the failure message on error", () => {
    renderView({ status: "error", message: "signature mismatch" });
    expect(screen.getByText(/signature mismatch/)).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
npx vitest run src/components/UpdateModal.test.tsx
```

Expected: FAIL — `Failed to resolve import "./UpdateModal"`.

- [ ] **Step 3: Write the styles**

Create `src/components/UpdateModal.css`:

```css
.update-modal-backdrop {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 4000;
  max-width: 380px;
}

.update-modal {
  background: #1b1b1b;
  border: 1px solid #333;
  border-radius: 10px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.55);
  padding: 16px 18px;
  color: #e6e6e6;
  font-family: "Consolas", "Cascadia Code", "Menlo", monospace;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.update-modal-title {
  font-size: 15px;
  font-weight: 700;
  margin: 0;
}

.update-modal-version {
  font-size: 13px;
  color: #7aadff;
  margin: 0;
}

.update-modal-notes {
  font-size: 12px;
  color: #aaa;
  margin: 0;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
}

.update-modal-warning {
  font-size: 12px;
  color: #d9a441;
  margin: 0;
}

.update-modal-error {
  font-size: 12px;
  color: #e06c75;
  margin: 0;
  word-break: break-word;
}

.update-modal-progress {
  height: 6px;
  border-radius: 3px;
  background: #2c2c2c;
  overflow: hidden;
}

.update-modal-progress-bar {
  height: 100%;
  background: #7aadff;
  transition: width 120ms linear;
}

.update-modal-progress-bar--indeterminate {
  width: 40%;
  animation: update-modal-slide 1.1s ease-in-out infinite;
}

@keyframes update-modal-slide {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(250%); }
}

.update-modal-actions {
  display: flex;
  gap: 10px;
  justify-content: flex-end;
}
```

- [ ] **Step 4: Implement the component**

Create `src/components/UpdateModal.tsx`:

```tsx
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

  const percent =
    state.status === "downloading" && state.total
      ? Math.min(100, Math.round((state.downloaded / state.total) * 100))
      : null;

  return (
    <div className="update-modal-backdrop">
      <div className="update-modal" role="dialog" aria-label={t.update_modal_title}>
        <p className="update-modal-title">
          {state.status === "error" ? t.update_failed : t.update_modal_title}
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
```

- [ ] **Step 5: Run the tests, confirm they pass**

```bash
npx vitest run src/components/UpdateModal.test.tsx
```

Expected: `Tests  8 passed`.

- [ ] **Step 6: Commit**

```bash
git add src/components/UpdateModal.tsx src/components/UpdateModal.test.tsx src/components/UpdateModal.css
git commit -m "feat(updater): add update modal with progress and restart gate"
```

---

## Task 7: Wire `App.tsx`

Delete the first of the two duplicated tags checks and drive the TabBar dot from the shared hook.

**Files:**
- Modify: `src/App.tsx:1-15` (imports/consts), `src/App.tsx:33-52` (remove), `src/App.tsx:55-68` (deps), `src/App.tsx:88` (prop)

- [ ] **Step 1: Replace the header, remove the tags check**

Replace lines 1-15 of `src/App.tsx`:

```tsx
import { useEffect, useState } from "react";
import { MemoryRouter, Routes, Route, useNavigate, useLocation } from "react-router-dom";
import { TerminalApp } from "./components/TerminalApp";
import { SettingsView } from "./components/Settings/SettingsView";
import { OnboardingWizard } from "./components/Onboarding/OnboardingWizard";
import { UpdateModal } from "./components/UpdateModal";
import { useUpdaterContext } from "./contexts/UpdaterContext";
import { isOnboardingDone } from "./ipc/config";
import "./App.css";
```

Note `getVersion` and the `UpdateInfo` interface and `TAGS_API` all go away — `AboutPage` keeps its own `getVersion` call for display.

- [ ] **Step 2: Swap local state for the shared hook**

Replace the `const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);` line with:

```tsx
  const { hasUpdate } = useUpdaterContext();
```

Then delete the entire `// Auto-check for updates once on mount` effect (originally lines 33-52) — the hook does this now.

- [ ] **Step 3: Update the keyboard handler**

Replace the Ctrl+, effect body and deps:

```tsx
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
    // hasUpdate must stay in deps — the handler is registered once via
    // addEventListener, and without it the closure would keep seeing the
    // initial false value even after the async version-check resolves.
  }, [navigate, hasUpdate]);
```

- [ ] **Step 4: Pass the prop and mount the modal**

Replace `<TerminalApp hasUpdate={updateInfo?.hasUpdate ?? false} />` with:

```tsx
        <TerminalApp hasUpdate={hasUpdate} />
```

Then add the modal as the last child of the outer wrapper `<div>` in the returned JSX, immediately before its closing `</div>`:

```tsx
      <UpdateModal />
    </div>
```

- [ ] **Step 5: Verify**

```bash
npx tsc --noEmit && npm run test
npx eslint src/App.tsx
```

Expected: no type errors, all tests pass, and `eslint` on the changed file exits 0.

**Do not run bare `npm run lint` as a pass/fail gate** — this repo has ~181 pre-existing lint problems (162 errors), so it never exits clean. Lint the files you changed instead. In particular `src/components/TabBar/index.test.tsx` must still pass — it drives `hasUpdate` as a prop and is unaffected.

- [ ] **Step 6: Commit**

```bash
git add src/App.tsx
git commit -m "refactor(updater): drive App update state from useUpdater, mount modal"
```

---

## Task 8: Wire `AboutPage.tsx`

Delete the second duplicated tags check. The page keeps `getVersion()` for showing the current version, but all update state now comes from the shared hook — so the modal and this page can never disagree.

This task also retires `about_update_error`: the hook's single `error` state covers both a failed check and a failed download, and "檢查失敗，請稍後再試" is wrong for the latter. `update_failed` replaces it, and this is the commit where the old key becomes unused.

**Files:**
- Modify: `src/components/Settings/AboutPage.tsx` (full rewrite)
- Modify: `src/lib/i18n.ts` (delete `about_update_error` from both locales)

- [ ] **Step 1: Rewrite the component**

Replace the entire contents of `src/components/Settings/AboutPage.tsx`:

```tsx
import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useLocale } from "../../contexts/LocaleContext";
import { useUpdaterContext } from "../../contexts/UpdaterContext";
import { openUrl } from "../../ipc/shell";
import iconUrl from "../../../src-tauri/icons/128x128.png";
import "./AboutPage.css";

const GITHUB_URL = "https://github.com/jamesju9999/AITERM";
const RELEASES_URL = "https://github.com/jamesju9999/AITERM/releases/latest";

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
    openUrl(GITHUB_URL).catch(console.error);
  };

  const percent =
    state.status === "downloading" && state.total
      ? Math.min(100, Math.round((state.downloaded / state.total) * 100))
      : null;

  const statusText = () => {
    switch (state.status) {
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
              onClick={() => openUrl(RELEASES_URL).catch(console.error)}
            >
              {t.about_update_link}
            </button>
          </span>
        );
      case "error":
        return <span>{t.update_failed}</span>;
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
          disabled={state.status === "checking" || state.status === "downloading"}
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

- [ ] **Step 2: Delete the orphaned string**

Remove the `about_update_error` line from **both** locale objects in `src/lib/i18n.ts` — the zh-TW one (`about_update_error: "檢查失敗，請稍後再試",`) and the en one (`about_update_error: "Check failed, please try again",`). Nothing references it after Step 1.

Confirm it is gone:

```bash
grep -rn "about_update_error" src/ || echo "no references remain"
```

Expected: `no references remain`.

- [ ] **Step 3: Verify**

```bash
npx tsc --noEmit && npm run test
npx eslint src/components/Settings/AboutPage.tsx src/lib/i18n.ts
```

Expected: no type errors, all tests pass, `eslint` on the changed files exits 0. (Bare `npm run lint` never exits clean here — ~181 pre-existing problems.)

- [ ] **Step 4: Manually confirm the app still boots**

```bash
npm run tauri:dev
```

Expected: window opens; Ctrl+, reaches Settings → About; the version shows; pressing "檢查更新" moves through `checking` and lands on a real status. In dev the updater endpoint has no matching release yet, so `up to date` or an error are both acceptable — what matters is that nothing crashes and the app does not hang.

Stop the dev server before continuing.

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/AboutPage.tsx src/lib/i18n.ts
git commit -m "refactor(updater): drive AboutPage from shared updater state"
```

---

## Task 9: Sign and draft the release builds

All six `tauri-action` steps need the signing key, and all six must produce a *draft* release so `latest.json` can land before anything is public.

**Files:**
- Modify: `.github/workflows/release.yml` (six `tauri-action` steps)

- [ ] **Step 1: Add signing env and draft flag to every build step**

There are six steps that use `tauri-apps/tauri-action@v0`:
`Build (macOS arm64)`, `Build (Windows)`, `Build (Linux AppImage — Ubuntu 22.04)`, `Build (Linux x64 .deb — Ubuntu 24.04)`, `Build (Linux ARM64 AppImage — Ubuntu 22.04)`, `Build (Linux ARM64 .deb — Ubuntu 24.04)`.

For **each** of them, extend the `env:` block with the two signing secrets. For example, the macOS step's `env:` becomes:

```yaml
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          APPLE_SIGNING_IDENTITY: "-"
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
```

and every other step's becomes:

```yaml
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
```

The `.deb` steps need the secrets too, and **not** for the reason you might assume. `tauri-cli`'s `sign_updaters` treats `Deb` as an updater-enabled package type alongside `Updater`, `Nsis`, `WindowsMsi`, `AppImage` and `Rpm`. So a `.deb` build with `createUpdaterArtifacts` enabled but no key available fails outright with *"A public key has been found, but no private key."* — and when the key **is** present it emits an extra `AITerm_<version>_amd64.deb.sig`.

That stray `.deb.sig` is harmless: Task 10's `platform_for()` returns `None` for it and logs `skipping unrecognised artifact`, so it never reaches `latest.json`. The only visible effect is two extra `.sig` assets on the release page.

In the same six steps, change:

```yaml
          releaseDraft: false
```

to:

```yaml
          releaseDraft: true
```

- [ ] **Step 2: Verify the workflow parses**

```bash
python3 -c "import yaml,sys; d=yaml.safe_load(open('.github/workflows/release.yml')); print('jobs:', list(d['jobs']))"
```

Expected: `jobs: ['build']`.

Then confirm the edit landed on all six steps:

```bash
grep -c "TAURI_SIGNING_PRIVATE_KEY:" .github/workflows/release.yml
grep -c "releaseDraft: true" .github/workflows/release.yml
grep -c "releaseDraft: false" .github/workflows/release.yml
```

Expected: `6`, `6`, `0`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(updater): sign updater artifacts and build releases as drafts"
```

---

## Task 10: `finalize` job — compose `latest.json` and publish

Six parallel jobs cannot each read-modify-write the same `latest.json` asset; the later writer silently drops the earlier writer's platforms. One serialized writer after the barrier removes the race entirely.

**Files:**
- Modify: `.github/workflows/release.yml` (append a new job)

- [ ] **Step 1: Append the job**

Add this at the end of `.github/workflows/release.yml`, at the same indentation as `build:` (two spaces, inside `jobs:`):

```yaml
  # Six parallel build jobs cannot each write latest.json — the later writer would
  # drop the earlier writer's platform entries. This job is the single writer: it
  # runs only after every build finishes, composes the full manifest, then flips
  # the release out of draft so users never see a half-populated manifest.
  finalize:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - name: Download updater signatures
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
        run: |
          mkdir -p sigs
          gh release download "${{ github.ref_name }}" --pattern '*.sig' --dir sigs
          ls -la sigs

      - name: Compose latest.json
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
          TAG: ${{ github.ref_name }}
          # Already RFC 3339, and present on both push and workflow_dispatch events,
          # so no shell date arithmetic is needed. Tauri only displays pub_date.
          PUB_DATE: ${{ github.event.repository.updated_at }}
        run: |
          python3 - <<'PY'
          import json, os, pathlib, sys

          tag = os.environ["TAG"]
          repo = os.environ["GH_REPO"]
          version = tag[1:] if tag.startswith("v") else tag

          def platform_for(name):
              if name.endswith(".app.tar.gz"):
                  return "darwin-aarch64"
              if name.endswith("-setup.exe"):
                  return "windows-x86_64"
              if name.endswith(".AppImage"):
                  return "linux-aarch64" if ("aarch64" in name or "arm64" in name) else "linux-x86_64"
              return None

          platforms = {}
          for sig_path in sorted(pathlib.Path("sigs").glob("*.sig")):
              artifact = sig_path.name[:-4]  # strip ".sig"
              key = platform_for(artifact)
              if key is None:
                  print(f"skipping unrecognised artifact: {artifact}")
                  continue
              platforms[key] = {
                  "signature": sig_path.read_text().strip(),
                  "url": f"https://github.com/{repo}/releases/download/{tag}/{artifact}",
              }
              print(f"{key} <- {artifact}")

          expected = {"darwin-aarch64", "windows-x86_64", "linux-x86_64", "linux-aarch64"}
          missing = expected - set(platforms)
          if missing:
              sys.exit(f"missing updater artifacts for: {sorted(missing)}")

          manifest = {
              "version": version,
              "notes": f"https://github.com/{repo}/releases/tag/{tag}",
              "pub_date": os.environ["PUB_DATE"],
              "platforms": platforms,
          }
          pathlib.Path("latest.json").write_text(json.dumps(manifest, indent=2) + "\n")
          print(json.dumps(manifest, indent=2))
          PY

      - name: Upload latest.json and publish the release
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          GH_REPO: ${{ github.repository }}
        run: |
          gh release upload "${{ github.ref_name }}" latest.json --clobber
          gh release edit "${{ github.ref_name }}" --draft=false --latest
```

- [ ] **Step 2: Verify the workflow parses and the job graph is right**

```bash
python3 -c "
import yaml
d = yaml.safe_load(open('.github/workflows/release.yml'))
print('jobs:', list(d['jobs']))
print('finalize needs:', d['jobs']['finalize']['needs'])
print('steps:', [s['name'] for s in d['jobs']['finalize']['steps']])
"
```

Expected:
```
jobs: ['build', 'finalize']
finalize needs: build
steps: ['Download updater signatures', 'Compose latest.json', 'Upload latest.json and publish the release']
```

- [ ] **Step 3: Dry-run the manifest composer locally**

Confirm the platform mapping is right before spending a CI run on it:

```bash
mkdir -p /tmp/updater-dry/sigs && cd /tmp/updater-dry
for f in AITerm.app.tar.gz AITerm_1.2.0_x64-setup.exe AITerm_1.2.0_amd64.AppImage AITerm_1.2.0_aarch64.AppImage; do
  echo "fake-signature-for-$f" > "sigs/$f.sig"
done
TAG=v1.2.0 GH_REPO=jamesju9999/AITERM PUB_DATE=2026-07-28T00:00:00Z python3 - <<'PY'
import json, os, pathlib, sys
tag = os.environ["TAG"]; repo = os.environ["GH_REPO"]
version = tag[1:] if tag.startswith("v") else tag
def platform_for(name):
    if name.endswith(".app.tar.gz"): return "darwin-aarch64"
    if name.endswith("-setup.exe"): return "windows-x86_64"
    if name.endswith(".AppImage"):
        return "linux-aarch64" if ("aarch64" in name or "arm64" in name) else "linux-x86_64"
    return None
platforms = {}
for sig_path in sorted(pathlib.Path("sigs").glob("*.sig")):
    artifact = sig_path.name[:-4]
    key = platform_for(artifact)
    if key is None:
        print(f"skipping unrecognised artifact: {artifact}"); continue
    platforms[key] = {"signature": sig_path.read_text().strip(),
                      "url": f"https://github.com/{repo}/releases/download/{tag}/{artifact}"}
expected = {"darwin-aarch64", "windows-x86_64", "linux-x86_64", "linux-aarch64"}
missing = expected - set(platforms)
if missing: sys.exit(f"missing updater artifacts for: {sorted(missing)}")
print(json.dumps({"version": version, "notes": "", "pub_date": os.environ["PUB_DATE"],
                  "platforms": platforms}, indent=2))
PY
```

Expected: a manifest with **exactly four** platform keys — `darwin-aarch64`, `windows-x86_64`, `linux-x86_64`, `linux-aarch64` — and no `missing updater artifacts` error.

Then clean up: `rm -rf /tmp/updater-dry`

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(updater): add finalize job composing latest.json before publish"
```

---

## Task 11: End-to-end verification

Auto-update cannot be verified under `tauri:dev` — a dev build never self-updates. The only real proof is two published releases. Unit tests passing is **not** evidence that this feature works.

**Files:** none (verification only)

- [ ] **Step 1: Full local verification first**

```bash
npx tsc --noEmit && npm run test && (cd src-tauri && cargo test)
npx eslint $(git diff --name-only master...HEAD -- '*.ts' '*.tsx')
```

Expected: no type errors, all frontend tests pass, all Rust tests pass, and `eslint` on every file this branch touched exits 0. (Bare `npm run lint` never exits clean here — ~181 pre-existing problems unrelated to this work.) Do not proceed until this passes.

- [ ] **Step 2: Ask the user before tagging**

Pushing a `vX.Y.Z` tag triggers a full release build. **Do not push a tag without explicit user confirmation.** Ask, and wait for the answer.

- [ ] **Step 3: Publish the first updater-enabled release**

Once confirmed:

```bash
git tag v1.2.0 && git push origin v1.2.0
gh run watch
```

Then verify the release itself:

```bash
gh release view v1.2.0 --json isDraft,assets \
  --jq '{draft: .isDraft, assets: [.assets[].name]}'
```

Expected: `draft: false`, and the asset list contains `latest.json`, four `.sig` files, `AITerm.app.tar.gz`, the NSIS `-setup.exe`, both `.AppImage`s, both `.deb`s, and the `.dmg`.

```bash
curl -sL https://github.com/jamesju9999/AITERM/releases/latest/download/latest.json | python3 -m json.tool
```

Expected: `version` is `1.2.0` and `platforms` has all four keys with non-empty signatures.

- [ ] **Step 4: Install v1.2.0 manually**

Download and install the platform bundle by hand. This step cannot be skipped — existing v1.1.0 installs have no updater plugin, so v1.2.0 is necessarily a manual install.

- [ ] **Step 5: Publish a second release and test the upgrade**

Ask the user again before tagging. Then:

```bash
git tag v1.2.1 && git push origin v1.2.1
gh run watch
```

- [ ] **Step 6: Verify the update in the installed app**

Launch the manually-installed v1.2.0 and confirm, in order:

1. The update modal appears on its own within a few seconds of launch, showing `v1.2.1`.
2. "稍後" dismisses the modal, and the TabBar red dot stays visible.
3. Ctrl+, → About shows `🆕 有新版本 v1.2.1 — 立即更新`.
4. Pressing 立即更新 shows a progress percentage that advances.
5. On completion the restart warning appears: 「重新啟動將結束所有終端機分頁與執行中的指令。」
6. The app does **not** restart on its own — confirm by waiting at least 30 seconds.
7. Pressing 重新啟動以完成更新 relaunches the app, and About now shows `v1.2.1`.
8. On macOS specifically: the updated app opens without needing `xattr -cr`.

- [ ] **Step 7: Verify the `.deb` fallback**

On a Linux machine with the `.deb` installed (not the AppImage), launch the app and confirm the modal shows 「此安裝方式不支援自動更新，請至 GitHub 下載新版。」 with a working download link — and that **no** 立即更新 button is offered.

- [ ] **Step 8: Record the result**

Append the outcome to `docs/superpowers/specs/2026-07-28-in-app-updater-design.md` under a new "驗證結果" section: which platforms were tested, on what OS versions, and anything that behaved unexpectedly.

```bash
git add -f docs/superpowers/specs/2026-07-28-in-app-updater-design.md
git commit -m "docs: record in-app updater end-to-end verification results"
```

---

## Notes for the implementer

- **Green tests are not evidence.** Task 3's original suite passed 10/10 while 5 of 7 mutations survived, because one test observed `result.current` only after the operation it was meant to measure had already finished — making its assertion vacuous. When a test targets an *intermediate* state, drive the operation event-by-event (park the stub on a deferred promise) so React can re-render between steps, and assert the full state object at each step. If you are unsure a test earns its keep, mutate the implementation and confirm the test fails.
- **Do not use bare `npm run lint` as a gate** — this repo carries ~181 pre-existing problems (162 errors) unrelated to this work. Run `npx eslint <changed files>` instead.
- **`docs/` is gitignored** (`.gitignore:47`) but specs and plans are tracked anyway. Use `git add -f` for anything under `docs/`.
- **Never push a tag without asking.** Tags trigger release builds.
- **The private key at `~/.tauri/aiterm_updater.key` is irreplaceable.** If it is lost, every installed AITerm is stranded on the old public key and all users must reinstall by hand. It should be backed up outside this machine and its permissions tightened (`chmod 600`).
