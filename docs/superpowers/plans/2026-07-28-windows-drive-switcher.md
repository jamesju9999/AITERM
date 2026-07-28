# Windows Drive Switcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the Windows file panel a toolbar dropdown that switches drives, so a user sitting in `C:` can reach `D:` without typing `cd D:\` in the terminal.

**Architecture:** A `list_drives` Tauri command returns drive roots (`["C:/", "D:/"]`) from a single `GetLogicalDrives()` call on Windows and an empty vector everywhere else. The bit-to-letter conversion lives in a pure function so it is testable off-Windows. The frontend renders a toolbar dropdown only when two or more drives exist; selecting one calls `loadDir()` on exactly the path the breadcrumb would — deliberately never touching `ptyCwdRef`, which is what keeps the panel following the terminal.

**Tech Stack:** Rust (Tauri 2, `windows-sys` behind a `cfg(windows)` target block), React 19 + TypeScript, Vitest + React Testing Library.

**Spec:** `docs/superpowers/specs/2026-07-28-windows-drive-switcher-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `src-tauri/Cargo.toml` | `[target.'cfg(windows)'.dependencies]` for `windows-sys` |
| `src-tauri/src/pty/commands.rs` | `drives_from_mask` (pure) + `list_drives` (command) |
| `src-tauri/src/lib.rs` | Register `list_drives` |
| `src/ipc/fs.ts` | `listDrives()` wrapper |
| `src/lib/i18n.ts` | Dropdown title strings, both locales |
| `src/components/FileExplorer/FileExplorer.tsx` | Toolbar button + dropdown + selection handler |
| `src/components/FileExplorer/FileExplorer.css` | Dropdown styling |
| `src/components/FileExplorer/FileExplorer.test.tsx` | Mock refactor (Task 1) + new tests (Task 5) |

---

## Task 1: Make the existing tests survive a new mount-time call

**Do this first, before any feature code.** All six existing tests chain `invokeMock.mockResolvedValueOnce(...)` in the exact order the component happens to call things, with comments tracking the sequence:

```tsx
    invokeMock
      .mockResolvedValueOnce(null) // getSessionCwd (ptyCwdRef seed)
      .mockResolvedValueOnce([])   // listDirectory → empty
      .mockResolvedValueOnce(null) // getSessionCwd (inside loadDir)
      .mockResolvedValue(null);    // polling
```

Task 5 adds a `list_drives` call on mount. That shifts every queued value by one and breaks **all six at once**, in ways that look like feature bugs. Converting the mock to dispatch on command name (and arguments) first means the feature lands on a suite that cannot be broken this way again.

This task changes **no production code**. The six tests must still pass, testing exactly what they tested before.

**Files:**
- Modify: `src/components/FileExplorer/FileExplorer.test.tsx`

- [ ] **Step 1: Add the dispatcher helper**

Insert after the `import { FileExplorer } from "./FileExplorer";` line and before `beforeEach`:

```tsx
/** Results returned for commands a test does not explicitly handle. */
const DEFAULT_RESULTS: Record<string, unknown> = {
  pty_get_cwd: null,
  pty_list_dir: [],
};

/**
 * Dispatch invoke() by command name and arguments instead of by call order.
 *
 * These tests used to chain mockResolvedValueOnce in the exact sequence the
 * component happens to call things. That made every test hostage to that
 * sequence: adding one new call on mount shifts every queued value by one and
 * breaks the whole file at once, with failures that look like feature bugs.
 * Keying on the command name means a new call only affects the tests that
 * actually care about it.
 */
function mockCommands(
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  invokeMock.mockImplementation((cmd: string, args: Record<string, unknown> = {}) => {
    const handler = handlers[cmd];
    if (handler) return Promise.resolve(handler(args));
    return Promise.resolve(cmd in DEFAULT_RESULTS ? DEFAULT_RESULTS[cmd] : null);
  });
}
```

Also delete the now-stale sequence comment above the first `describe`:

```tsx
// Component call order on mount:
//   1. getSessionCwd (seed ptyCwdRef useEffect)
//   2. listDirectory (loadDir useEffect)
//   3. getSessionCwd (inside loadDir when path === "")
// Then polling getSessionCwd every 1500ms (ignored in tests)
```

- [ ] **Step 2: Replace each test's mock setup**

In each of the six tests, replace **only** the `invokeMock.mockResolvedValueOnce(...)` chain with the call below. **Leave every assertion, render call and user interaction exactly as it is** — this task must not change what any test asserts.

`"shows empty viewer state initially"`:
```tsx
    mockCommands();
```

`"clicking a file loads its content in the viewer"`:
```tsx
    mockCommands({
      pty_get_cwd: () => "/p",
      pty_list_dir: () => [{ name: "index.ts", path: "/p/index.ts", is_dir: false, size: 20 }],
      pty_read_file: () => ({ content: "export default 1;", truncated: false }),
    });
```

`"clicking a directory does NOT load file content"`:
```tsx
    mockCommands({
      pty_get_cwd: () => "/p",
      pty_list_dir: ({ path }) =>
        path === "/p/src" ? [] : [{ name: "src", path: "/p/src", is_dir: true, size: null }],
    });
```

`"does not render the button when onSwitchTerminalHere is not provided"`:
```tsx
    mockCommands({ pty_get_cwd: () => "/p" });
```

`"calls onSwitchTerminalHere with the currently-browsed directory when clicked"`:
```tsx
    mockCommands({ pty_get_cwd: () => "/Users/jamesju/Downloads" });
```

`"follows the last-clicked tree folder without navigating away from the root listing"`:
```tsx
    const ROOT = "/Users/jamesju/Downloads";
    const CHILD = `${ROOT}/08_軟體安裝檔`;
    mockCommands({
      pty_get_cwd: () => ROOT,
      pty_list_dir: ({ path }) =>
        path === CHILD
          ? [{ name: "desktop.ini", path: `${CHILD}/desktop.ini`, is_dir: false, size: 282 }]
          : [{ name: "08_軟體安裝檔", path: CHILD, is_dir: true, size: null }],
    });
```

Note the last two use the same literal paths the old chains used, so their assertions keep matching.

- [ ] **Step 3: Run the tests**

```bash
npx vitest run src/components/FileExplorer/FileExplorer.test.tsx
```

Expected: `Tests  6 passed`. If any fails, the conversion changed behaviour — fix the mock, do not change the assertion.

- [ ] **Step 4: Prove the conversion actually removed the order-coupling**

This is the whole point of the task, so verify it rather than assume. Temporarily add a throwaway call at the very top of `FileExplorer`'s body:

```tsx
  useEffect(() => { void invoke("some_unrelated_probe"); }, []);
```

(importing `invoke` from `@tauri-apps/api/core`). Run the suite: all six must **still pass**. Then remove the probe and the import, and confirm `git diff src/components/FileExplorer/FileExplorer.tsx` is empty.

Under the old order-based chains this probe would have broken every test.

- [ ] **Step 5: Commit**

```bash
git add src/components/FileExplorer/FileExplorer.test.tsx
git commit -m "test(file-explorer): dispatch invoke mocks by command, not call order"
```

---

## Task 2: `drives_from_mask` and `list_drives`

`GetLogicalDrives()` returns a bitmask — bit 0 is `A:`, bit 1 is `B:`, and so on. The conversion from that mask to `["C:/", "D:/"]` is where off-by-one and letter-mapping mistakes live, and it cannot be exercised on macOS if it is welded to the Win32 call. So it goes in a pure function, exactly like `commands/updater.rs::supported_for`.

**Files:**
- Modify: `src-tauri/src/pty/commands.rs`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/pty/commands.rs`:

```rust
/// Expands a `GetLogicalDrives` bitmask into drive roots.
///
/// Bit 0 is `A:`, bit 1 is `B:`, through bit 25 for `Z:`; higher bits are
/// undefined and ignored. Roots use a forward slash to match `norm()`'s output
/// so the frontend never has to normalise separators.
///
/// Kept free of the Win32 call so it is testable on any platform — the bit
/// arithmetic is the part that can actually be wrong.
fn drives_from_mask(mask: u32) -> Vec<String> {
    let _ = mask;
    Vec::new()
}

#[cfg(test)]
mod drive_tests {
    use super::drives_from_mask;

    #[test]
    fn empty_mask_yields_no_drives() {
        assert!(drives_from_mask(0).is_empty());
    }

    #[test]
    fn bit_zero_is_drive_a() {
        assert_eq!(drives_from_mask(0b1), vec!["A:/"]);
    }

    #[test]
    fn typical_windows_machine_has_c_and_d() {
        // bits 2 and 3
        assert_eq!(drives_from_mask(0b1100), vec!["C:/", "D:/"]);
    }

    #[test]
    fn bit_twenty_five_is_drive_z() {
        assert_eq!(drives_from_mask(1 << 25), vec!["Z:/"]);
    }

    #[test]
    fn all_twenty_six_letters_are_expanded_in_order() {
        let all = drives_from_mask((1 << 26) - 1);
        assert_eq!(all.len(), 26);
        assert_eq!(all[0], "A:/");
        assert_eq!(all[25], "Z:/");
    }

    #[test]
    fn bits_above_twenty_five_are_ignored() {
        // Nothing beyond Z: exists, so the high bits must not invent entries.
        assert!(drives_from_mask(1 << 26).is_empty());
        assert_eq!(drives_from_mask((1 << 31) | 0b100), vec!["C:/"]);
    }
}
```

- [ ] **Step 2: Run the tests, confirm five of six fail**

```bash
cd src-tauri && cargo test drive_tests
```

Expected: `empty_mask_yields_no_drives` and `bits_above_twenty_five_are_ignored`'s first assertion pass by accident; the rest FAIL with `assertion `left == right` failed`.

- [ ] **Step 3: Implement**

Replace the stub body:

```rust
fn drives_from_mask(mask: u32) -> Vec<String> {
    (0..26u32)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| format!("{}:/", (b'A' + bit as u8) as char))
        .collect()
}
```

and add the command below it:

```rust
/// Drive roots available on this machine, for the file panel's drive switcher.
///
/// Windows only — every other platform has a single rooted filesystem, so an
/// empty list is the honest answer and the frontend hides the control.
#[tauri::command]
pub fn list_drives() -> Vec<String> {
    #[cfg(windows)]
    {
        // A single kernel32 call reading a bitmask: no per-drive filesystem
        // access, so a mapped-but-disconnected network drive cannot stall it
        // the way probing each letter with fs::metadata would.
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        drives_from_mask(mask)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
```

- [ ] **Step 4: Run the tests, confirm all pass**

```bash
cd src-tauri && cargo test drive_tests
```

Expected: `test result: ok. 6 passed`.

Note: on macOS the `#[cfg(windows)]` branch is not compiled, so `drives_from_mask` will be reported as dead code by `cargo build` until Task 3 registers the command. That warning is expected here.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pty/commands.rs
git commit -m "feat(file-explorer): add list_drives command for the drive switcher"
```

---

## Task 3: Wire the command through to TypeScript

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/ipc/fs.ts`

- [ ] **Step 1: Add the Windows-only dependency**

Append to the end of `src-tauri/Cargo.toml`:

```toml
# Windows-only: GetLogicalDrives for the file panel's drive switcher.
# Already present in Cargo.lock as a transitive dependency, so this adds no
# meaningful build cost, and nothing is compiled on macOS or Linux.
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.60", features = ["Win32_Storage_FileSystem"] }
```

Verified: `GetLogicalDrives` is declared in `windows-sys-0.60.2/src/Windows/Win32/Storage/FileSystem/mod.rs` as `fn GetLogicalDrives() -> u32`, and `Win32_Storage_FileSystem` is a real feature in that crate's `Cargo.toml`.

- [ ] **Step 2: Register the command**

In `src-tauri/src/lib.rs`, extend the pty import (currently lines 85-86):

```rust
    pty_close, pty_create, pty_get_cwd, pty_get_recent_output, pty_get_shell_type,
    pty_list_dir, pty_read_file, pty_resize, pty_write, read_file_as_bytes, write_text_file,
    list_drives,
```

and add it to `generate_handler!`, directly after the `pty_list_dir,` line:

```rust
            pty_list_dir,
            list_drives,
```

- [ ] **Step 3: Add the IPC wrapper**

In `src/ipc/fs.ts`, add after `listDirectory`:

```ts
/** Drive roots (e.g. ["C:/", "D:/"]). Empty on non-Windows platforms. */
export const listDrives = (): Promise<string[]> =>
  invoke<string[]>("list_drives");
```

- [ ] **Step 4: Verify**

```bash
cd src-tauri && cargo build 2>&1 | tail -3
```

Expected: `Finished`, and the `drives_from_mask` dead-code warning from Task 2 is gone on Windows. On macOS it will still warn, because the only caller is inside `#[cfg(windows)]` — that is correct and expected.

```bash
npx tsc --noEmit
```

Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src/ipc/fs.ts
git commit -m "feat(file-explorer): register list_drives and add its IPC wrapper"
```

---

## Task 4: i18n strings

`Translations` is derived from the zh-TW object, so a key added to one locale but not the other is a type error. That check is the test.

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add the strings**

Find `file_go_up` (it exists in both locale objects — that is the `↑` button's title). Insert directly after it in the **zh-TW** object:

```ts
    file_switch_drive: "切換磁碟機",
```

and directly after it in the **en** object:

```ts
    file_switch_drive: "Switch drive",
```

- [ ] **Step 2: Verify both locales stayed in sync**

```bash
npx tsc --noEmit
```

Expected: no output. A key in only one object surfaces here as a `Translations` mismatch.

Confirm it really landed twice:

```bash
grep -c "file_switch_drive" src/lib/i18n.ts
```

Expected: `2`.

- [ ] **Step 3: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(file-explorer): add drive switcher strings for zh-TW and en"
```

---

## Task 5: The dropdown

**Files:**
- Modify: `src/components/FileExplorer/FileExplorer.tsx`
- Modify: `src/components/FileExplorer/FileExplorer.css`
- Modify: `src/components/FileExplorer/FileExplorer.test.tsx`

- [ ] **Step 1: Write the failing tests**

Add `list_drives: []` to `DEFAULT_RESULTS` in `FileExplorer.test.tsx` so tests that do not care about drives keep seeing no dropdown:

```ts
const DEFAULT_RESULTS: Record<string, unknown> = {
  pty_get_cwd: null,
  pty_list_dir: [],
  list_drives: [],
};
```

Then append this describe block:

```tsx
describe("FileExplorer — drive switcher", () => {
  it("does not render the control when there are no drives (non-Windows)", async () => {
    mockCommands({ pty_get_cwd: () => "/p" });
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByTitle("切換磁碟機")).not.toBeInTheDocument();
  });

  it("does not render the control when there is only one drive", async () => {
    // Nothing to switch to, and the breadcrumb already shows C:.
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => ["C:/"],
    });
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());

    expect(screen.queryByTitle("切換磁碟機")).not.toBeInTheDocument();
  });

  it("shows the drive taken from the current path", async () => {
    mockCommands({
      pty_get_cwd: () => "D:/work",
      list_drives: () => ["C:/", "D:/"],
    });
    render(<FileExplorer sessionId="s1" />);

    const button = await screen.findByTitle("切換磁碟機");
    expect(button).toHaveTextContent("D:");
  });

  it("navigates to the drive root when one is picked", async () => {
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => ["C:/", "D:/"],
    });
    render(<FileExplorer sessionId="s1" />);

    await userEvent.click(await screen.findByTitle("切換磁碟機"));
    await userEvent.click(await screen.findByRole("button", { name: "D:" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("pty_list_dir", { id: "s1", path: "D:/" })
    );
  });

  it("re-reads the drive list each time the menu opens", async () => {
    // A USB stick plugged in after launch has to show up.
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => ["C:/", "D:/"],
    });
    render(<FileExplorer sessionId="s1" />);

    const button = await screen.findByTitle("切換磁碟機");
    const afterMount = invokeMock.mock.calls.filter((c) => c[0] === "list_drives").length;

    await userEvent.click(button);

    await waitFor(() =>
      expect(invokeMock.mock.calls.filter((c) => c[0] === "list_drives").length)
        .toBeGreaterThan(afterMount)
    );
  });

  it("keeps the previous list when re-reading fails", async () => {
    // The user has the menu open; blanking it is worse than showing a list that
    // may be a moment stale.
    let firstCall = true;
    mockCommands({
      pty_get_cwd: () => "C:/Users",
      list_drives: () => {
        if (firstCall) { firstCall = false; return ["C:/", "D:/"]; }
        throw new Error("drive enumeration failed");
      },
    });
    render(<FileExplorer sessionId="s1" />);

    await userEvent.click(await screen.findByTitle("切換磁碟機"));

    expect(await screen.findByRole("button", { name: "D:" })).toBeInTheDocument();
  });

  it("keeps following the terminal after a drive is picked", async () => {
    // The invariant this whole design is shaped around: picking a drive is user
    // navigation, so it must not touch ptyCwdRef. If it did, the poll would
    // believe the terminal had already moved and would stop following it.
    vi.useFakeTimers();
    try {
      let terminalCwd = "C:/Users";
      mockCommands({
        pty_get_cwd: () => terminalCwd,
        list_drives: () => ["C:/", "D:/"],
      });
      render(<FileExplorer sessionId="s1" />);

      // user-event v14 takes advanceTimers on setup(), not per call — passing it
      // to click() is silently ignored and the click then hangs on fake timers.
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

      await vi.waitFor(() => screen.getByTitle("切換磁碟機"));
      await user.click(screen.getByTitle("切換磁碟機"));
      await user.click(screen.getByRole("button", { name: "D:" }));

      // Now the terminal itself moves somewhere new.
      terminalCwd = "C:/Windows";
      await vi.advanceTimersByTimeAsync(1600);

      await vi.waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("pty_list_dir", { id: "s1", path: "C:/Windows" })
      );
    } finally {
      vi.useRealTimers();
    }
  });
});
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
npx vitest run src/components/FileExplorer/FileExplorer.test.tsx
```

Expected: the seven new tests FAIL (`Unable to find an element with the title: 切換磁碟機`, and the two "does not render" ones passing vacuously for now). The six from Task 1 must still pass.

- [ ] **Step 3: Implement the component changes**

In `src/components/FileExplorer/FileExplorer.tsx`, extend the import on line 2:

```tsx
import { listDirectory, getSessionCwd, listDrives, type DirEntry } from "../../ipc/fs";
```

Add state beside the other `useState` declarations:

```tsx
  const [drives, setDrives] = useState<string[]>([]);
  const [driveMenuOpen, setDriveMenuOpen] = useState(false);
```

Add the mount fetch, after the `ptyCwdRef` seeding effect:

```tsx
  // Drive list for the switcher. Empty on non-Windows, which hides the control.
  useEffect(() => {
    listDrives().then(setDrives).catch(() => {});
  }, []);
```

Add the handlers, next to `goUp`:

```tsx
  const openDriveMenu = () => {
    setDriveMenuOpen(true);
    // Re-read on open so a drive mounted after launch shows up. On failure keep
    // the list we already have — blanking a menu the user just opened is worse
    // than showing one that may be a moment stale.
    listDrives().then((d) => { if (d.length) setDrives(d); }).catch(() => {});
  };

  const selectDrive = (drive: string) => {
    setDriveMenuOpen(false);
    // Deliberately identical to a breadcrumb click, and deliberately NOT
    // touching ptyCwdRef: that ref is the poll's record of where the *terminal*
    // is. Updating it here would make the poll think the terminal had already
    // moved, and the panel would stop following it for the rest of the session.
    loadDir(drive);
    setExpanded(new Set());
    setSubEntries({});
  };

  // "C:/Users/foo" → "C:"; null when the path carries no drive letter.
  const currentDrive = /^([A-Za-z]:)/.exec(cwd.replace(/\\/g, "/"))?.[1] ?? null;
```

Render the control in the toolbar, directly after the refresh (`↻`) button and before `<div className="fe-breadcrumb">`:

```tsx
        {drives.length > 1 && (
          <div className="fe-drive">
            <button
              className="fe-btn aiterm-btn aiterm-btn--secondary"
              title={t.file_switch_drive}
              onClick={() => (driveMenuOpen ? setDriveMenuOpen(false) : openDriveMenu())}
            >
              {currentDrive ?? drives[0].replace("/", "")} ▾
            </button>
            {driveMenuOpen && (
              <>
                <div className="fe-drive-backdrop" onClick={() => setDriveMenuOpen(false)} />
                <div className="fe-drive-menu">
                  {drives.map((d) => (
                    <button
                      key={d}
                      className="fe-drive-item"
                      onClick={() => selectDrive(d)}
                    >
                      {d.replace("/", "")}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
        )}
```

- [ ] **Step 4: Add the styles**

Append to `src/components/FileExplorer/FileExplorer.css`:

```css
.fe-drive {
  position: relative;
  flex-shrink: 0;
}

/* Catches clicks anywhere else so the menu closes without a document listener. */
.fe-drive-backdrop {
  position: fixed;
  inset: 0;
  z-index: 10;
}

.fe-drive-menu {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: 11;
  min-width: 72px;
  max-height: 240px;
  overflow-y: auto;
  background: #1b1b1b;
  border: 1px solid #333;
  border-radius: 6px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.5);
  padding: 4px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.fe-drive-item {
  background: none;
  border: none;
  color: #e6e6e6;
  font-family: inherit;
  font-size: 13px;
  text-align: left;
  padding: 5px 10px;
  border-radius: 4px;
  cursor: pointer;
}

.fe-drive-item:hover {
  background: #2a2a2a;
}
```

- [ ] **Step 5: Run the tests, confirm they pass**

```bash
npx vitest run src/components/FileExplorer/FileExplorer.test.tsx
```

Expected: `Tests  13 passed`.

- [ ] **Step 6: Prove the invariant test actually guards the invariant**

Green tests are not evidence — verify by mutation. One at a time, edit `FileExplorer.tsx`, run the suite, confirm a FAILURE, then `git checkout -- src/components/FileExplorer/FileExplorer.tsx`:

| # | Mutation | Should be caught by |
|---|---|---|
| M1 | add `ptyCwdRef.current = drive;` inside `selectDrive` | "keeps following the terminal after a drive is picked" |
| M2 | `drives.length > 1` → `drives.length > 0` | "does not render the control when there is only one drive" |
| M3 | `selectDrive` calls `loadDir(cwd)` instead of `loadDir(drive)` | "navigates to the drive root when one is picked" |
| M4 | `openDriveMenu` only sets state, no re-fetch | "re-reads the drive list each time the menu opens" |
| M5 | `currentDrive` → always `drives[0]` | "shows the drive taken from the current path" |
| M6 | `openDriveMenu`'s `.catch(() => {})` → `.catch(() => setDrives([]))` | "keeps the previous list when re-reading fails" |

**M1 is the one that matters.** It is the regression this design exists to prevent, and before this task nothing in the suite would have noticed it. If any mutation survives, strengthen the test rather than lowering the bar.

Restore the file afterwards and confirm `git status --short src/` shows only the intended changes.

- [ ] **Step 7: Verify the whole suite and types**

```bash
npx tsc --noEmit && npm run test
npx eslint src/components/FileExplorer/FileExplorer.tsx src/components/FileExplorer/FileExplorer.test.tsx src/ipc/fs.ts src/lib/i18n.ts
```

Expected: no type errors, full suite green, eslint exits 0 on the changed files. **Do not run bare `npm run lint` as a gate** — this repo carries ~181 pre-existing problems.

- [ ] **Step 8: Commit**

```bash
git add src/components/FileExplorer/FileExplorer.tsx \
        src/components/FileExplorer/FileExplorer.css \
        src/components/FileExplorer/FileExplorer.test.tsx
git commit -m "feat(file-explorer): add a Windows drive switcher to the toolbar"
```

---

## Task 6: Windows verification

`GetLogicalDrives()` cannot run on macOS, so everything above tests the pure function and the React wiring — not the actual enumeration. This task is the only thing that proves the feature works.

**Files:** none (verification only)

- [ ] **Step 1: Confirm the control is absent on macOS**

```bash
npm run tauri:dev
```

Open the file panel. The drive button must **not** appear. Stop the dev server.

This is a real check, not a formality: it proves the non-Windows branch returns an empty list rather than erroring.

- [ ] **Step 2: Ask before tagging**

A Windows build requires a release. **Do not push a tag without explicit user confirmation.** Ask and wait.

- [ ] **Step 3: Verify on Windows**

On a Windows machine with at least two drives, open the file panel and confirm:

1. The drive button appears and shows the current drive (e.g. `C: ▾`).
2. Opening it lists exactly the drives Explorer shows.
3. Picking `D:` lists `D:\`'s contents, and the breadcrumb updates.
4. **After switching to `D:`, typing `cd C:\Windows` in the terminal makes the panel follow.** This is the invariant — if the panel stays on `D:`, `ptyCwdRef` was touched somewhere.
5. Plugging in a USB stick and reopening the menu shows the new drive.
6. A machine with a single drive shows no button.

- [ ] **Step 4: Record the result**

Append a "驗證結果" section to `docs/superpowers/specs/2026-07-28-windows-drive-switcher-design.md` covering which of the six checks passed, on what Windows version, and anything unexpected. Note explicitly anything left unverified.

```bash
git add -f docs/superpowers/specs/2026-07-28-windows-drive-switcher-design.md
git commit -m "docs: record Windows drive switcher verification results"
```

---

## Notes for the implementer

- **Green tests are not evidence.** Task 5 Step 6 exists because this repo has already shipped a suite that passed 10/10 while 5 of 7 mutations survived. Mutate and confirm the failure.
- **`docs/` is gitignored** (`.gitignore:47`) but specs and plans are tracked. Use `git add -f` for anything under `docs/`.
- **Never push a tag without asking.** Tags trigger release builds.
- The one thing that must not regress is CWD sync. If a change makes the file panel stop following the terminal, that is a user-visible breakage of existing behaviour, not a shortcoming of the new feature.
